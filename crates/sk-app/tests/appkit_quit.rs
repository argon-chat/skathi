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
        kAEQuitApplication, kAEQuitReason, kAEReallyLogOut, kAnyTransactionID,
        kAutoGenerateReturnID, kCoreEventClass,
    };
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventSendOptions, NSRunLoop};
    use sk_app::embed::Builder;

    const CASE: &str = "SK_TEST_APPKIT_QUIT";
    const HOST_EXIT: i32 = 23;
    pub(super) fn run() {
        if let Ok(case) = env::var(CASE) {
            run_host(case);
            return;
        }
        for case in ["menu", "main-queue", "session"]
            .into_iter()
            .chain(std::iter::repeat_n("dock", 6))
        {
            let profile = tempfile::tempdir().unwrap();
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
        Builder::new()
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
                        thread::spawn(move || send_quit_event(session));
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
            kCoreEventClass, kAEQuitApplication, Some(&NSAppleEventDescriptor::currentProcessDescriptor()),
            kAutoGenerateReturnID as i16, kAnyTransactionID,
        );
        if session {
            event.setAttributeDescriptor_forKeyword(
                &NSAppleEventDescriptor::descriptorWithEnumCode(kAEReallyLogOut),
                kAEQuitReason,
            );
        }
        let _ =
            event.sendEventWithOptions_timeout_error(NSAppleEventSendOptions::WaitForReply, 10.);
    }
}
