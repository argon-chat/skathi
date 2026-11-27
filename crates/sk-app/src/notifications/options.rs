use super::{image::ImageInput, native_id, Action, Group, Notice, Request};
use crate::js::{self, Context, Value};
use crate::tray::Roots;

fn invalid(cx: &Context, message: &str) -> js::Error {
    js::Error::Exception(cx.error(1, message))
}

fn text(cx: &Context, value: Value, field: &str) -> js::Result<String> {
    if cx.kind(value) != 5 {
        return Err(invalid(
            cx,
            &format!("Notification {field} must be a string"),
        ));
    }
    let s = cx.read_string(value)?;
    if s.contains('\0') || (field != "body" && s.trim().is_empty()) {
        return Err(invalid(cx, &format!("Invalid notification {field}")));
    }
    Ok(s)
}

struct Reader<'a> {
    cx: &'a Context,
    roots: Roots<'a>,
}

impl Reader<'_> {
    fn object(&self, value: Value, field: &str) -> js::Result<()> {
        if self.cx.kind(value) != 8 {
            return Err(invalid(
                self.cx,
                &format!("Notification {field} must be an object"),
            ));
        }
        Ok(())
    }
    fn get(&mut self, obj: Value, field: &str) -> js::Result<Value> {
        Ok(self.roots.keep(self.cx.get(obj, &self.cx.atom(field))?))
    }
    fn required(&mut self, obj: Value, field: &str) -> js::Result<String> {
        let v = self.get(obj, field)?;
        text(self.cx, v, field)
    }
    fn optional(&mut self, obj: Value, field: &str) -> js::Result<Option<String>> {
        let v = self.get(obj, field)?;
        if v.is_undefined() {
            Ok(None)
        } else {
            text(self.cx, v, field).map(Some)
        }
    }
    fn actions(&mut self, obj: Value) -> js::Result<Vec<Action>> {
        let value = self.get(obj, "actions")?;
        if value.is_undefined() {
            return Ok(Vec::new());
        }
        if self.cx.kind(value) != 10 || self.cx.array_length(value)? > 2 {
            return Err(invalid(
                self.cx,
                "Expected an array of at most two notification actions",
            ));
        }
        let mut actions = Vec::new();
        let mut previous = None;
        let mut reply = false;
        for index in 0..self.cx.array_length(value)? {
            let action = self.roots.keep(self.cx.get_index(value, index)?);
            self.object(action, "action")?;
            let id = self.required(action, "id")?;
            let label = self.required(action, "label")?;
            if previous.as_ref() == Some(&id) {
                return Err(invalid(self.cx, "Duplicate notification action ID"));
            }
            previous = Some(id.clone());
            match self.required(action, "kind")?.as_str() {
                "button" => actions.push(Action {
                    id,
                    label,
                    reply: false,
                }),
                "reply" if !reply => {
                    reply = true;
                    actions.push(Action {
                        id,
                        label,
                        reply: true,
                    });
                }
                _ => {
                    return Err(invalid(
                        self.cx,
                        "Expected buttons and at most one reply action",
                    ))
                }
            }
        }
        Ok(actions)
    }
    fn image(&mut self, value: Value) -> js::Result<ImageInput> {
        self.object(value, "image")?;
        let asset = self.optional(value, "asset")?;
        let png = self.get(value, "png")?;
        match (asset, png.is_undefined()) {
            (Some(path), true) if sk_payload::content_key(&path).is_some() => {
                Ok(ImageInput::Asset(path))
            }
            (None, false) => {
                let info = self
                    .cx
                    .typedarray_info(png)
                    .map_err(|_| invalid(self.cx, "Expected PNG bytes in a Uint8Array"))?;
                if info.kind == 2
                    && info.detached == 0
                    && info.shared == 0
                    && !info.data.is_null()
                    && (8..=2 * 1024 * 1024).contains(&info.byte_length)
                {
                    // The rooted non-shared view stays attached during this copy, with no JS calls.
                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            info.data.cast::<u8>(),
                            info.byte_length as usize,
                        )
                    };
                    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                        return Ok(ImageInput::Png(bytes.to_vec()));
                    }
                }
                Err(invalid(self.cx, "Expected PNG bytes (maximum 2 MiB)"))
            }
            _ => Err(invalid(
                self.cx,
                "Provide one packaged image asset or PNG byte array",
            )),
        }
    }
}

pub(super) fn show(cx: &Context, obj: Value) -> js::Result<Request> {
    let mut read = Reader {
        cx,
        roots: Roots::new(cx),
    };
    read.object(obj, "options")?;
    let title = read.required(obj, "title")?;
    let body = read.optional(obj, "body")?;
    let id = read
        .optional(obj, "id")?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let silent = match read.optional(obj, "sound")?.as_deref() {
        None | Some("default") => false,
        Some("silent") => true,
        _ => return Err(invalid(cx, "Expected default or silent notification sound")),
    };
    let actions = read.actions(obj)?;
    let group_value = read.get(obj, "group")?;
    let group = if group_value.is_undefined() {
        None
    } else {
        read.object(group_value, "group")?;
        Some(Group {
            id: native_id(&read.required(group_value, "id")?),
            label: read.required(group_value, "label")?,
        })
    };
    let value = read.get(obj, "image")?;
    let image = if value.is_undefined() {
        None
    } else {
        Some(read.image(value)?)
    };
    Ok(Request::Show(
        Notice {
            native_id: native_id(&id),
            id,
            title,
            body,
            actions,
            silent,
            group,
            image: None,
        },
        image,
    ))
}

pub(super) fn close(cx: &Context, arg: Value) -> js::Result<Request> {
    Ok(Request::Close(native_id(&text(cx, arg, "id")?)))
}
