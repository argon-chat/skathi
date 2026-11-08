//! Authenticated inspector reads go directly to the live IDB server. This
//! module runs off the VM thread and never opens/upgrades a JS database.
use crate::json::Json;

#[cfg(feature = "indexeddb")]
mod enabled {
    use super::*;
    use vn_runtime::redjs::webapi::indexeddb::{
        inspection::{self, Preview, Query, Response},
        keypath::KeyPath,
    };
    fn text(request: &Json, key: &str, required: bool) -> Result<String, String> {
        let value = request.get(key).and_then(Json::as_str).unwrap_or("");
        if required && value.is_empty() {
            return Err(format!("indexeddb requires {key}"));
        }
        if value.len() > 4096 {
            return Err(format!("indexeddb {key} exceeds 4096 bytes"));
        }
        Ok(value.to_owned())
    }
    fn path(path: &KeyPath) -> Json {
        match path {
            KeyPath::String(path) => Json::s(String::from_utf16_lossy(path)),
            KeyPath::List(paths) => Json::arr(
                paths
                    .iter()
                    .take(128)
                    .map(|path| Json::s(String::from_utf16_lossy(path)))
                    .collect(),
            ),
        }
    }
    fn preview(value: Preview) -> Json {
        let mut fields = vec![
            ("type", Json::s(value.kind)),
            ("preview", Json::s(value.preview)),
            ("truncated", Json::Bool(value.truncated)),
        ];
        if !value.children.is_empty() {
            fields.push((
                "children",
                Json::arr(
                    value
                        .children
                        .into_iter()
                        .map(|child| {
                            Json::obj(vec![
                                ("name", Json::s(child.name)),
                                ("value", preview(child.value)),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        if let Some(size) = value.size {
            fields.push(("size", Json::n(size as f64)));
        }
        if let Some(mime) = value.mime {
            fields.push(("mime", Json::s(mime)));
        }
        if let Some(name) = value.name {
            fields.push(("name", Json::s(name)));
        }
        if let Some(reference) = value.reference {
            fields.push(("ref", Json::n(reference as f64)));
        }
        if let Some(id) = value.object_id {
            fields.push(("objectId", Json::n(id as f64)));
        }
        Json::obj(fields)
    }
    pub(super) fn command(request: &Json) -> Result<Json, String> {
        let operation = request.get("op").and_then(Json::as_str).unwrap_or("list");
        if operation == "list" && !vn_runtime::redjs::webapi::indexeddb::server::started() {
            return Ok(Json::obj(vec![
                ("available", Json::Bool(false)),
                (
                    "backend",
                    Json::s("IndexedDB has not been initialized by the application"),
                ),
                ("databases", Json::arr(Vec::new())),
                ("truncated", Json::Bool(false)),
            ]));
        }
        let query = match operation {
            "list" => Query::List,
            "schema" => Query::Schema {
                database: text(request, "database", true)?.encode_utf16().collect(),
            },
            "records" => {
                let limit = request.get("limit").and_then(Json::as_f64).unwrap_or(50.0);
                if !limit.is_finite() || limit.fract() != 0.0 || !(1.0..=100.0).contains(&limit) {
                    return Err("indexeddb limit must be between 1 and 100".into());
                }
                let search = text(request, "search", false)?;
                if search.chars().count() > 256 {
                    return Err("indexeddb search exceeds 256 characters".into());
                }
                let after = request
                    .get("after")
                    .and_then(Json::as_str)
                    .filter(|value| !value.is_empty())
                    .map(inspection::unhex)
                    .transpose()?;
                Query::Records {
                    database: text(request, "database", true)?.encode_utf16().collect(),
                    store: text(request, "store", true)?,
                    index: Some(text(request, "index", false)?).filter(|value| !value.is_empty()),
                    after,
                    limit: limit as usize,
                    search,
                }
            }
            _ => return Err("indexeddb is read-only: op must be list, schema, or records".into()),
        };
        let response = inspection::inspect(query)?
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| {
                "IndexedDB inspection timed out; the application transaction is left untouched"
                    .to_owned()
            })??;
        Ok(match response {
            Response::List {
                available,
                backend,
                databases,
                truncated,
            } => Json::obj(vec![
                ("available", Json::Bool(available)),
                ("backend", Json::s(backend)),
                (
                    "databases",
                    Json::arr(
                        databases
                            .into_iter()
                            .map(|database| {
                                Json::obj(vec![
                                    ("name", Json::s(database.name)),
                                    ("version", Json::n(database.version as f64)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("truncated", Json::Bool(truncated)),
            ]),
            Response::Schema {
                database,
                version,
                stores,
                truncated,
            } => Json::obj(vec![
                ("available", Json::Bool(true)),
                ("database", Json::s(database)),
                ("version", Json::n(version as f64)),
                (
                    "stores",
                    Json::arr(
                        stores
                            .into_iter()
                            .map(|store| {
                                Json::obj(vec![
                                    ("name", Json::s(store.name)),
                                    (
                                        "keyPath",
                                        store.key_path.as_ref().map(path).unwrap_or(Json::Null),
                                    ),
                                    ("autoIncrement", Json::Bool(store.auto_increment)),
                                    ("count", Json::n(store.count as f64)),
                                    (
                                        "indexes",
                                        Json::arr(
                                            store
                                                .indexes
                                                .into_iter()
                                                .map(|index| {
                                                    Json::obj(vec![
                                                        ("name", Json::s(index.name)),
                                                        ("keyPath", path(&index.key_path)),
                                                        ("unique", Json::Bool(index.unique)),
                                                        (
                                                            "multiEntry",
                                                            Json::Bool(index.multi_entry),
                                                        ),
                                                        ("count", Json::n(index.count as f64)),
                                                    ])
                                                })
                                                .collect(),
                                        ),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("truncated", Json::Bool(truncated)),
            ]),
            Response::Records {
                records,
                next_cursor,
                has_more,
                scanned,
                truncated,
            } => Json::obj(vec![
                ("available", Json::Bool(true)),
                (
                    "records",
                    Json::arr(
                        records
                            .into_iter()
                            .map(|record| {
                                Json::obj(vec![
                                    ("key", preview(record.key)),
                                    ("primaryKey", preview(record.primary_key)),
                                    ("value", preview(record.value)),
                                    ("encodedBytes", Json::n(record.encoded_bytes as f64)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    "nextCursor",
                    next_cursor
                        .map(|cursor| Json::s(inspection::hex(&cursor)))
                        .unwrap_or(Json::Null),
                ),
                ("hasMore", Json::Bool(has_more)),
                ("scanned", Json::n(scanned as f64)),
                ("truncated", Json::Bool(truncated)),
                (
                    "searchScope",
                    Json::s(
                        "Key and bounded structured-clone preview; Blob bodies are not searched",
                    ),
                ),
            ]),
        })
    }
}

/// Does not access JSC; invoke on the authenticated listener thread.
pub(super) fn command(request: &Json) -> Result<Json, String> {
    #[cfg(feature = "indexeddb")]
    {
        enabled::command(request)
    }
    #[cfg(not(feature = "indexeddb"))]
    {
        let _ = request;
        Ok(Json::obj(vec![
            ("available", Json::Bool(false)),
            ("backend", Json::s("IndexedDB is not enabled in this build")),
            ("databases", Json::arr(Vec::new())),
        ]))
    }
}
