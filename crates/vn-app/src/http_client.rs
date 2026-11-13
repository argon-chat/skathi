//! The HTTP client gpui's loaders fetch through: redjs's network stack, the one `fetch` and
//! `XMLHttpRequest` use.
//!
//! One stack rather than reqwest beside it: a single connection pool, TLS stack and HTTP cache under
//! every load (`<img>`, CSS `url()`, `@font-face`, `<lottie>`, `<vn-sticker>`, video posters), all
//! visible to devtools' Network panel.
//!
//! A `GET` or `HEAD` with no body that follows redirects (what every loader sends) goes to
//! [`redjs::webapi::http::start`]: the pool decides the connection, follows redirects hop by hop and
//! decodes the body. Anything else (a body, a request that must not follow a redirect) goes to a
//! `ReqwestClient`, built on first use so a session that never needs it starts no runtime for it.
//!
//! - **No cookies on image loads.** A browser's `<img>` without `crossorigin` is a `no-cors` request
//!   in credentials mode `include`. The loaders here cannot tell which element asked, so these
//!   requests are made in mode `omit`.
//! - **`<video>` bytes are not here.** Media Foundation fetches an http(s) `<video src>` through its
//!   own network source, with range requests; only the poster, an `<img>`, comes through this client.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Result as AnyResult};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui::http_client::{http, AsyncBody, HttpClient, Inner, RedirectPolicy, RequestTimeout, Url};
use vn_runtime::redjs::webapi::{cookies::Credentials, http as net};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// A STALL budget (reset by every byte), not a total: a slow but progressing
/// download is never cut off.
const IO_TIMEOUT: Duration = Duration::from_secs(60);

pub struct RedjsHttpClient {
    fallback: OnceLock<Option<Arc<dyn HttpClient>>>,
}

impl RedjsHttpClient {
    pub fn new() -> Self {
        RedjsHttpClient { fallback: OnceLock::new() }
    }

    fn fallback(&self) -> Option<Arc<dyn HttpClient>> {
        self.fallback
            .get_or_init(|| {
                match reqwest_client::ReqwestClient::user_agent(concat!("vue-native/", env!("CARGO_PKG_VERSION"))) {
                    Ok(client) => Some(Arc::new(client) as Arc<dyn HttpClient>),
                    Err(e) => {
                        log::warn!("could not build the fallback HTTP client ({e})");
                        None
                    }
                }
            })
            .clone()
    }
}

struct AbortOnDrop(Option<Arc<net::Cancel>>);

impl AbortOnDrop {
    fn done(&mut self) {
        self.0 = None;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(c) = self.0.take() {
            c.abort();
        }
    }
}

/// The HTTP cache's cap in MB: `VN_HTTP_CACHE` (`0` or `off` turns it off, a number is the cap), else
/// `vn.config.json`'s `"httpCache": { "maxMB": N }` (`0` or `false` turns it off), else 256.
#[cfg_attr(not(feature = "storages"), allow(dead_code))]
pub fn cache_mb(window_json: Option<&str>) -> u64 {
    if let Ok(v) = std::env::var("VN_HTTP_CACHE") {
        let v = v.trim();
        if v == "0" || v.eq_ignore_ascii_case("off") {
            return 0;
        }
        if let Ok(n) = v.parse::<u64>() {
            return n;
        }
        log::warn!("VN_HTTP_CACHE={v:?} is neither 0/off nor a size in MB; ignored");
    }
    config_mb(window_json).unwrap_or(DEFAULT_CACHE_MB)
}

#[cfg_attr(not(feature = "storages"), allow(dead_code))]
fn config_mb(window_json: Option<&str>) -> Option<u64> {
    window_json.and_then(crate::json::parse).and_then(|w| match w.get("httpCache")? {
        crate::json::Json::Bool(false) => Some(0),
        o => o.get("maxMB").and_then(crate::json::Json::as_f64).filter(|n| *n >= 0.0).map(|n| n as u64),
    })
}

/// The HTTP cache's cap when neither the environment nor the config names one. A fixed cap is easier
/// for an app to reason about than Chromium's free-disk sizing.
#[cfg_attr(not(feature = "storages"), allow(dead_code))]
const DEFAULT_CACHE_MB: u64 = 256;

fn plain_get(parts: &http::request::Parts, body: &AsyncBody) -> bool {
    let empty = match &body.0 {
        Inner::Empty => true,
        Inner::Bytes(b) => b.get_ref().is_empty(),
        Inner::AsyncReader(_) => false,
    };
    let follows = !matches!(parts.extensions.get::<RedirectPolicy>(), None | Some(RedirectPolicy::NoFollow));
    empty && follows && (parts.method == http::Method::GET || parts.method == http::Method::HEAD)
}

impl HttpClient for RedjsHttpClient {
    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(&self, req: http::Request<AsyncBody>) -> BoxFuture<'static, AnyResult<http::Response<AsyncBody>>> {
        let (parts, body) = req.into_parts();
        if !plain_get(&parts, &body) {
            return match self.fallback() {
                Some(client) => client.send(http::Request::from_parts(parts, body)),
                None => Box::pin(async { Err(anyhow!("no HTTP client for this request")) }),
            };
        }
        let headers = parts
            .headers
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let io_timeout = parts.extensions.get::<RequestTimeout>().map_or(IO_TIMEOUT, |t| t.0);
        let request = net::Request {
            method: parts.method.as_str().to_string(),
            url: parts.uri.to_string(),
            headers,
            body: None,
            redirect: net::Redirect::Follow,
            credentials: Credentials::Omit,
            connect_timeout: CONNECT_TIMEOUT,
            io_timeout,
            observer: None,
            cache: net::cache::CacheMode::Default,
        };
        let cancel = net::Cancel::new();
        let (tx, rx) = oneshot::channel();
        #[cfg(vn_devtools)]
        let row = crate::devtools::LoaderRow::start(&request.method, &request.url);
        let started = net::start(
            request,
            (*cancel).clone(),
            Box::new(move |result| {
                #[cfg(vn_devtools)]
                row.finish(&result);
                let _ = tx.send(result);
            }),
        );
        Box::pin(async move {
            let mut guard = AbortOnDrop(Some(cancel));
            started.map_err(|e| anyhow!("{}", e.message))?;
            let result = rx.await.map_err(|_| anyhow!("the request was dropped"))?;
            guard.done();
            let response = result.map_err(|e| anyhow!("{}", e.message))?;
            let mut builder = http::Response::builder().status(response.status);
            for (k, v) in &response.headers {
                // The jar was not asked (credentials `omit`); a loader has no use for them.
                if k.eq_ignore_ascii_case("set-cookie") {
                    continue;
                }
                builder = builder.header(k.as_str(), v.as_str());
            }
            Ok(builder.body(AsyncBody::from(response.body))?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::config_mb;

    #[test]
    fn the_config_names_a_cap_turns_the_cache_off_or_says_nothing() {
        let w = |s: &str| config_mb(Some(s));
        assert_eq!(w(r#"{"httpCache":{"maxMB":3}}"#), Some(3));
        assert_eq!(w(r#"{"httpCache":{"maxMB":0}}"#), Some(0));
        assert_eq!(w(r#"{"httpCache":false}"#), Some(0));
        assert_eq!(w(r#"{"httpCache":{"maxMB":-1}}"#), None);
        assert_eq!(w(r#"{"httpCache":true}"#), None);
        assert_eq!(w(r#"{"title":"x"}"#), None);
        assert_eq!(config_mb(None), None);
    }
}
