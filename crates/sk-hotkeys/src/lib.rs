//! System-wide hotkeys: a low-level hook (Windows) or event tap (macOS) feeding a chord engine.
//! Hooks do no VM work and run no user callbacks; a separate worker calls the handler.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};
mod engine;
#[cfg(windows)]
mod native;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod native;

/// Set in a keyboard [`Button::code`] that holds a scan code rather than a virtual-key code.
pub const RAW: u32 = 0x10000;
/// A DOM physical key, encoded as a Windows set-1 scan position.
pub const POSITION: u32 = 0x20000;
/// Either side of a modifier (Shift, Control, Alt or Meta).
pub const EITHER: u32 = 0x30000;
/// Device 0 is the keyboard (Windows virtual-key codes, or [`RAW`] scan codes), device 1 the mouse
/// (1 left, 2 right, 3 middle, 4 and 5 the side buttons).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Button {
    pub device: u8,
    pub code: u32,
}
impl Button {
    pub fn keyboard(code: u32) -> Self {
        Self { device: 0, code }
    }
    fn normalized(mut self) -> Self {
        if self.device == 0 {
            self.code = match self.code {
                0x10 => 0xa0,
                0x11 => 0xa2,
                0x12 => 0xa4,
                other => other,
            };
        }
        self
    }
    fn modifier(&self) -> bool {
        self.device == 0 && matches!(self.code, 0xa0..=0xa5 | 0x5b | 0x5c)
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Chord {
    pub buttons: Vec<Button>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub id: String,
    pub chord: Chord,
    /// Consume the key events that complete the chord, so the focused app does not see them.
    pub suppress: Option<bool>,
    /// Minimum time between two down edges of this binding.
    pub trigger_cooldown_ms: Option<u64>,
}
/// `phase` is "down" or "up". `generation` is the service's [`Hotkeys::current_generation`] when
/// the event was produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fired {
    pub id: String,
    pub phase: String,
    pub generation: u64,
    /// The service ended this hold without observing its physical release.
    pub cancelled: bool,
}
impl Fired {
    fn down(id: &str) -> Self {
        Self {
            id: id.into(),
            phase: "down".into(),
            generation: 0,
            cancelled: false,
        }
    }
    fn up(id: &str) -> Self {
        Self {
            id: id.into(),
            phase: "up".into(),
            generation: 0,
            cancelled: false,
        }
    }
    fn cancel(id: &str) -> Self {
        Self {
            cancelled: true,
            ..Self::up(id)
        }
    }
}
#[derive(Debug, Clone)]
pub struct Status {
    pub running: bool,
    pub platform: String,
    pub accessibility_granted: bool,
    pub error: Option<String>,
    pub restarts: u64,
}
type Handler = Arc<dyn Fn(Fired) + Send + Sync>;
type CaptureCallback = Box<dyn FnOnce(Result<Chord, String>) + Send>;
struct CaptureJob {
    callback: CaptureCallback,
    result: Result<Chord, String>,
}
#[derive(Default)]
struct Queue {
    events: VecDeque<Fired>,
    urgent_up: BTreeMap<String, Fired>,
    delivered_down: BTreeSet<String>,
    captures: VecDeque<CaptureJob>,
    closed: bool,
}
struct Dispatch {
    queue: Mutex<Queue>,
    wake: Condvar,
    captures_reserved: AtomicUsize,
    handler: Arc<Mutex<Option<Handler>>>,
    generation: AtomicU64,
}
impl Dispatch {
    fn new(handler: Arc<Mutex<Option<Handler>>>, generation: u64) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            captures_reserved: AtomicUsize::new(0),
            handler,
            generation: AtomicU64::new(generation),
        })
    }
    fn invalidate(&self, generation: u64) {
        let mut queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        queue.events.clear();
        queue.urgent_up.clear();
        let held: Vec<_> = queue.delivered_down.iter().cloned().collect();
        for id in held {
            let mut event = Fired::cancel(&id);
            event.generation = generation;
            queue.urgent_up.insert(id, event);
        }
        self.generation.store(generation, Ordering::Release);
        self.wake.notify_one();
    }
    fn events(&self, events: Vec<Fired>) {
        let mut queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        for event in events {
            if event.generation != self.generation.load(Ordering::Acquire) {
                continue;
            }
            if event.phase == "up" && queue.urgent_up.contains_key(&event.id) {
                queue.events.retain(|old| old.id != event.id);
                continue;
            }
            if queue.events.len() < 256 {
                queue.events.push_back(event);
            } else {
                // Overloaded listeners may miss a press, but must never retain
                // a hold. Remove queued presses for this ID before its urgent up.
                queue.events.retain(|old| old.id != event.id);
                if queue.delivered_down.contains(&event.id) {
                    queue.urgent_up.insert(
                        event.id.clone(),
                        Fired {
                            phase: "up".into(),
                            cancelled: true,
                            ..event
                        },
                    );
                }
            }
        }
        self.wake.notify_one();
    }
    fn reserve_capture(&self) -> bool {
        self.captures_reserved
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 16).then_some(n + 1)
            })
            .is_ok()
    }
    fn capture(&self, callback: CaptureCallback, result: Result<Chord, String>) {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .captures
            .push_back(CaptureJob { callback, result });
        self.wake.notify_one();
    }
    fn close(&self) {
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
        self.wake.notify_all();
    }
    fn run(self: Arc<Self>) {
        enum Job {
            Event(Fired),
            Capture(CaptureJob),
        }
        loop {
            let job = {
                let mut queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
                loop {
                    if let Some(job) = queue.captures.pop_front() {
                        break Some(Job::Capture(job));
                    }
                    if let Some((_, event)) = queue.urgent_up.pop_first() {
                        if queue.delivered_down.remove(&event.id) {
                            break Some(Job::Event(event));
                        }
                        continue;
                    }
                    if let Some(event) = queue.events.pop_front() {
                        if event.phase == "down" {
                            // Bound the number of delivered holds even while
                            // producers continually replace binding IDs.
                            if queue.delivered_down.len() >= 128
                                || !queue.delivered_down.insert(event.id.clone())
                            {
                                continue;
                            }
                        } else if !queue.delivered_down.remove(&event.id) {
                            continue;
                        }
                        break Some(Job::Event(event));
                    }
                    if queue.closed {
                        break None;
                    }
                    queue = self.wake.wait(queue).unwrap_or_else(|p| p.into_inner());
                }
            };
            match job {
                None => break,
                Some(Job::Capture(job)) => {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        (job.callback)(job.result)
                    }));
                    self.captures_reserved.fetch_sub(1, Ordering::AcqRel);
                }
                Some(Job::Event(event)) => {
                    if event.phase == "down"
                        && event.generation != self.generation.load(Ordering::Acquire)
                    {
                        continue;
                    }
                    let handler = self
                        .handler
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    if let Some(handler) = handler {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            handler(event)
                        }));
                    }
                }
            }
        }
    }
}
#[derive(Default)]
struct State {
    engine: engine::Engine,
    capture: Option<(CaptureCallback, Arc<Dispatch>)>,
}
struct Workers {
    input: JoinHandle<()>,
    notify: JoinHandle<()>,
    dispatch: Arc<Dispatch>,
}

pub struct Hotkeys {
    state: Mutex<State>,
    handler: Arc<Mutex<Option<Handler>>>,
    dispatch: Mutex<Option<Arc<Dispatch>>>,
    workers: Mutex<Option<Workers>>,
    running: AtomicBool,
    stopping: AtomicBool,
    thread_id: AtomicU32,
    restarts: AtomicU64,
    generation: AtomicU64,
    error: Mutex<Option<String>>,
}
impl Hotkeys {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            handler: Arc::new(Mutex::new(None)),
            dispatch: Mutex::new(None),
            workers: Mutex::new(None),
            running: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            thread_id: AtomicU32::new(0),
            restarts: AtomicU64::new(0),
            generation: AtomicU64::new(1),
            error: Mutex::new(None),
        })
    }
    pub fn set_handler(&self, handler: Arc<dyn Fn(Fired) + Send + Sync>) {
        *self.handler.lock().unwrap_or_else(|p| p.into_inner()) = Some(handler);
    }
    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err("hotkeys are stopping".into());
        }
        let mut workers = self.workers.lock().unwrap_or_else(|p| p.into_inner());
        if self.running.load(Ordering::Acquire) {
            return Ok(());
        }
        if workers.is_some() {
            return Err("hotkey hook stopped unexpectedly; call restart".into());
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            self.fail("global hotkeys are unsupported on this platform".into());
            return Err("global hotkeys are unsupported on this platform".into());
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            let dispatch = Dispatch::new(Arc::clone(&self.handler), self.current_generation());
            let notifier = Arc::clone(&dispatch);
            let notify = std::thread::Builder::new()
                .name("sk-hotkeys-events".into())
                .spawn(move || notifier.run())
                .map_err(|e| e.to_string())?;
            *self.dispatch.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::clone(&dispatch));
            // The Mac worker publishes running before ready and owns every
            // subsequent failure. Clear a previous attempt before it can run.
            #[cfg(target_os = "macos")]
            {
                *self.error.lock().unwrap_or_else(|p| p.into_inner()) = None;
            }
            let owner = Arc::downgrade(self);
            let (ready, receive) = std::sync::mpsc::sync_channel(1);
            let input = match std::thread::Builder::new()
                .name("sk-hotkeys".into())
                .spawn(move || native::run(owner, ready))
            {
                Ok(thread) => thread,
                Err(error) => {
                    dispatch.close();
                    join(notify);
                    self.dispatch
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .take();
                    return Err(error.to_string());
                }
            };
            let result = receive
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap_or_else(|_| Err("timed out starting global hotkey hooks".into()));
            // A late Mac startup must see its readiness receiver disappear and
            // release the event tap instead of entering a loop nobody owns.
            drop(receive);
            if let Err(error) = result {
                native::wake_stop(self.thread_id.load(Ordering::Acquire));
                join(input);
                dispatch.close();
                join(notify);
                self.dispatch
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                self.fail(error.clone());
                return Err(error);
            }
            self.accept_startup_ready();
            *workers = Some(Workers {
                input,
                notify,
                dispatch,
            });
            Ok(())
        }
    }
    fn accept_startup_ready(&self) {
        // The Windows worker relies on its parent's ready handshake. The Mac
        // worker publishes before sending ready; it may already have failed or
        // lost permission by the time this thread wakes, so never overwrite it.
        #[cfg(windows)]
        {
            self.running.store(true, Ordering::Release);
            *self.error.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }
    pub fn set_bindings(&self, bindings: Vec<Binding>) -> Result<bool, String> {
        let bindings = normalize_bindings(bindings)?;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let (changed, output) = state.engine.set_bindings(bindings);
        if changed {
            self.invalidate_notifications();
        }
        self.deliver(&mut state, output);
        #[cfg(any(windows, target_os = "macos"))]
        if changed {
            state.engine.seed(native::held_modifiers());
            native::wake_sync(self.thread_id.load(Ordering::Acquire));
        }
        Ok(changed)
    }
    pub fn would_change(&self, bindings: &[Binding]) -> Result<bool, String> {
        let normalized = normalize_bindings(bindings.to_vec())?;
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        Ok(state.engine.bindings() != normalized.as_slice())
    }
    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub fn is_current(&self, event: &Fired) -> bool {
        event.phase == "up" || event.generation == self.current_generation()
    }
    // Called only while the engine lock is held: event creation and generation
    // changes share one ordering even if the notification worker is stalled.
    fn invalidate_notifications(&self) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(dispatch) = self
            .dispatch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            dispatch.invalidate(generation);
        }
    }
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
    pub fn status(&self) -> Status {
        Status {
            running: self.running.load(Ordering::Acquire),
            platform: std::env::consts::OS.into(),
            accessibility_granted: self.accessibility_granted(),
            error: self.error.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            restarts: self.restarts.load(Ordering::Acquire),
        }
    }
    pub fn accessibility_granted(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            native::accessibility_granted()
        }
        #[cfg(not(target_os = "macos"))]
        {
            cfg!(windows)
        }
    }
    /// Shows the macOS Accessibility prompt. Call it only from an explicit user action.
    pub fn request_accessibility(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            native::request_accessibility()
        }
        #[cfg(not(target_os = "macos"))]
        {
            cfg!(windows)
        }
    }
    pub fn restart(self: &Arc<Self>) -> Result<bool, String> {
        self.stop();
        self.start()?;
        self.restarts.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    }
    pub fn capture_once(
        &self,
        callback: impl FnOnce(Result<Chord, String>) + Send + 'static,
    ) -> Result<(), String> {
        if !self.running.load(Ordering::Acquire) {
            return Err("hotkeys are not running".into());
        }
        let dispatch = self
            .dispatch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or("hotkeys are not running")?;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if !self.running.load(Ordering::Acquire) {
            return Err("hotkeys are not running".into());
        }
        if !dispatch.reserve_capture() {
            return Err("hotkey capture callback queue is busy".into());
        }
        #[cfg(any(windows, target_os = "macos"))]
        let held = native::held_modifiers();
        #[cfg(not(any(windows, target_os = "macos")))]
        let held = Vec::new();
        self.invalidate_notifications();
        let output = state.engine.begin_capture(Instant::now(), held);
        self.deliver(&mut state, output); // settles the previous capture before installing its successor
        state.capture = Some((Box::new(callback), dispatch));
        #[cfg(any(windows, target_os = "macos"))]
        native::wake_sync(self.thread_id.load(Ordering::Acquire));
        Ok(())
    }
    pub fn cancel_capture(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let output = state.engine.cancel_capture();
        self.deliver(&mut state, output);
    }
    pub fn stop(&self) {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return;
        }
        self.running.store(false, Ordering::Release);
        let workers = self
            .workers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        self.reset();
        if let Some(workers) = workers {
            #[cfg(any(windows, target_os = "macos"))]
            native::wake_stop(self.thread_id.load(Ordering::Acquire));
            // No lifecycle/state lock is held while a callback or input thread exits.
            join(workers.input);
            workers.dispatch.close();
            join(workers.notify);
        }
        self.dispatch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        self.stopping.store(false, Ordering::Release);
    }
    fn deliver(&self, state: &mut State, mut output: engine::Output) {
        let generation = self.current_generation();
        for event in &mut output.events {
            event.generation = generation;
        }
        if let Some(dispatch) = self
            .dispatch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            dispatch.events(output.events);
        }
        if let Some(result) = output.capture {
            if let Some((callback, dispatch)) = state.capture.take() {
                dispatch.capture(callback, result);
            }
        }
    }
    fn input(&self, button: Button, down: bool) -> bool {
        self.input_at(button, 0, down)
    }
    fn input_at(&self, button: Button, scan: u16, down: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let output = state.engine.input_at(button, scan, down, Instant::now());
        let suppress = output.suppress;
        self.deliver(&mut state, output);
        suppress
    }
    fn tick(&self, now: Instant) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let output = state.engine.tick(now);
        self.deliver(&mut state, output);
    }
    fn reset(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        self.invalidate_notifications();
        let output = state.engine.reset();
        self.deliver(&mut state, output);
    }
    fn fail(&self, error: String) {
        self.running.store(false, Ordering::Release);
        *self.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
    }
}
impl Drop for Hotkeys {
    fn drop(&mut self) {
        self.stop();
    }
}
fn join(thread: JoinHandle<()>) {
    if thread.thread().id() != std::thread::current().id() {
        let _ = thread.join();
    }
}
fn normalize_bindings(mut bindings: Vec<Binding>) -> Result<Vec<Binding>, String> {
    if bindings.len() > 128 {
        return Err("at most 128 hotkey bindings are allowed".into());
    }
    let mut ids = BTreeSet::new();
    for binding in &mut bindings {
        if binding.id.is_empty()
            || binding.id.len() > 128
            || binding.id.contains('\0')
            || !ids.insert(binding.id.clone())
        {
            return Err("hotkey IDs must be unique nonempty strings of at most 128 bytes".into());
        }
        if binding.chord.buttons.len() > 16 || binding.trigger_cooldown_ms.unwrap_or(0) > 60_000 {
            return Err("hotkey chord or cooldown exceeds its limit".into());
        }
        for button in &mut binding.chord.buttons {
            *button = button.normalized();
            let valid = match button.device {
                0 => {
                    (1..=255).contains(&button.code)
                        || (button.code >> 16 == POSITION >> 16 && button.code & 0xffff != 0)
                        || (button.code >> 16 == EITHER >> 16
                            && matches!(button.code & 0xffff, 0x10..=0x12 | 0x5b))
                        || (button.code & RAW != 0
                            && button.code <= 0x1ffff
                            && button.code & 0xffff != 0)
                }
                1 => (1..=5).contains(&button.code),
                _ => false,
            };
            if !valid {
                return Err("invalid hotkey device or Windows virtual-key/raw code".into());
            }
        }
        binding.chord.buttons.sort();
        binding.chord.buttons.dedup();
    }
    bindings.retain(|b| !b.chord.buttons.is_empty());
    bindings.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn held_binding(code: u32) -> Binding {
        Binding {
            id: "ptt".into(),
            chord: Chord {
                buttons: vec![Button::keyboard(code)],
            },
            suppress: None,
            trigger_cooldown_ms: None,
        }
    }
    fn queued_service() -> (Arc<Hotkeys>, Arc<Dispatch>) {
        let service = Hotkeys::new();
        let dispatch = Dispatch::new(Arc::clone(&service.handler), service.current_generation());
        *service.dispatch.lock().unwrap() = Some(dispatch.clone());
        (service, dispatch)
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn late_startup_ready_cannot_erase_a_mac_worker_failure() {
        let service = Hotkeys::new();
        // The worker sent ready, then the tap was disabled before the parent
        // received it. Model that ordering without scheduling or OS permission.
        service.running.store(true, Ordering::Release);
        service.fail("the tap was disabled after ready".into());
        service.accept_startup_ready();
        assert!(!service.running.load(Ordering::Acquire));
        assert_eq!(
            service.error.lock().unwrap().as_deref(),
            Some("the tap was disabled after ready")
        );
    }
    #[test]
    fn generic_modifiers_normalize_left_and_raw_keys_remain_distinct() {
        let bindings = normalize_bindings(vec![Binding {
            id: "x".into(),
            chord: Chord {
                buttons: vec![
                    Button::keyboard(0x11),
                    Button::keyboard(0xa2),
                    Button::keyboard(RAW | 0x0e1c),
                ],
            },
            suppress: None,
            trigger_cooldown_ms: None,
        }])
        .unwrap();
        assert_eq!(
            bindings[0].chord.buttons,
            [Button::keyboard(0xa2), Button::keyboard(RAW | 0x0e1c)]
        );
    }
    #[test]
    fn overflow_queue_cannot_deliver_a_stale_down_after_an_urgent_release() {
        let dispatch = Dispatch::new(Arc::new(Mutex::new(None)), 0);
        dispatch
            .queue
            .lock()
            .unwrap()
            .delivered_down
            .insert("x".into());
        dispatch.events((0..256).map(|_| Fired::down("x")).collect());
        dispatch.events(vec![Fired::up("x")]);
        let queue = dispatch.queue.lock().unwrap();
        assert!(queue.events.is_empty());
        assert_eq!(queue.urgent_up.get("x"), Some(&Fired::cancel("x")));
    }
    #[test]
    fn stalled_listener_reconfiguration_does_not_accumulate_undelivered_releases() {
        let dispatch = Dispatch::new(Arc::new(Mutex::new(None)), 0);
        dispatch.events(
            (0..256)
                .map(|i| Fired::down(&format!("queued-{i}")))
                .collect(),
        );
        for i in 0..10_000 {
            dispatch.events(vec![
                Fired::down(&format!("new-{i}")),
                Fired::up(&format!("new-{i}")),
            ]);
        }
        let queue = dispatch.queue.lock().unwrap();
        assert_eq!(queue.events.len(), 256);
        assert!(queue.urgent_up.is_empty());
    }
    #[test]
    fn changed_bindings_invalidate_queued_down_and_stamp_balanced_release() {
        let (service, dispatch) = queued_service();
        service.set_bindings(vec![held_binding(0x70)]).unwrap();
        service.input(Button::keyboard(0x70), true);
        let in_flight = {
            let mut queue = dispatch.queue.lock().unwrap();
            let event = queue.events.pop_front().unwrap();
            queue.delivered_down.insert(event.id.clone());
            event
        };
        assert!(service.is_current(&in_flight));
        assert!(service.set_bindings(vec![held_binding(0x71)]).unwrap());
        assert!(
            !service.is_current(&in_flight),
            "popped events keep their original generation"
        );
        let queue = dispatch.queue.lock().unwrap();
        assert!(queue.events.iter().all(|event| event.phase != "down"));
        let release = queue.urgent_up.get("ptt").unwrap();
        assert_eq!(release.generation, service.current_generation());
        assert!(service.is_current(release));
        assert_eq!((release.id.as_str(), release.phase.as_str()), ("ptt", "up"));
    }
    #[test]
    fn capture_and_reset_discard_backlogged_downs() {
        let (service, dispatch) = queued_service();
        service.set_bindings(vec![held_binding(0x70)]).unwrap();
        service.input(Button::keyboard(0x70), true);
        let old = dispatch
            .queue
            .lock()
            .unwrap()
            .events
            .front()
            .unwrap()
            .clone();
        service.running.store(true, Ordering::Release);
        service.capture_once(|_| {}).unwrap();
        assert!(!service.is_current(&old));
        assert!(
            dispatch
                .queue
                .lock()
                .unwrap()
                .events
                .iter()
                .all(|event| event.phase != "down")
        );
        service.cancel_capture();
        service.input(Button::keyboard(0x70), false);
        service.input(Button::keyboard(0x70), true);
        let old = dispatch
            .queue
            .lock()
            .unwrap()
            .events
            .back()
            .unwrap()
            .clone();
        assert_eq!(old.phase, "down");
        service.reset();
        assert!(!service.is_current(&old));
        assert!(
            dispatch
                .queue
                .lock()
                .unwrap()
                .events
                .iter()
                .all(|event| event.phase != "down")
        );
        let old_up = Fired {
            phase: "up".into(),
            ..old
        };
        assert!(
            service.is_current(&old_up),
            "an already posted release must remain deliverable"
        );
    }
    #[test]
    fn unchanged_binding_does_not_release_a_hold_or_advance_generation() {
        let (service, dispatch) = queued_service();
        let binding = held_binding(0x70);
        service.set_bindings(vec![binding.clone()]).unwrap();
        service.input(Button::keyboard(0x70), true);
        let before = service.current_generation();
        assert!(
            !service
                .would_change(std::slice::from_ref(&binding))
                .unwrap()
        );
        assert!(!service.set_bindings(vec![binding]).unwrap());
        assert_eq!(service.current_generation(), before);
        assert_eq!(dispatch.queue.lock().unwrap().events.len(), 1);
        assert_eq!(dispatch.queue.lock().unwrap().events[0].phase, "down");
        service.input(Button::keyboard(0x70), false);
        let queue = dispatch.queue.lock().unwrap();
        assert_eq!(
            queue
                .events
                .iter()
                .map(|event| event.phase.as_str())
                .collect::<Vec<_>>(),
            ["down", "up"]
        );
    }
}
