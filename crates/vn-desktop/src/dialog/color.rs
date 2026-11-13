//! HTML simple-color picker, sharing the file dialogs' owner and worker rules.
use super::*;

pub fn normalize(value: &str) -> Option<String> {
    (value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
    .then(|| value.to_ascii_lowercase())
}

pub fn start(value: &str) -> Result<u32, Fail> {
    start_owned(RESOLVER.get().and_then(|resolve| resolve()), value)
}

fn start_owned(owner: Option<Owner>, value: &str) -> Result<u32, Fail> {
    let value = normalize(value).unwrap_or_else(|| "#000000".into());
    let key = owner.map_or(0, |owner| owner.key);
    let hwnd = owner.map_or(0, |owner| owner.hwnd);
    let state = owner_state(key);
    let token = crate::next_token();
    if state.closed.load(Ordering::Acquire) {
        crate::settle(token, Ok(cancelled()));
        return Ok(token);
    }
    if state.active.swap(true, Ordering::AcqRel) {
        return Err(Fail::new("NotAllowedError", "Color picker already active.")
            .code("EBUSY")
            );
    }
    let cancel = Cancellation(state.clone());
    let worker = std::thread::Builder::new()
        .name("vn-color-picker".into())
        .spawn(move || {
            let picked = if scripted_mode() {
                scripted::color(&value, key, &cancel)
            } else {
                imp::color(&value, hwnd, &cancel)
            };
            state.active.store(false, Ordering::Release);
            let result = match picked {
                Ok(Some(value)) if !cancel.is_requested() => {
                    Ok(Done::json(&json!({ "color": value }).to_string()))
                }
                Ok(_) => Ok(Done::json(&cancelled())),
                Err(error) => Err(error),
            };
            crate::settle_done(token, result);
        });
    if worker.is_err() {
        owner_state(key).active.store(false, Ordering::Release);
        return Err(Fail::new(
            "InvalidStateError",
            "Could not start the color picker thread.",
        )
        .code("EIO")
        );
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn colors_are_six_digit_srgb_without_alpha_or_css_names() {
        assert_eq!(normalize("#Ab12EF").as_deref(), Some("#ab12ef"));
        for invalid in [
            "red",
            "#abc",
            "#12345678",
            "#12gg00",
            " #123456",
            "#123456 ",
        ] {
            assert!(normalize(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn scripted_picker_uses_owner_lifecycle_and_settles_selection_or_cancel() {
        let key = 0x434f4c4f52;
        let owner = Some(Owner { key, hwnd: 0 });
        let start_token = |reply: Result<u32, Fail>| reply.unwrap();
        let pending = || {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(id) = scripted::list()
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["window"] == key)
                    .and_then(|entry| entry["id"].as_u64())
                {
                    break id;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let settled = |token| {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(result) = crate::take_settled(&[token]).remove(&token) {
                    break result;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let token = start_token(start_owned(owner, "#aabbcc"));
        let id = pending();
        assert_eq!(start_owned(owner, "#ff0000").unwrap_err().code, Some("EBUSY"));
        assert!(scripted::choose(id, "red").is_err());
        scripted::choose(id, "#12ABEF").unwrap();
        let (ok, result) = settled(token);
        assert!(ok);
        assert_eq!(
            serde_json::from_str::<Json>(&result).unwrap()["color"],
            "#12abef"
        );
        let token = start_token(start_owned(owner, "#aabbcc"));
        scripted::cancel(pending()).unwrap();
        assert_eq!(
            serde_json::from_str::<Json>(&settled(token).1).unwrap()["cancelled"],
            true
        );
        let token = start_token(start_owned(owner, "#aabbcc"));
        pending();
        assert!(owner_closing(key));
        assert_eq!(
            serde_json::from_str::<Json>(&settled(token).1).unwrap()["cancelled"],
            true
        );
        assert!(is_idle(key));
    }
}
