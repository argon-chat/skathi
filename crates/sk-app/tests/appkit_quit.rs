#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{env, process::Command, thread, time::Duration};

    use block2::RcBlock;
    use dispatch2::DispatchQueue;
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::NSApplication;
    use objc2_core_services::{
        kAEDefaultTimeout, kAENoReply, kAEQuitApplication, kAEQuitReason, kAEReallyLogOut,
        kAnyTransactionID, kAutoGenerateReturnID, kCoreEventClass, typeNull, AEDesc, AEDisposeDesc,
        AESendMessage,
    };
    use objc2_foundation::{NSAppleEventDescriptor, NSRunLoop};
    use sk_app::embed::Builder;

    const CASE: &str = "SK_TEST_APPKIT_QUIT";
    const HOST_EXIT: i32 = 23;

    pub(super) fn run() {
        if let Ok(case) = env::var(CASE) {
            run_host(case);
            return;
        }
        for case in ["menu", "main-queue"]
            .into_iter()
            .chain(std::iter::repeat_n("dock", 6))
            .chain(["session"])
        {
            let profile = tempfile::tempdir().unwrap();
            eprintln!("Testing AppKit quit via {case}");
            let status = Command::new(env::current_exe().unwrap())
                .env(CASE, case)
                .env("SK_DATA_DIR", profile.path())
                .env("SK_DIST", profile.path())
                .env("SK_SHUTDOWN_DELAY_MS", "20")
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(HOST_EXIT), "AppKit quit via {case}");
        }
    }

    fn run_host(case: String) {
        thread::spawn(|| {
            thread::sleep(Duration::from_secs(30));
            eprintln!("AppKit did not hand control back to the host");
            std::process::exit(1);
        });
        let builder = Builder::new();
        #[cfg(feature = "prod")]
        let builder = {
            use sk_payload::{kind, name, write_unaligned};
            builder.payload_bytes(write_unaligned(&[
                (kind::META, name::META, br#"{"name":"appkit-quit-test"}"#),
                (kind::SCRIPT, name::SCRIPT, b"void 0;"),
            ]))
        };
        builder
            .activate(false)
            .visible(false)
            .setup(move |_| {
                match case.as_str() {
                    // SAFETY: the block captures nothing and AppKit runs it on the main thread.
                    "menu" => unsafe {
                        NSRunLoop::mainRunLoop().performBlock(&RcBlock::new(quit_from_menu))
                    },
                    "main-queue" => DispatchQueue::main().exec_async(|| {
                        NSApplication::sharedApplication(MainThreadMarker::new().unwrap())
                            .terminate(None);
                    }),
                    "dock" | "session" => {
                        let session = case == "session";
                        // Deliver after launch, once the app can receive Apple Events.
                        unsafe {
                            NSRunLoop::mainRunLoop()
                                .performBlock(&RcBlock::new(move || send_quit_event(session)))
                        };
                    }
                    _ => unreachable!(),
                }
                Ok(())
            })
            .run_on_main_thread(|exit| {
                assert!(exit.clean, "{}", exit.summary);
                assert!(
                    exit.summary.contains("SK_SHUTDOWN_DELAY_MS"),
                    "{}",
                    exit.summary
                );
                HOST_EXIT
            })
            .unwrap();
    }

    fn quit_from_menu() {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        let menu = app
            .mainMenu()
            .unwrap()
            .itemAtIndex(0)
            .unwrap()
            .submenu()
            .unwrap();
        let quit = (0..menu.numberOfItems())
            .find(|&i| menu.itemAtIndex(i).unwrap().action() == Some(sel!(terminate:)))
            .expect("Quit item in application menu");
        menu.performActionForItemAtIndex(quit);
    }

    fn send_quit_event(session: bool) {
        let event = NSAppleEventDescriptor::appleEventWithEventClass_eventID_targetDescriptor_returnID_transactionID(
            kCoreEventClass, kAEQuitApplication, Some(&NSAppleEventDescriptor::descriptorWithProcessIdentifier(std::process::id() as i32)),
            kAutoGenerateReturnID as i16, kAnyTransactionID,
        );
        if session {
            event.setAttributeDescriptor_forKeyword(
                &NSAppleEventDescriptor::descriptorWithEnumCode(kAEReallyLogOut),
                kAEQuitReason,
            );
        }
        // SAFETY: the descriptor owns its event and the reply slot starts empty.
        let mut reply = AEDesc {
            descriptorType: typeNull,
            dataHandle: std::ptr::null_mut(),
        };
        let status = unsafe {
            let status = AESendMessage(
                event.aeDesc(),
                &mut reply,
                kAENoReply as i32,
                kAEDefaultTimeout.into(),
            );
            AEDisposeDesc(&mut reply);
            status
        };
        assert_eq!(status, 0, "sending quit Apple Event to the test process");
    }
}
