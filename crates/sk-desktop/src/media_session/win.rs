//! The window's System Media Transport Controls: the media flyout, media keys, the lock screen.

use std::cell::RefCell;
use std::ffi::c_void;

use windows::core::HSTRING;
use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
    SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::{
    DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

use super::event;

thread_local! {
    static SMTC: RefCell<Option<(isize, SystemMediaTransportControls)>> = const { RefCell::new(None) };
}

fn action_of(b: SystemMediaTransportControlsButton) -> Option<&'static str> {
    Some(match b {
        SystemMediaTransportControlsButton::Play => "play",
        SystemMediaTransportControlsButton::Pause => "pause",
        SystemMediaTransportControlsButton::Stop => "stop",
        SystemMediaTransportControlsButton::Next => "nexttrack",
        SystemMediaTransportControlsButton::Previous => "previoustrack",
        SystemMediaTransportControlsButton::FastForward => "seekforward",
        SystemMediaTransportControlsButton::Rewind => "seekbackward",
        _ => return None,
    })
}

fn controls(hwnd: isize) -> windows::core::Result<SystemMediaTransportControls> {
    if let Some(c) = SMTC.with(|s| {
        s.borrow()
            .as_ref()
            .filter(|(h, _)| *h == hwnd)
            .map(|(_, c)| c.clone())
    }) {
        return Ok(c);
    }
    // SAFETY: COM on the window's thread; an initialised apartment is fine.
    let c: SystemMediaTransportControls = unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let interop = windows::core::factory::<
            SystemMediaTransportControls,
            ISystemMediaTransportControlsInterop,
        >()?;
        interop.GetForWindow(HWND(hwnd as *mut c_void))?
    };
    c.ButtonPressed(&TypedEventHandler::<
        SystemMediaTransportControls,
        SystemMediaTransportControlsButtonPressedEventArgs,
    >::new(|_, args| {
        if let Some(a) = args
            .as_ref()
            .and_then(|a| a.Button().ok())
            .and_then(action_of)
        {
            event(a, None);
        }
        Ok(())
    }))?;
    c.PlaybackPositionChangeRequested(&TypedEventHandler::<
        SystemMediaTransportControls,
        PlaybackPositionChangeRequestedEventArgs,
    >::new(|_, args| {
        if let Some(p) = args
            .as_ref()
            .and_then(|a| a.RequestedPlaybackPosition().ok())
        {
            event("seekto", Some(p.Duration as f64 / 1e7));
        }
        Ok(())
    }))?;
    SMTC.with(|s| *s.borrow_mut() = Some((hwnd, c.clone())));
    Ok(c)
}

fn span(seconds: f64) -> TimeSpan {
    TimeSpan {
        Duration: (seconds.max(0.0) * 1e7) as i64,
    }
}

fn thumbnail(
    url: Option<&str>,
    bytes: Option<&[u8]>,
) -> windows::core::Result<Option<RandomAccessStreamReference>> {
    if let Some(b) = bytes {
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(b)?;
        writer.StoreAsync()?.join()?;
        writer.DetachStream()?;
        stream.Seek(0)?;
        return RandomAccessStreamReference::CreateFromStream(&stream).map(Some);
    }
    match url {
        Some(u) => RandomAccessStreamReference::CreateFromUri(&Uri::CreateUri(&HSTRING::from(u))?)
            .map(Some),
        None => Ok(None),
    }
}

/// `state` is the session as runtime.ts sends it; `artwork` the image bytes when it is not an http(s) URL.
pub fn update(
    owner: Option<isize>,
    state: &serde_json::Value,
    artwork: Option<&[u8]>,
) -> windows::core::Result<()> {
    let Some(hwnd) = owner else { return Ok(()) };
    let c = controls(hwnd)?;
    let s = |k: &str| state[k].as_str().unwrap_or("");
    let actions: Vec<&str> = state["actions"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let has = |a: &str| actions.contains(&a);
    let playback = s("playbackState");
    let enabled = state["metadata"].is_object() || playback != "none";
    c.SetIsEnabled(enabled)?;
    if !enabled {
        return Ok(());
    }
    c.SetPlaybackStatus(match playback {
        "playing" => MediaPlaybackStatus::Playing,
        "paused" => MediaPlaybackStatus::Paused,
        _ => MediaPlaybackStatus::Stopped,
    })?;
    c.SetIsPlayEnabled(has("play"))?;
    c.SetIsPauseEnabled(has("pause"))?;
    c.SetIsStopEnabled(has("stop"))?;
    c.SetIsNextEnabled(has("nexttrack"))?;
    c.SetIsPreviousEnabled(has("previoustrack"))?;
    c.SetIsFastForwardEnabled(has("seekforward"))?;
    c.SetIsRewindEnabled(has("seekbackward"))?;

    let d = c.DisplayUpdater()?;
    d.ClearAll()?;
    if let Some(m) = state["metadata"].as_object() {
        let field = |k: &str| HSTRING::from(m.get(k).and_then(|v| v.as_str()).unwrap_or(""));
        d.SetType(MediaPlaybackType::Music)?;
        let music = d.MusicProperties()?;
        music.SetTitle(&field("title"))?;
        music.SetArtist(&field("artist"))?;
        music.SetAlbumTitle(&field("album"))?;
        // A broken image must not cost the title and the buttons.
        if let Ok(Some(t)) = thumbnail(m.get("artworkUrl").and_then(|v| v.as_str()), artwork) {
            d.SetThumbnail(&t)?;
        }
    }
    d.Update()?;

    let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
    if let Some(p) = state["position"].as_object() {
        let n = |k: &str| p.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let duration = if n("duration").is_finite() {
            n("duration")
        } else {
            0.0
        };
        timeline.SetStartTime(span(0.0))?;
        timeline.SetMinSeekTime(span(0.0))?;
        timeline.SetEndTime(span(duration))?;
        timeline.SetMaxSeekTime(span(if has("seekto") { duration } else { 0.0 }))?;
        timeline.SetPosition(span(n("position")))?;
        c.SetPlaybackRate(n("playbackRate"))?;
    }
    c.UpdateTimelineProperties(&timeline)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn buttons_map_to_media_session_actions() {
        use windows::Media::SystemMediaTransportControlsButton as B;
        assert_eq!(super::action_of(B::Play), Some("play"));
        assert_eq!(super::action_of(B::Next), Some("nexttrack"));
        assert_eq!(super::action_of(B::Rewind), Some("seekbackward"));
        assert_eq!(super::action_of(B::Record), None);
    }

    #[test]
    fn a_session_drives_the_window_controls() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED,
        };
        // SAFETY: an invisible top-level window of a system class, destroyed below.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("smtc"),
                WS_OVERLAPPED,
                0,
                0,
                10,
                10,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let h = hwnd.0 as isize;
        let state = serde_json::json!({
            "metadata": { "title": "Song", "artist": "Band", "album": "LP", "artworkUrl": "https://example.com/a.png" },
            "playbackState": "playing",
            "actions": ["play", "pause", "nexttrack", "seekto"],
            "position": { "duration": 200.0, "position": 12.5, "playbackRate": 1.0 },
        });
        super::update(Some(h), &state, None).unwrap();
        let c = super::controls(h).unwrap();
        assert!(
            c.IsEnabled().unwrap() && c.IsNextEnabled().unwrap() && !c.IsPreviousEnabled().unwrap()
        );
        assert_eq!(
            c.PlaybackStatus().unwrap(),
            super::MediaPlaybackStatus::Playing
        );
        assert_eq!(
            c.DisplayUpdater()
                .unwrap()
                .MusicProperties()
                .unwrap()
                .Title()
                .unwrap(),
            "Song"
        );
        super::update(
            Some(h),
            &serde_json::json!({ "metadata": null, "playbackState": "none", "actions": [] }),
            None,
        )
        .unwrap();
        assert!(!c.IsEnabled().unwrap());
        // SAFETY: the window made above.
        unsafe { DestroyWindow(hwnd).unwrap() };
    }

    #[test]
    fn artwork_bytes_become_a_stream() {
        let png = [0x89, b'P', b'N', b'G', 13, 10, 26, 10];
        assert!(super::thumbnail(None, Some(&png)).unwrap().is_some());
        assert!(super::thumbnail(Some("https://example.com/a.png"), None)
            .unwrap()
            .is_some());
        assert!(super::thumbnail(None, None).unwrap().is_none());
    }
}
