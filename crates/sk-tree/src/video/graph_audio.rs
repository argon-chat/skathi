use super::*;
use sk_runtime::redjs::webapi::audio::{ElementError, MediaInput};
use sk_video::AudioOutput;
use std::ffi::c_void;

thread_local! {
    static INPUTS: RefCell<HashMap<u32, Arc<MediaInput>>> = RefCell::default();
}

struct Output(Arc<MediaInput>);

impl AudioOutput for Output {
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }
    fn push(&self, samples: &[f32], channels: usize, rate: u32, gain: f32) {
        self.0.push(samples, channels, rate, gain)
    }
    fn flush(&self) {
        self.0.flush()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.0.flush()
    }
}

pub(crate) unsafe fn connect(
    g: *mut c_void,
    el: u64,
    input: Arc<MediaInput>,
) -> Result<(), ElementError> {
    let id = crate::native_dom::node_id(g, el)
        .filter(|&id| crate::native_dom::node_by_id(g, id) == el)
        .ok_or(ElementError::InvalidElement)?;
    if !tree::read_tree(|t| {
        t.node(id)
            .is_some_and(|n| matches!(t.name(n.tag), "audio" | "video"))
    }) {
        return Err(ElementError::InvalidElement);
    }
    INPUTS.with(|inputs| {
        let mut inputs = inputs.borrow_mut();
        if inputs.contains_key(&id) {
            return Err(ElementError::AlreadyConnected);
        }
        if let Some(result) = with_state(id, |state| {
            state.route_audio(Arc::new(Output(input.clone())))
        }) {
            result.map_err(|e| ElementError::Unavailable(e.message))?;
        }
        inputs.insert(id, input);
        Ok(())
    })?;
    track_audio(id);
    if api_of(id, false).playing == Some(true) {
        sync_audio_playback(id, true);
    } else {
        tree::read_tree(|t| {
            if t.connected(id) {
                sync_audio_element(t, id);
            }
        });
    }
    Ok(())
}

pub(super) fn restore(id: u32, src: &str, mut state: VideoState) -> Option<VideoState> {
    if let Some(input) = INPUTS.with(|m| m.borrow().get(&id).cloned()) {
        if let Err(error) = state.route_audio(Arc::new(Output(input))) {
            report(src, &error);
            return None;
        }
    }
    Some(state)
}

pub(super) fn gc(live: &impl Fn(u32) -> bool) {
    INPUTS.with(|m| m.borrow_mut().retain(|id, _| live(*id)))
}
