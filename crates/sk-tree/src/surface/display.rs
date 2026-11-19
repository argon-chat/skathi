//! The display each window is on, read from the frame tick. Neither macOS's EDR headroom nor the
//! Windows SDR brightness slider sends a notification, so it is read every tick, and while a
//! surface is visible and nothing else ticks, a thread wakes the pump every [`IDLE`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant};

use gpui::Window;

use super::SurfaceDisplay;
use crate::windows::WinKey;

const IDLE: Duration = Duration::from_millis(250);

/// How long a window stays in HDR after nothing it shows needs it, so content that comes and goes
/// does not switch the swap chain back and forth.
const HDR_LINGER: Duration = Duration::from_secs(2);

/// While a level keeps moving, as macOS's headroom does for a second or two after a window enters
/// or leaves EDR, it is reported at most this often.
const LEVEL_EVERY: Duration = Duration::from_millis(200);

/// macOS's refresh interval range can blink for a few milliseconds; a `variable_refresh` change is
/// taken once it has held this long.
const REFRESH_HOLD: Duration = Duration::from_millis(100);

#[derive(Default)]
struct Probe {
    /// What the platform read last, with the window's output.
    display: SurfaceDisplay,
    /// `display` as elements and producers hear it.
    reported: Reported,
    platform: platform::State,
    output: Output,
}

/// The window's HDR output: the SDR white it draws at while on, and the headroom it asks for.
#[derive(Default)]
struct Output {
    sdr_white: Option<f32>,
    headroom: f32,
    unwanted_since: Option<Instant>,
    /// The renderer refused HDR output; the window stays standard.
    refused: bool,
}

impl Probe {
    fn capable(&self) -> bool {
        self.display.known && self.display.hdr && !self.output.refused
    }

    fn take(&mut self, read: Option<SurfaceDisplay>, now: Instant) {
        if let Some(display) = read {
            self.display = display;
        }
        self.display.output_headroom = if self.capable() {
            self.display.headroom
        } else {
            1.0
        };
        self.display.output_hdr = self.output.sdr_white.is_some();
        self.reported.offer(self.display, now);
    }
}

impl Output {
    /// What to ask the window for, if anything: HDR while it can and `headroom` > 1.0 (or for
    /// [`HDR_LINGER`] after), tagged with that headroom, else standard output.
    fn request(
        &mut self,
        capable: bool,
        headroom: f32,
        sdr_white: f32,
        now: Instant,
    ) -> Option<Option<gpui::HdrOutput>> {
        let wanted = headroom > 1.0;
        if wanted {
            self.unwanted_since = None;
        }
        let lingering = !wanted
            && self.sdr_white.is_some()
            && now.duration_since(*self.unwanted_since.get_or_insert(now)) < HDR_LINGER;
        let target = (capable && (wanted || lingering)).then_some(sdr_white);
        let tag = if wanted { headroom } else { self.headroom };
        let retag = target.is_some() && (tag - self.headroom).abs() > 0.01 * tag;
        (target != self.sdr_white || retag).then(|| {
            target.map(|sdr_white| gpui::HdrOutput {
                sdr_white,
                content_headroom: tag,
            })
        })
    }

    /// Takes the window's answer to `request`.
    fn answered(&mut self, request: Option<gpui::HdrOutput>, set: bool) {
        if set || request.is_none() {
            self.sdr_white = request.map(|r| r.sdr_white);
            self.headroom = request.map_or(1.0, |r| r.content_headroom);
        } else {
            self.refused = true;
            self.sdr_white = None;
        }
    }
}

/// What has been reported of a display: a flag change at once, a level change past 1 % at most
/// every [`LEVEL_EVERY`], and a `variable_refresh` change once it held for [`REFRESH_HOLD`].
#[derive(Default)]
struct Reported {
    display: SurfaceDisplay,
    at: Option<Instant>,
    /// A `variable_refresh` other than the reported one, and since when it is read.
    refresh: Option<(Option<bool>, Instant)>,
}

impl Reported {
    fn offer(&mut self, mut read: SurfaceDisplay, now: Instant) {
        let shown = self.display;
        if shown.variable_refresh.is_some() && read.variable_refresh != shown.variable_refresh {
            let since = match self.refresh {
                Some((value, since)) if value == read.variable_refresh => since,
                _ => self.refresh.insert((read.variable_refresh, now)).1,
            };
            if now.duration_since(since) < REFRESH_HOLD {
                read.variable_refresh = shown.variable_refresh;
            }
        } else {
            self.refresh = None;
        }
        let flag = read.known != shown.known
            || read.hdr != shown.hdr
            || read.output_hdr != shown.output_hdr
            || read.variable_refresh != shown.variable_refresh;
        let due = self
            .at
            .is_none_or(|at| now.duration_since(at) >= LEVEL_EVERY);
        if flag || (due && read.differs(&shown)) {
            self.display = read;
            self.at = Some(now);
        }
    }
}

static ANY_CAPABLE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static PROBES: RefCell<HashMap<WinKey, Probe>> = RefCell::new(HashMap::new());
}

/// Reads `window`'s display.
pub(super) fn sample(window: &Window, key: WinKey) {
    if !platform::SAMPLED {
        return;
    }
    PROBES.with(|p| {
        let mut probes = p.borrow_mut();
        let probe = probes.entry(key).or_default();
        let read = platform::read(window, &mut probe.platform);
        probe.take(read, Instant::now());
        ANY_CAPABLE.store(probes.values().any(Probe::capable), Ordering::Relaxed);
    });
}

/// A macOS screen read at `current` EDR headroom, as [`sample`] takes it.
#[cfg(test)]
pub(super) fn sample_edr(key: WinKey, current: f64, now: Instant) {
    PROBES.with(|p| {
        let read = macos_display(current, 16.0, 120, 1.0 / 120.0, 1.0 / 24.0);
        p.borrow_mut().entry(key).or_default().take(Some(read), now);
    });
}

/// Whether `key`'s window can draw HDR on its display now (`dynamic-range: high`).
pub(super) fn dynamic_range_high(key: WinKey) -> bool {
    PROBES.with(|p| p.borrow().get(&key).is_some_and(Probe::capable))
}

/// The most headroom `key`'s window can get: the display's potential where it can draw HDR, else
/// 1.0. macOS raises the current headroom only once a window asks for EDR, so asking goes by this.
pub(super) fn reach(key: WinKey) -> f32 {
    PROBES.with(|p| {
        p.borrow()
            .get(&key)
            .filter(|probe| probe.capable())
            .map_or(1.0, |probe| probe.display.potential_headroom)
    })
}

/// Whether any window can draw HDR, so a half-float CPU frame keeps its values for one.
pub(super) fn any_capable() -> bool {
    ANY_CAPABLE.load(Ordering::Relaxed)
}

/// Switches `window` to HDR output while it can and its content wants more than SDR white
/// (`headroom` > 1.0), asking for that headroom, and back to standard [`HDR_LINGER`] after it no
/// longer does. Returns whether it draws HDR.
pub(super) fn update_output(window: &Window, key: WinKey, headroom: f32) -> bool {
    PROBES.with(|p| {
        let mut probes = p.borrow_mut();
        let Some(probe) = probes.get_mut(&key) else {
            return false;
        };
        let capable = probe.capable();
        // Windows composes SDR content at its SDR white; macOS EDR is relative to it.
        let sdr_white = probe.display.sdr_white_nits.map_or(1.0, |nits| nits / 80.0);
        let output = &mut probe.output;
        if let Some(request) = output.request(capable, headroom, sdr_white, Instant::now()) {
            let set = window.set_hdr_output(request);
            let was = output.sdr_white.is_some();
            output.answered(request, set);
            if output.refused {
                probe.display.output_headroom = 1.0;
            }
            match (was, output.sdr_white.is_some()) {
                (false, true) => {
                    log::info!("[sk] window {key} switched to HDR output (half float)")
                }
                (true, false) => {
                    log::info!("[sk] window {key} switched to standard output (8-bit)")
                }
                _ => {}
            }
        }
        let hdr = probe.output.sdr_white.is_some();
        probe.display.output_hdr = hdr;
        probe.reported.offer(probe.display, Instant::now());
        hdr
    })
}

/// What is reported of `key`'s display; unknown before its first read.
pub(super) fn of(key: WinKey) -> SurfaceDisplay {
    PROBES.with(|p| {
        p.borrow()
            .get(&key)
            .map(|p| p.reported.display)
            .unwrap_or_default()
    })
}

/// What `key`'s display reads now, for painting: reports are spaced, a picture is not.
pub(super) fn live(key: WinKey) -> SurfaceDisplay {
    PROBES.with(|p| p.borrow().get(&key).map(|p| p.display).unwrap_or_default())
}

pub(super) fn forget(key: WinKey) {
    PROBES.with(|p| p.borrow_mut().remove(&key));
}

static WATCHING: AtomicBool = AtomicBool::new(false);
static LAST_TICK: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Called each primary tick: keeps an idle window ticking every [`IDLE`] while `visible`.
pub(super) fn note_tick(visible: bool) {
    if !platform::SAMPLED {
        return;
    }
    WATCHING.store(visible, Ordering::Relaxed);
    LAST_TICK.store(now_ms(), Ordering::Relaxed);
    if visible {
        static SPAWN: Once = Once::new();
        SPAWN.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("sk-surface-display".into())
                .spawn(|| loop {
                    std::thread::sleep(IDLE);
                    let idle = now_ms().saturating_sub(LAST_TICK.load(Ordering::Relaxed));
                    if WATCHING.load(Ordering::Relaxed) && idle >= IDLE.as_millis() as u64 {
                        sk_runtime::frames::wake();
                    }
                });
        });
    }
}

/// `DISPLAYCONFIG_SDR_WHITE_LEVEL::SDRWhiteLevel` is in thousandths of scRGB's 80 nits.
fn sdr_white_nits(level: u32) -> f32 {
    level as f32 / 1000.0 * 80.0
}

/// Windows: in HDR mode SDR content is composed at the user's SDR white, so the display's peak over
/// it is the room above 1.0. In SDR mode there is none, and the white level Windows reports is only
/// its nominal 80 nits, so it is left out.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_display(
    hdr: bool,
    peak_nits: f32,
    full_frame_nits: f32,
    white_level: Option<u32>,
    refresh_hz: Option<f32>,
) -> SurfaceDisplay {
    let known = |nits: f32| (nits > 0.0).then_some(nits);
    let sdr_white_nits = white_level.map(sdr_white_nits).filter(|_| hdr);
    let headroom = match (sdr_white_nits, known(peak_nits)) {
        (Some(white), Some(peak)) => (peak / white).max(1.0),
        _ => 1.0,
    };
    SurfaceDisplay {
        known: true,
        hdr,
        headroom,
        potential_headroom: headroom,
        sdr_white_nits,
        peak_nits: known(peak_nits),
        full_frame_nits: known(full_frame_nits),
        refresh_hz,
        ..SurfaceDisplay::default()
    }
}

/// macOS: EDR values are already relative to SDR white. A screen whose refresh interval has a
/// range (ProMotion, Adaptive-Sync) refreshes at a variable rate.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_display(
    current: f64,
    potential: f64,
    max_fps: isize,
    min_interval: f64,
    max_interval: f64,
) -> SurfaceDisplay {
    let headroom = (current as f32).max(1.0);
    let potential_headroom = (potential as f32).max(headroom);
    SurfaceDisplay {
        known: true,
        hdr: potential_headroom > 1.0,
        headroom,
        potential_headroom,
        refresh_hz: (max_fps > 0).then_some(max_fps as f32),
        variable_refresh: (min_interval > 0.0 && max_interval > 0.0)
            .then_some(max_interval > min_interval * 1.05),
        ..SurfaceDisplay::default()
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::core::Interface;
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
        DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
        DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
        DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE,
        DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
        DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
    };
    use windows::Win32::Foundation::{HWND, LUID};
    use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput6};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITORINFOEXW,
        MONITOR_DEFAULTTONEAREST,
    };

    use super::SurfaceDisplay;

    pub(super) const SAMPLED: bool = true;

    /// The SDR brightness slider and a refresh-rate change send no notification a window sees, so
    /// the display configuration is read this often.
    const POLL: Duration = Duration::from_secs(1);

    #[derive(Default)]
    pub(super) struct State {
        monitor: isize,
        factory: Option<IDXGIFactory1>,
        /// HDR on, peak and full-frame nits, from DXGI.
        output: (bool, f32, f32),
        config: Config,
        polled: Option<Instant>,
        /// The last periodic read taken.
        seen: u64,
    }

    #[derive(Clone, Copy, Default, PartialEq)]
    struct Config {
        white_level: Option<u32>,
        advanced_color: bool,
        refresh_hz: Option<f32>,
    }

    fn hwnd(window: &Window) -> Option<HWND> {
        match <Window as HasWindowHandle>::window_handle(window)
            .ok()?
            .as_raw()
        {
            RawWindowHandle::Win32(w) => Some(HWND(w.hwnd.get() as *mut c_void)),
            _ => None,
        }
    }

    pub(super) fn read(window: &Window, state: &mut State) -> Option<SurfaceDisplay> {
        // SAFETY: a live window's handle.
        let monitor = unsafe { MonitorFromWindow(hwnd(window)?, MONITOR_DEFAULTTONEAREST) };
        let moved = monitor.0 as isize != state.monitor;
        state.monitor = monitor.0 as isize;
        // DXGI's factory goes stale when outputs or their modes change (WM_DISPLAYCHANGE).
        // SAFETY: plain COM calls.
        let stale = state
            .factory
            .as_ref()
            .is_none_or(|f| !unsafe { f.IsCurrent() }.as_bool());
        let mut flipped = false;
        // Read on the tick only where the answer is needed now; the periodic read is taken on a
        // later tick.
        let config = if moved || stale || state.polled.is_none() {
            state.polled = Some(Instant::now());
            let _ = polled::take(state.monitor, &mut state.seen);
            Some(display_config(monitor).unwrap_or_default())
        } else if state.polled.is_some_and(|t| t.elapsed() >= POLL) {
            state.polled = Some(Instant::now());
            match polled::ask(state.monitor) {
                true => polled::take(state.monitor, &mut state.seen),
                false => Some(display_config(monitor).unwrap_or_default()),
            }
        } else {
            polled::take(state.monitor, &mut state.seen)
        };
        if let Some(config) = config {
            flipped = config.advanced_color != state.config.advanced_color;
            state.config = config;
        }
        // Some drivers leave the factory current across an HDR switch; the advanced-colour flag
        // flipping catches those.
        if moved || stale || flipped {
            if stale {
                state.factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.ok();
            }
            state.output = state
                .factory
                .as_ref()
                .and_then(|f| output(f, monitor))
                .unwrap_or_default();
        }
        let (hdr, peak, full_frame) = state.output;
        Some(super::windows_display(
            hdr,
            peak,
            full_frame,
            state.config.white_level,
            state.config.refresh_hz,
        ))
    }

    fn output(factory: &IDXGIFactory1, monitor: HMONITOR) -> Option<(bool, f32, f32)> {
        for a in 0.. {
            // SAFETY: plain COM calls; enumeration ends with an error.
            let adapter = unsafe { factory.EnumAdapters1(a) }.ok()?;
            for o in 0.. {
                let Ok(output) = (unsafe { adapter.EnumOutputs(o) }) else {
                    break;
                };
                let Ok(desc) = (unsafe { output.GetDesc() }) else {
                    continue;
                };
                if desc.Monitor != monitor {
                    continue;
                }
                let desc = unsafe { output.cast::<IDXGIOutput6>().ok()?.GetDesc1() }.ok()?;
                return Some((
                    desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
                    desc.MaxLuminance,
                    desc.MaxFullFrameLuminance,
                ));
            }
        }
        None
    }

    fn header<T>(
        kind: DISPLAYCONFIG_DEVICE_INFO_TYPE,
        adapter: LUID,
        id: u32,
    ) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
        DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: kind,
            size: std::mem::size_of::<T>() as u32,
            adapterId: adapter,
            id,
        }
    }

    /// The periodic read of the display configuration, off the tick: it takes tens of microseconds,
    /// at times most of a millisecond, before the tick takes the surfaces' frames.
    mod polled {
        use std::sync::mpsc::{channel, Sender};
        use std::sync::{Mutex, OnceLock};

        use super::{display_config, Config, HMONITOR};

        /// The latest read of each monitor, numbered.
        static READ: Mutex<Vec<(isize, u64, Config)>> = Mutex::new(Vec::new());

        /// Asks for `monitor`'s configuration; `false` where there is no thread to read it.
        pub(super) fn ask(monitor: isize) -> bool {
            static ASK: OnceLock<Option<Sender<isize>>> = OnceLock::new();
            let ask = ASK.get_or_init(|| {
                let (ask, asked) = channel::<isize>();
                std::thread::Builder::new()
                    .name("sk-display-config".into())
                    .spawn(move || {
                        for (n, monitor) in (1..).zip(asked) {
                            let config =
                                display_config(HMONITOR(monitor as *mut _)).unwrap_or_default();
                            let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
                            read.retain(|(m, ..)| *m != monitor);
                            read.push((monitor, n, config));
                        }
                    })
                    .ok()
                    .map(|_| ask)
            });
            ask.as_ref().is_some_and(|ask| ask.send(monitor).is_ok())
        }

        /// `monitor`'s configuration if it was read after the one numbered `seen`.
        pub(super) fn take(monitor: isize, seen: &mut u64) -> Option<Config> {
            let read = READ.lock().unwrap_or_else(|e| e.into_inner());
            let &(_, n, config) = read.iter().find(|(m, ..)| *m == monitor)?;
            (n > *seen).then(|| {
                *seen = n;
                config
            })
        }
    }

    /// The monitor's SDR white level, advanced-colour state and refresh rate.
    fn display_config(monitor: HMONITOR) -> Option<Config> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` is a MONITORINFOEXW, as cbSize says.
        unsafe {
            GetMonitorInfoW(
                monitor,
                (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
            )
        }
        .as_bool()
        .then_some(())?;
        let (mut path_count, mut mode_count) = (0u32, 0u32);
        // SAFETY: the buffers are sized from GetDisplayConfigBufferSizes.
        unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
                .ok()
                .ok()?;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
            .ok()
            .ok()?;
        }
        let name = |wide: &[u16]| {
            wide.iter()
                .take_while(|&&c| c != 0)
                .copied()
                .collect::<Vec<_>>()
        };
        let device = name(&info.szDevice);
        for path in &paths[..path_count as usize] {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: header::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(
                    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    path.sourceInfo.adapterId,
                    path.sourceInfo.id,
                ),
                ..Default::default()
            };
            // SAFETY: each packet's header names its own type and size.
            if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0
                || name(&source.viewGdiDeviceName) != device
            {
                continue;
            }
            let (adapter, id) = (path.targetInfo.adapterId, path.targetInfo.id);
            let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: header::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(
                    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    adapter,
                    id,
                ),
                ..Default::default()
            };
            let white_level = (unsafe { DisplayConfigGetDeviceInfo(&mut white.header) } == 0)
                .then_some(white.SDRWhiteLevel);
            let mut color = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                header: header::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(
                    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                    adapter,
                    id,
                ),
                ..Default::default()
            };
            // Bit 1 of the flags is advancedColorEnabled.
            let advanced_color = unsafe { DisplayConfigGetDeviceInfo(&mut color.header) } == 0
                && unsafe { color.Anonymous.value } & 0x2 != 0;
            let rate = path.targetInfo.refreshRate;
            let refresh_hz = (rate.Denominator > 0 && rate.Numerator > 0)
                .then(|| (f64::from(rate.Numerator) / f64::from(rate.Denominator)) as f32);
            return Some(Config {
                white_level,
                advanced_color,
                refresh_hz,
            });
        }
        None
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use gpui::Window;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::SurfaceDisplay;

    pub(super) const SAMPLED: bool = true;

    #[derive(Default)]
    pub(super) struct State;

    pub(super) fn read(window: &Window, _: &mut State) -> Option<SurfaceDisplay> {
        let RawWindowHandle::AppKit(handle) = <Window as HasWindowHandle>::window_handle(window)
            .ok()?
            .as_raw()
        else {
            return None;
        };
        let view = handle.ns_view.as_ptr().cast::<AnyObject>();
        // SAFETY: the view lives as long as its window, which this frame is drawing; these are
        // AppKit property getters (macOS 12+), called on the main thread.
        unsafe {
            let window: *mut AnyObject = msg_send![&*view, window];
            let screen: *mut AnyObject = msg_send![window.as_ref()?, screen];
            let screen = screen.as_ref()?;
            let current: f64 = msg_send![screen, maximumExtendedDynamicRangeColorComponentValue];
            let potential: f64 = msg_send![
                screen,
                maximumPotentialExtendedDynamicRangeColorComponentValue
            ];
            let max_fps: isize = msg_send![screen, maximumFramesPerSecond];
            let min_interval: f64 = msg_send![screen, minimumRefreshInterval];
            let max_interval: f64 = msg_send![screen, maximumRefreshInterval];
            Some(super::macos_display(
                current,
                potential,
                max_fps,
                min_interval,
                max_interval,
            ))
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use gpui::Window;

    use super::SurfaceDisplay;

    pub(super) const SAMPLED: bool = false;

    #[derive(Default)]
    pub(super) struct State;

    pub(super) fn read(_: &Window, _: &mut State) -> Option<SurfaceDisplay> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_values_are_relative_to_sdr_white_and_scrgb_one_is_80_nits() {
        assert_eq!(sdr_white_nits(1000), 80.0);
        assert_eq!(sdr_white_nits(3000), 240.0);
        let hdr = windows_display(true, 960.0, 600.0, Some(3000), Some(144.0));
        assert_eq!((hdr.headroom, hdr.potential_headroom), (4.0, 4.0));
        assert_eq!(hdr.sdr_white_nits, Some(240.0));
        assert_eq!(hdr.nits(1.0), Some(240.0), "1.0 is SDR white");
        assert_eq!(
            hdr.nits(hdr.headroom),
            Some(960.0),
            "the headroom reaches the peak"
        );
        let scrgb_one = hdr.nits(80.0 / 240.0).unwrap();
        assert!(
            (scrgb_one - 80.0).abs() < 1e-3,
            "scRGB 1.0 is 80 nits: {scrgb_one}"
        );
        assert_eq!(
            hdr.output_headroom, 1.0,
            "raised once the window is known to draw HDR"
        );
        assert_eq!((hdr.refresh_hz, hdr.variable_refresh), (Some(144.0), None));

        let sdr = windows_display(false, 300.0, 300.0, Some(1000), Some(60.0));
        assert_eq!(
            (sdr.hdr, sdr.headroom, sdr.sdr_white_nits),
            (false, 1.0, None)
        );
        let no_edid = windows_display(true, 0.0, 0.0, Some(1000), None);
        assert_eq!((no_edid.headroom, no_edid.peak_nits), (1.0, None));
    }

    #[test]
    fn macos_edr_values_are_taken_as_they_are() {
        let edr = macos_display(2.5, 16.0, 120, 1.0 / 120.0, 1.0 / 24.0);
        assert!(edr.known && edr.hdr);
        assert_eq!(
            (edr.headroom, edr.potential_headroom, edr.sdr_white_nits),
            (2.5, 16.0, None)
        );
        assert_eq!(
            (edr.refresh_hz, edr.variable_refresh),
            (Some(120.0), Some(true))
        );
        let sdr = macos_display(1.0, 1.0, 60, 1.0 / 60.0, 1.0 / 60.0);
        assert_eq!((sdr.hdr, sdr.variable_refresh), (false, Some(false)));
    }

    /// macOS reports a current headroom of 1.0 until a window asks for EDR.
    #[test]
    fn a_window_asks_for_the_headroom_its_screen_can_reach_not_what_it_shows_now() {
        let probe = |display| Probe {
            display,
            ..Probe::default()
        };
        PROBES.with(|p| {
            let mut probes = p.borrow_mut();
            probes.insert(1, probe(macos_display(1.0, 16.0, 60, 0.0, 0.0)));
            probes.insert(2, probe(macos_display(1.0, 1.0, 60, 0.0, 0.0)));
            let mut refused = probe(macos_display(1.0, 16.0, 60, 0.0, 0.0));
            refused.output.refused = true;
            probes.insert(3, refused);
        });
        assert_eq!(
            (reach(1), reach(2), reach(3), reach(4)),
            (16.0, 1.0, 1.0, 1.0)
        );
        for key in 1..=3 {
            forget(key);
        }
    }

    /// macOS lets the headroom fall back to 1.0 some 15 s after nothing on the screen shows EDR.
    #[test]
    fn a_window_asks_for_edr_on_a_screen_at_sdr_white_and_again_after_an_edr_pause() {
        use sk_css::DynamicRangeLimit as Limit;
        let key = 7;
        let start = Instant::now();
        let screen = |current| {
            PROBES.with(|p| {
                p.borrow_mut().entry(key).or_default().display =
                    macos_display(current, 16.0, 120, 0.0, 0.0)
            })
        };
        let ask = |limit: Limit, secs: f32| {
            let headroom = limit.headroom(reach(key));
            PROBES.with(|p| {
                let mut probes = p.borrow_mut();
                let probe = probes.get_mut(&key).expect("probed");
                let now = start + Duration::from_secs_f32(secs);
                let capable = probe.capable();
                let request = probe.output.request(capable, headroom, 1.0, now);
                if let Some(request) = request {
                    probe.output.answered(request, true);
                }
                request.map(|r| r.map(|r| r.content_headroom))
            })
        };
        screen(1.0);
        assert_eq!(ask(Limit::NO_LIMIT, 0.0), Some(Some(16.0)));
        screen(6.11);
        assert_eq!(ask(Limit::NO_LIMIT, 1.0), None, "the ramp asks nothing new");
        assert_eq!(ask(Limit::CONSTRAINED, 1.5), Some(Some(2.0)), "retagged");
        assert_eq!(ask(Limit::STANDARD, 2.0), None, "lingers");
        assert_eq!(ask(Limit::STANDARD, 4.1), Some(None), "back to standard");
        screen(1.0);
        assert_eq!(ask(Limit::STANDARD, 20.0), None);
        assert_eq!(ask(Limit::NO_LIMIT, 21.0), Some(Some(16.0)), "asks again");
        forget(key);
    }

    #[test]
    fn a_headroom_ramp_is_reported_a_few_times_and_where_it_settles() {
        let start = Instant::now();
        let mut reported = Reported::default();
        let mut heard: Vec<(u64, f32)> = Vec::new();
        // 120 Hz frames over 1.6 s, the headroom rising from 1.0 to 6.11, then idle ticks.
        let frames = 192;
        let ramp = (0..=frames).map(|f| (f * 1000 / 120, 1.0 + 5.11 * f as f64 / frames as f64));
        let idle = (1..=4).map(|i| (frames * 1000 / 120 + i * 250, 6.11));
        for (ms, headroom) in ramp.chain(idle) {
            let mut read = macos_display(headroom, 16.0, 120, 1.0 / 120.0, 1.0 / 24.0);
            read.output_hdr = true;
            read.output_headroom = read.headroom;
            let before = reported.display;
            reported.offer(read, start + Duration::from_millis(ms));
            if reported.display != before {
                heard.push((ms, reported.display.headroom));
            }
        }
        assert!(heard.len() <= 10, "{} reports: {heard:?}", heard.len());
        assert!(
            heard.windows(2).all(|w| w[1].0 - w[0].0 >= 200),
            "{heard:?}"
        );
        let (last, settled) = *heard.last().expect("reported");
        assert!((settled - 6.11).abs() <= 0.01 * 6.11, "{heard:?}");

        let mut step = reported.display;
        step.headroom = 3.0;
        reported.offer(step, start + Duration::from_millis(last + 300));
        assert_eq!(
            reported.display.headroom, 3.0,
            "a step after a quiet spell is reported at once"
        );
    }

    #[test]
    fn a_variable_refresh_blink_is_not_reported_and_a_change_that_holds_is() {
        let start = Instant::now();
        let mut reported = Reported::default();
        let read = |variable: bool| {
            let max_interval = if variable { 1.0 / 24.0 } else { 1.0 / 120.0 };
            macos_display(1.0, 16.0, 120, 1.0 / 120.0, max_interval)
        };
        let at = |ms| start + Duration::from_millis(ms);
        reported.offer(read(true), at(0));
        assert_eq!(reported.display.variable_refresh, Some(true));
        reported.offer(read(false), at(1000));
        reported.offer(read(true), at(1008));
        reported.offer(read(true), at(1500));
        assert_eq!(reported.display.variable_refresh, Some(true), "a blink");
        reported.offer(read(false), at(2000));
        reported.offer(read(false), at(2050));
        assert_eq!(reported.display.variable_refresh, Some(true));
        reported.offer(read(false), at(2100));
        assert_eq!(reported.display.variable_refresh, Some(false), "held");
    }
}
