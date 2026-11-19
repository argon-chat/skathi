use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use sk_tree::surface::{surface, SurfaceError};

#[test]
fn a_producer_racing_app_close_gets_one_event_for_every_accepted_frame() {
    let surface_handle = surface("closing");
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let _watch = surface_handle.on_frame(move |event| sink.lock().unwrap().push(event.frame()));
    let (started, producing) = mpsc::sync_channel(0);
    let producer = std::thread::spawn(move || {
        let mut accepted = Vec::new();
        loop {
            match surface_handle.present_bgra(&[0; 4], 1, 1, 4) {
                Ok(frame) => {
                    accepted.push(frame);
                    if accepted.len() == 1 {
                        started.send(()).unwrap();
                    }
                }
                Err(SurfaceError::Closed) => break,
                Err(error) => panic!("{error}"),
            }
        }
        accepted
    });
    producing.recv_timeout(Duration::from_secs(5)).unwrap();
    sk_tree::surface_app_closed();
    let accepted = producer.join().unwrap();
    assert!(!accepted.is_empty());
    assert_eq!(*events.lock().unwrap(), accepted);
    for name in ["closing", "created after quit"] {
        let late = surface(name);
        let before = late.stats().presented;
        assert_eq!(
            late.present_bgra(&[0; 4], 1, 1, 4),
            Err(SurfaceError::Closed)
        );
        assert_eq!(late.stats().presented, before);
    }
}
