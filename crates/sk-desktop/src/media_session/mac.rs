//! The app's Now Playing entry (Control Center, the media keys) through `MPNowPlayingInfoCenter`, with
//! `MPRemoteCommandCenter` commands enabled for the actions the page handles. Commands run on the main
//! thread.

use std::cell::Cell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSMutableDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle,
    MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
    MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommand,
    MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};

use super::event;

static PLAYING: AtomicBool = AtomicBool::new(false);

thread_local! {
    static HOOKED: Cell<bool> = const { Cell::new(false) };
}

fn on(command: &MPRemoteCommand, f: impl Fn(NonNull<MPRemoteCommandEvent>) + 'static) {
    let handler = RcBlock::new(move |e: NonNull<MPRemoteCommandEvent>| {
        f(e);
        MPRemoteCommandHandlerStatus::Success
    });
    // SAFETY: the command copies the block; the target it returns lives as long as the command.
    let _ = unsafe { command.addTargetWithHandler(&handler) };
}

fn hook(center: &MPRemoteCommandCenter) {
    if HOOKED.with(|h| h.replace(true)) {
        return;
    }
    // SAFETY: getters on the shared command center; a position command's events are position changes.
    unsafe {
        for (command, action) in [
            (center.playCommand(), "play"),
            (center.pauseCommand(), "pause"),
            (center.stopCommand(), "stop"),
            (center.nextTrackCommand(), "nexttrack"),
            (center.previousTrackCommand(), "previoustrack"),
        ] {
            on(&command, move |_| event(action, None));
        }
        on(&center.skipForwardCommand(), |_| event("seekforward", None));
        on(&center.skipBackwardCommand(), |_| {
            event("seekbackward", None)
        });
        on(&center.togglePlayPauseCommand(), |_| {
            event(
                if PLAYING.load(Ordering::Relaxed) {
                    "pause"
                } else {
                    "play"
                },
                None,
            )
        });
        on(&center.changePlaybackPositionCommand(), |e| {
            event(
                "seekto",
                Some(
                    e.cast::<MPChangePlaybackPositionCommandEvent>()
                        .as_ref()
                        .positionTime(),
                ),
            );
        });
    }
}

fn artwork(bytes: &[u8]) -> Option<Retained<MPMediaItemArtwork>> {
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?;
    let size = image.size();
    let handler = RcBlock::new(move |_: CGSize| NonNull::from(&*image));
    // SAFETY: the block keeps the image alive for as long as the artwork can ask for it.
    Some(unsafe {
        MPMediaItemArtwork::initWithBoundsSize_requestHandler(
            MPMediaItemArtwork::alloc(),
            size,
            &handler,
        )
    })
}

/// `state` is the session as web/media_session.rs sends it; `artwork` the image bytes.
pub fn update(
    _owner: Option<isize>,
    state: &serde_json::Value,
    artwork_bytes: Option<&[u8]>,
) -> Result<(), String> {
    let actions: Vec<&str> = state["actions"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let has = |a: &str| actions.contains(&a);
    let playback = state["playbackState"].as_str().unwrap_or("none");
    let enabled = state["metadata"].is_object() || playback != "none";
    PLAYING.store(playback == "playing", Ordering::Relaxed);
    // SAFETY: the shared objects, used from the main thread like the commands' handlers.
    unsafe {
        let center = MPRemoteCommandCenter::sharedCommandCenter();
        hook(&center);
        for (command, on) in [
            (center.playCommand(), has("play")),
            (center.pauseCommand(), has("pause")),
            (center.togglePlayPauseCommand(), has("play") || has("pause")),
            (center.stopCommand(), has("stop")),
            (center.nextTrackCommand(), has("nexttrack")),
            (center.previousTrackCommand(), has("previoustrack")),
            (center.skipForwardCommand().into_super(), has("seekforward")),
            (
                center.skipBackwardCommand().into_super(),
                has("seekbackward"),
            ),
            (
                center.changePlaybackPositionCommand().into_super(),
                has("seekto"),
            ),
        ] {
            command.setEnabled(enabled && on);
        }

        let now = MPNowPlayingInfoCenter::defaultCenter();
        if !enabled {
            now.setNowPlayingInfo(None);
            now.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
            return Ok(());
        }
        let info = NSMutableDictionary::<NSString, AnyObject>::new();
        if let Some(m) = state["metadata"].as_object() {
            let text =
                |k: &str| NSString::from_str(m.get(k).and_then(|v| v.as_str()).unwrap_or(""));
            info.insert(MPMediaItemPropertyTitle, &text("title"));
            info.insert(MPMediaItemPropertyArtist, &text("artist"));
            info.insert(MPMediaItemPropertyAlbumTitle, &text("album"));
            if let Some(art) = artwork_bytes.and_then(artwork) {
                info.insert(MPMediaItemPropertyArtwork, &art);
            }
        }
        if let Some(p) = state["position"].as_object() {
            let n = |k: &str| p.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let rate = if playback == "playing" {
                n("playbackRate")
            } else {
                0.0
            };
            info.insert(
                MPMediaItemPropertyPlaybackDuration,
                &NSNumber::new_f64(n("duration")),
            );
            info.insert(
                MPNowPlayingInfoPropertyElapsedPlaybackTime,
                &NSNumber::new_f64(n("position")),
            );
            info.insert(
                MPNowPlayingInfoPropertyPlaybackRate,
                &NSNumber::new_f64(rate),
            );
        }
        now.setNowPlayingInfo(Some(&**info));
        now.setPlaybackState(match playback {
            "playing" => MPNowPlayingPlaybackState::Playing,
            "paused" => MPNowPlayingPlaybackState::Paused,
            _ => MPNowPlayingPlaybackState::Stopped,
        });
    }
    Ok(())
}
