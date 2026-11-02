//! The HTTP client gpui's loaders fetch through: redjs's network stack, the one
//! `fetch` and `XMLHttpRequest` already use.
//!
//! # Why not reqwest
//!
//! Until m26 the host installed the fork's `ReqwestClient`, so a page had two
//! network stacks. `fetch` and XHR went through redjs (`webapi/http.rs`: the
//! pooled HTTP/2 + HTTP/1.1 keep-alive transport, the cookie jar, devtools'
//! Network panel). Everything that loads by URL outside JS went through reqwest:
//! `<img>` and CSS `url()` (gpui's `ImageAssetLoader`), video posters (an
//! `<img>`), `@font-face` (`vn_tree::fonts`), `<lottie>` and `<vn-sticker>`. That
//! is a second connection pool to the same CDN, a second TLS stack (rustls
//! beside schannel), a runtime of its own, and loads devtools never saw.
//! A browser has one stack and one HTTP cache under all of them, and the cache
//! is what this unification is for (research/reports/m26/fixes-lag-1.md §4.12).
//!
//! # What goes where
//!
//! A `GET` or `HEAD` with no body that follows redirects is what every loader
//! sends, and it goes to [`redjs::webapi::http::start`]: the pool decides the
//! connection, follows the redirects hop by hop and decodes the body. Anything
//! else (a body, a request that must not follow a redirect) goes to the
//! `ReqwestClient` this module used to be, built on first use so a session that
//! never needs it starts no runtime for it. No caller sends one today.
//!
//! # Divergences, named
//!
//! - **No cookies on image loads.** A browser's `<img>` without `crossorigin` is
//!   a `no-cors` request in credentials mode `include`: the jar's cookies go
//!   with it, subject to `SameSite`. The loaders here cannot tell which element
//!   asked, or what its `crossorigin` says, and reqwest sent no cookies, so
//!   these requests are made in mode `omit`, as before (docs/RUNTIME.md §5.26,
//!   IC-1).
//! - **`<video>` bytes are not here.** Media Foundation fetches an http(s)
//!   `<video src>` through its own network source, with range requests; only the
//!   poster, an `<img>`, comes through this client.
//! - The user agent is the one `fetch` sends (`navigator.userAgent`), no longer
//!   `vue-native/<version>`: the same server sees one client.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui::http_client::{
    anyhow, http, AsyncBody, HttpClient, Inner, RedirectPolicy, RequestTimeout, Result as AnyResult, Url,
};
use vn_runtime::redjs::webapi::{cookies::Credentials, http as net};

/// A dead host must fail fast; `fetch` uses the same split (`fetch.rs`).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// A STALL budget (reset by every byte), not a total: a slow but progressing
/// download is never cut off.
const IO_TIMEOUT: Duration = Duration::from_secs(60);

pub struct RedjsHttpClient {
    /// Built on first use (see the module header).
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

/// Aborts the request if the loader drops its future before the answer: a
/// detached `<img>` whose load gpui gave up on does not keep a connection busy.
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

/// Is this a request redjs's transport carries as is: no body, redirects followed?
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
        };
        let cancel = net::Cancel::new();
        let (tx, rx) = oneshot::channel();
        let started = net::start(
            request,
            (*cancel).clone(),
            Box::new(move |result| {
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
