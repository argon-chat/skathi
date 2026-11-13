//! The system's audio device changes, as Windows reports them
//! (`IMMNotificationClient`), handed to redjs (webapi/mediadevices.rs,
//! "Change notifications from the embedder").
//!
//! The callbacks run on a thread of the audio service. They only bump
//! redjs's counters, which are atomics: Windows asks that they never block
//! and never call back into the enumerator.

use vn_runtime::redjs::webapi::mediadevices::{self, Flow};

#[derive(Debug, PartialEq, Eq)]
enum Change {
    Devices,
    Default(Flow),
}

fn report(change: Option<Change>) {
    match change {
        Some(Change::Devices) => mediadevices::notify_devices_changed(),
        Some(Change::Default(flow)) => {
            log::info!("[vn] the system default {} moved", if flow == Flow::Input { "input" } else { "output" });
            mediadevices::notify_default_changed(flow);
        }
        None => {}
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::core::{implement, Result, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::{
        eCapture, eConsole, eRender, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
        IMMNotificationClient_Impl, MMDeviceEnumerator, DEVICE_STATE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    /// role on its own. "Console" is the one CPAL's default is, so it is the
    /// one a page's default follows; the others change nothing it can see.
    pub(super) fn default_change(flow: EDataFlow, role: ERole) -> Option<Change> {
        if role != eConsole {
            return None;
        }
        if flow == eRender {
            Some(Change::Default(Flow::Output))
        } else if flow == eCapture {
            Some(Change::Default(Flow::Input))
        } else {
            None
        }
    }

    #[implement(IMMNotificationClient)]
    pub(super) struct Client;

    impl IMMNotificationClient_Impl for Client_Impl {
        fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
            report(Some(Change::Devices));
            Ok(())
        }
        fn OnDeviceAdded(&self, _id: &PCWSTR) -> Result<()> {
            report(Some(Change::Devices));
            Ok(())
        }
        fn OnDeviceRemoved(&self, _id: &PCWSTR) -> Result<()> {
            report(Some(Change::Devices));
            Ok(())
        }
        fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> Result<()> {
            report(default_change(flow, role));
            Ok(())
        }
        fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
            // Volume, format and name changes arrive here many times a second
            // while a device is in use; none changes the device list.
            Ok(())
        }
    }

    pub(super) fn install() {
        let spawned = std::thread::Builder::new().name("vn-devices".into()).spawn(|| {
            // SAFETY: COM on this thread, multithreaded: the callbacks arrive
            // on the audio service's own threads, never on this one.
            let registered = unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let enumerator: Result<IMMDeviceEnumerator> = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL);
                enumerator.and_then(|e| {
                    let client: IMMNotificationClient = Client.into();
                    e.RegisterEndpointNotificationCallback(&client).map(|()| (e, client))
                })
            };
            match registered {
                Ok(_alive) => {
                    mediadevices::set_host_notifications();
                    log::info!("[vn] audio device notifications on");
                    // Parked for good. The process ends with the pair still
                    // registered, which the audio service handles.
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    // redjs keeps polling: devicechange still fires, every two
                    // seconds at worst, and nothing follows the default.
                    log::warn!("[vn] audio device notifications unavailable ({e}); polling instead");
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("[vn] audio device notifications unavailable ({e}); polling instead");
        }
    }
}

pub fn install() {
    #[cfg(windows)]
    win::install();
}

#[cfg(all(test, windows))]
mod tests {
    use super::win::*;
    use super::*;
    use windows::Win32::Media::Audio::{eAll, eCapture, eCommunications, eConsole, eMultimedia, eRender};

    #[test]
    fn only_the_console_default_moves_what_a_page_sees() {
        assert_eq!(default_change(eRender, eConsole), Some(Change::Default(Flow::Output)));
        assert_eq!(default_change(eCapture, eConsole), Some(Change::Default(Flow::Input)));
        // Windows reports the same move once per role. The other two are not
        // what CPAL calls the default, so following them would move a page's
        // audio to a device its own enumeration does not call default.
        for role in [eMultimedia, eCommunications] {
            assert_eq!(default_change(eRender, role), None);
            assert_eq!(default_change(eCapture, role), None);
        }
        assert_eq!(default_change(eAll, eConsole), None);
    }

    #[test]
    fn the_client_hands_each_notification_to_redjs() {
        use windows::core::PCWSTR;
        use windows::Win32::Media::Audio::{IMMNotificationClient, DEVICE_STATE_ACTIVE};
        let client: IMMNotificationClient = Client.into();
        let input = mediadevices::default_generation(Flow::Input);
        unsafe {
            client.OnDefaultDeviceChanged(eCapture, eConsole, PCWSTR::null()).unwrap();
            client.OnDefaultDeviceChanged(eCapture, eCommunications, PCWSTR::null()).unwrap();
            client.OnDeviceStateChanged(PCWSTR::null(), DEVICE_STATE_ACTIVE).unwrap();
        }
        assert_eq!(mediadevices::default_generation(Flow::Input), input + 1);
    }
}
