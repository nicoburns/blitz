//! Networking (HTTP, filesystem, Data URIs) for Blitz
//!
//! Provides an implementation of the [`blitz_traits::net::NetProvider`] trait.

use blitz_traits::net::{
    AbortSignal, Body, Bytes, HeaderMap, NetHandler, NetProvider, NetWaker, Request,
};
use data_url::DataUrl;
use std::{
    collections::HashMap,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Poll,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(feature = "cache")]
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:60.0) Gecko/20100101 Firefox/81.0";

/// Matches real browsers' per-origin cap of 6.
const PER_HOST_MAX_CONCURRENT: usize = 6;

type HostLimits = Arc<Mutex<HashMap<String, Arc<Semaphore>>>>;

#[cfg(feature = "cache")]
type Client = reqwest_middleware::ClientWithMiddleware;
#[cfg(not(feature = "cache"))]
type Client = reqwest::Client;

#[cfg(feature = "cache")]
fn get_cache_path() -> std::path::PathBuf {
    use directories::ProjectDirs;
    let path = ProjectDirs::from("com", "DioxusLabs", "Blitz")
        .expect("Failed to find cache directory")
        .cache_dir()
        .to_owned();
    #[cfg(feature = "tracing")]
    tracing::info!(path = ?path.display(), "Using cache dir");
    path
}

#[cfg(target_arch = "wasm32")]
fn spawn(fut: impl Future + 'static) {
    wasm_bindgen_futures::spawn_local(async move {
        fut.await;
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn<F>(fut: F)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(fut);
}

pub struct Provider {
    client: Client,
    /// A cache-free reqwest client used by [`Provider::send`]. The cache
    /// middleware's `CACacheManager` buffers the entire response body before
    /// returning, which defeats the header-first two-phase fetch used to detect
    /// downloads (and to show them as in-progress). `send` therefore bypasses
    /// the cache; sub-resource fetches still go through the caching `client`.
    direct_client: reqwest::Client,
    waker: Arc<dyn NetWaker>,
    per_host_limits: HostLimits,
    #[cfg(feature = "cache")]
    cache_manager: CACacheManager,
}
impl Provider {
    pub fn new(waker: Option<Arc<dyn NetWaker>>) -> Self {
        let builder = reqwest::Client::builder();
        #[cfg(feature = "cookies")]
        let builder = builder.cookie_store(true);
        let base_client = builder.build().unwrap();
        let direct_client = base_client.clone();

        #[cfg(feature = "cache")]
        let cache_manager = CACacheManager::new(get_cache_path(), true);

        #[cfg(feature = "cache")]
        let client = reqwest_middleware::ClientBuilder::new(base_client)
            .with(Cache(HttpCache {
                mode: CacheMode::Default,
                manager: cache_manager.clone(),
                options: HttpCacheOptions::default(),
            }))
            .build();
        #[cfg(not(feature = "cache"))]
        let client = base_client;

        let waker = waker.unwrap_or(Arc::new(DummyNetWaker));
        Self {
            client,
            direct_client,
            waker,
            per_host_limits: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "cache")]
            cache_manager,
        }
    }
    pub fn shared(waker: Option<Arc<dyn NetWaker>>) -> Arc<dyn NetProvider> {
        Arc::new(Self::new(waker))
    }
    pub fn is_empty(&self) -> bool {
        Arc::strong_count(&self.waker) == 1
    }
    pub fn count(&self) -> usize {
        Arc::strong_count(&self.waker) - 1
    }

    #[cfg(feature = "cache")]
    pub async fn clear_cache(&self) {
        if let Err(e) = self.cache_manager.clear().await {
            #[cfg(feature = "tracing")]
            tracing::error!("Failed to clear HTTP cache: {:?}", e);
            #[cfg(not(feature = "tracing"))]
            let _ = e;
        }
    }
}
/// A response whose status and headers have been received but whose body has
/// not yet been read. This lets callers inspect the headers (e.g. to detect a
/// `Content-Disposition: attachment` download) before committing to buffering
/// the whole body.
pub struct ResponseHead {
    url: String,
    headers: HeaderMap,
    body: ResponseBody,
}

enum ResponseBody {
    /// Body already resolved in memory (`data:` / `file:` URLs).
    Buffered(Bytes),
    /// Streamed HTTP body. The permit bounds per-host concurrency and is held
    /// until the body is consumed (or the head is dropped).
    Http {
        response: reqwest::Response,
        _permit: OwnedSemaphorePermit,
    },
}

impl ResponseHead {
    /// The final URL after any redirects.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Consume the head and read the full response body into memory.
    pub async fn bytes(self) -> Result<Bytes, ProviderError> {
        match self.body {
            ResponseBody::Buffered(bytes) => Ok(bytes),
            ResponseBody::Http { response, .. } => Ok(response.bytes().await?),
        }
    }
}

/// Acquire a per-host permit, bounding the number of concurrent in-flight
/// requests per origin. The returned permit must be held for the duration of
/// the request (including reading the body).
async fn acquire_host_permit(
    request: &Request,
    per_host_limits: &HostLimits,
) -> OwnedSemaphorePermit {
    let host_key = request
        .url
        .host_str()
        .map(str::to_owned)
        .unwrap_or_default();
    let semaphore = {
        let mut map = per_host_limits.lock().unwrap();
        map.entry(host_key)
            .or_insert_with(|| Arc::new(Semaphore::new(PER_HOST_MAX_CONCURRENT)))
            .clone()
    };
    semaphore
        .acquire_owned()
        .await
        .expect("per-host semaphore was closed")
}

impl Provider {
    /// Buffered fetch used for sub-resources (images, stylesheets, favicons,
    /// …). Goes through the caching `client`.
    async fn fetch_inner(
        client: Client,
        request: Request,
        per_host_limits: HostLimits,
    ) -> Result<(String, HeaderMap, Bytes), ProviderError> {
        match request.url.scheme() {
            "data" => {
                let data_url = DataUrl::process(request.url.as_str())?;
                let decoded = data_url.decode_to_vec()?;
                Ok((
                    request.url.to_string(),
                    HeaderMap::new(),
                    Bytes::from(decoded.0),
                ))
            }
            "file" => {
                let file_content = std::fs::read(request.url.path())?;
                Ok((
                    request.url.to_string(),
                    HeaderMap::new(),
                    Bytes::from(file_content),
                ))
            }
            _ => Self::fetch_http(client, request, per_host_limits).await,
        }
    }

    async fn fetch_http(
        client: Client,
        request: Request,
        per_host_limits: HostLimits,
    ) -> Result<(String, HeaderMap, Bytes), ProviderError> {
        let _permit = acquire_host_permit(&request, &per_host_limits).await;

        let mut req = client
            .request(request.method, request.url)
            .headers(request.headers)
            .header("User-Agent", USER_AGENT);

        if let Some(content_type) = request.content_type.as_ref() {
            req = req.header("Content-Type", content_type);
        }

        let req = req
            .apply_body(request.body, request.content_type.as_deref())
            .await;
        let response = req.send().await?;
        let status = response.status();
        let final_url = response.url().to_string();

        if status.is_success() {
            let headers = response.headers().clone();
            let bytes = response.bytes().await?;
            return Ok((final_url, headers, bytes));
        }

        #[cfg(feature = "tracing")]
        tracing::warn!(
            url = final_url.as_str(),
            status = status.as_u16(),
            "HTTP error status"
        );
        Err(ProviderError::HttpStatus {
            status,
            url: final_url,
        })
    }

    /// Header-first send used for top-level navigations and downloads. Uses the
    /// cache-free `direct_client` so it returns as soon as headers arrive and
    /// streams the body via [`ResponseHead::bytes`].
    async fn send_inner(
        client: reqwest::Client,
        request: Request,
        per_host_limits: HostLimits,
    ) -> Result<ResponseHead, ProviderError> {
        match request.url.scheme() {
            "data" => {
                let data_url = DataUrl::process(request.url.as_str())?;
                let decoded = data_url.decode_to_vec()?;
                Ok(ResponseHead {
                    url: request.url.to_string(),
                    headers: HeaderMap::new(),
                    body: ResponseBody::Buffered(Bytes::from(decoded.0)),
                })
            }
            "file" => {
                let file_content = std::fs::read(request.url.path())?;
                Ok(ResponseHead {
                    url: request.url.to_string(),
                    headers: HeaderMap::new(),
                    body: ResponseBody::Buffered(Bytes::from(file_content)),
                })
            }
            _ => Self::send_http(client, request, per_host_limits).await,
        }
    }

    async fn send_http(
        client: reqwest::Client,
        request: Request,
        per_host_limits: HostLimits,
    ) -> Result<ResponseHead, ProviderError> {
        // Hold an owned permit until the streamed body is consumed (or the head
        // is dropped) to keep per-host concurrency bounded.
        let permit = acquire_host_permit(&request, &per_host_limits).await;

        let mut req = client
            .request(request.method, request.url)
            .headers(request.headers)
            .header("User-Agent", USER_AGENT);

        if let Some(content_type) = request.content_type.as_ref() {
            req = req.header("Content-Type", content_type);
        }

        let req = req
            .apply_body(request.body, request.content_type.as_deref())
            .await;
        let response = req.send().await?;
        let status = response.status();
        let final_url = response.url().to_string();

        if status.is_success() {
            let headers = response.headers().clone();
            return Ok(ResponseHead {
                url: final_url,
                headers,
                body: ResponseBody::Http {
                    response,
                    _permit: permit,
                },
            });
        }

        #[cfg(feature = "tracing")]
        tracing::warn!(
            url = final_url.as_str(),
            status = status.as_u16(),
            "HTTP error status"
        );
        Err(ProviderError::HttpStatus {
            status,
            url: final_url,
        })
    }

    #[allow(clippy::type_complexity)]
    pub fn fetch_with_callback(
        &self,
        request: Request,
        callback: Box<dyn FnOnce(Result<(String, Bytes), ProviderError>) + Send + Sync + 'static>,
    ) {
        #[cfg(feature = "tracing")]
        let url = request.url.to_string();

        let client = self.client.clone();
        let per_host_limits = self.per_host_limits.clone();
        spawn(async move {
            let result = Self::fetch_inner(client, request, per_host_limits)
                .await
                .map(|(url, _headers, bytes)| (url, bytes));

            #[cfg(feature = "tracing")]
            if let Err(e) = &result {
                #[cfg(feature = "tracing")]
                tracing::error!(url = url.as_str(), error = ?e, "Fetching");
            } else {
                #[cfg(feature = "tracing")]
                tracing::info!(url = url.as_str(), "Success fetching");
            }

            callback(result);
        });
    }

    pub async fn fetch_async(&self, request: Request) -> Result<(String, Bytes), ProviderError> {
        self.fetch_async_with_headers(request)
            .await
            .map(|(url, _headers, bytes)| (url, bytes))
    }

    /// Send a request and return once the status and headers are available,
    /// without reading the body. Use [`ResponseHead::bytes`] to read the body
    /// afterwards. This is useful for detecting downloads from response headers
    /// before buffering a potentially large body.
    pub async fn send(&self, request: Request) -> Result<ResponseHead, ProviderError> {
        let client = self.direct_client.clone();
        let per_host_limits = self.per_host_limits.clone();
        Self::send_inner(client, request, per_host_limits).await
    }

    /// Like [`fetch_async`](Self::fetch_async) but additionally returns the
    /// response headers. Non-HTTP schemes (`data:`, `file:`) yield an empty
    /// [`HeaderMap`].
    pub async fn fetch_async_with_headers(
        &self,
        request: Request,
    ) -> Result<(String, HeaderMap, Bytes), ProviderError> {
        #[cfg(feature = "tracing")]
        let url = request.url.to_string();

        let client = self.client.clone();
        let per_host_limits = self.per_host_limits.clone();
        let result = Self::fetch_inner(client, request, per_host_limits).await;

        #[cfg(feature = "tracing")]
        if let Err(e) = &result {
            #[cfg(feature = "tracing")]
            tracing::error!(url = url.as_str(), error = ?e, "Fetching");
        } else {
            #[cfg(feature = "tracing")]
            tracing::info!(url = url.as_str(), "Success fetching");
        }

        result
    }
}

impl NetProvider for Provider {
    fn fetch(&self, doc_id: usize, mut request: Request, handler: Box<dyn NetHandler>) {
        let client = self.client.clone();
        let per_host_limits = self.per_host_limits.clone();

        #[cfg(feature = "tracing")]
        tracing::info!(url = request.url.as_str(), "Fetching");

        let waker = self.waker.clone();
        spawn(async move {
            #[cfg(feature = "tracing")]
            let url = request.url.to_string();

            let signal = request.signal.take();
            let result = if let Some(signal) = signal {
                AbortFetch::new(
                    signal,
                    Box::pin(
                        async move { Self::fetch_inner(client, request, per_host_limits).await },
                    ),
                )
                .await
            } else {
                Self::fetch_inner(client, request, per_host_limits).await
            };

            waker.wake(doc_id);

            match result {
                Ok((response_url, _headers, bytes)) => {
                    handler.bytes(response_url, bytes);
                    #[cfg(feature = "tracing")]
                    tracing::info!(url = url.as_str(), "Success fetching");
                }
                Err(e) => {
                    #[cfg(feature = "tracing")]
                    tracing::error!(url = url.as_str(), error = ?e, "Error fetching");
                    #[cfg(not(feature = "tracing"))]
                    let _ = e;
                }
            };
        });
    }
}

struct AbortFetch<F, T> {
    signal: AbortSignal,
    future: F,
    _rt: PhantomData<T>,
}

impl<F, T> AbortFetch<F, T> {
    fn new(signal: AbortSignal, future: F) -> Self {
        Self {
            signal,
            future,
            _rt: PhantomData,
        }
    }
}

impl<F, T> Future for AbortFetch<F, T>
where
    F: Future + Unpin + 'static,
    F::Output: Into<Result<T, ProviderError>> + 'static,
    T: Unpin,
{
    type Output = Result<T, ProviderError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.signal.aborted() {
            return Poll::Ready(Err(ProviderError::Abort));
        }

        match Pin::new(&mut self.future).poll(cx) {
            Poll::Ready(output) => Poll::Ready(output.into()),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug)]
pub enum ProviderError {
    Abort,
    Io(std::io::Error),
    DataUrl(data_url::DataUrlError),
    DataUrlBase64(data_url::forgiving_base64::InvalidBase64),
    ReqwestError(reqwest::Error),
    #[cfg(feature = "cache")]
    ReqwestMiddlewareError(reqwest_middleware::Error),
    HttpStatus {
        status: reqwest::StatusCode,
        url: String,
    },
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Abort => write!(f, "request aborted"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::DataUrl(e) => write!(f, "data url error: {e:?}"),
            Self::DataUrlBase64(e) => write!(f, "data url base64 error: {e:?}"),
            Self::ReqwestError(e) => write!(f, "reqwest error: {e}"),
            #[cfg(feature = "cache")]
            Self::ReqwestMiddlewareError(e) => write!(f, "reqwest middleware error: {e}"),
            Self::HttpStatus { status, url } => write!(f, "HTTP {status} for {url}"),
        }
    }
}

impl From<std::io::Error> for ProviderError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<data_url::DataUrlError> for ProviderError {
    fn from(value: data_url::DataUrlError) -> Self {
        Self::DataUrl(value)
    }
}

impl From<data_url::forgiving_base64::InvalidBase64> for ProviderError {
    fn from(value: data_url::forgiving_base64::InvalidBase64) -> Self {
        Self::DataUrlBase64(value)
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(value: reqwest::Error) -> Self {
        Self::ReqwestError(value)
    }
}

#[cfg(feature = "cache")]
impl From<reqwest_middleware::Error> for ProviderError {
    fn from(value: reqwest_middleware::Error) -> Self {
        Self::ReqwestMiddlewareError(value)
    }
}

trait ReqwestExt {
    async fn apply_body(self, body: Body, content_type: Option<&str>) -> Self;
}

impl ReqwestExt for reqwest::RequestBuilder {
    async fn apply_body(self, body: Body, content_type: Option<&str>) -> Self {
        match body {
            Body::Bytes(bytes) => self.body(bytes),
            Body::Form(form_data) => match content_type {
                Some("application/x-www-form-urlencoded") => self.form(&form_data),
                #[cfg(feature = "multipart")]
                Some("multipart/form-data") => self.multipart(build_multipart_form(form_data).await),
                _ => self,
            },
            Body::Empty => self,
        }
    }
}

// With the cache feature the request builder is the middleware type; the send
// path still uses the raw reqwest builder above. Without the cache feature the
// alias is `reqwest::RequestBuilder`, so this impl would be a duplicate.
#[cfg(feature = "cache")]
impl ReqwestExt for reqwest_middleware::RequestBuilder {
    async fn apply_body(self, body: Body, content_type: Option<&str>) -> Self {
        match body {
            Body::Bytes(bytes) => self.body(bytes),
            Body::Form(form_data) => match content_type {
                Some("application/x-www-form-urlencoded") => self.form(&form_data),
                #[cfg(feature = "multipart")]
                Some("multipart/form-data") => self.multipart(build_multipart_form(form_data).await),
                _ => self,
            },
            Body::Empty => self,
        }
    }
}

#[cfg(feature = "multipart")]
async fn build_multipart_form(mut form_data: blitz_traits::net::FormData) -> reqwest::multipart::Form {
    use blitz_traits::net::{Entry, EntryValue};
    let mut form = reqwest::multipart::Form::new();
    for Entry { name, value } in form_data.0.drain(..) {
        form = match value {
            EntryValue::String(value) => form.text(name, value),
            EntryValue::File(path_buf) => form
                .file(name, path_buf)
                .await
                .expect("Couldn't read form file from disk"),
            EntryValue::EmptyFile => form.part(
                name,
                reqwest::multipart::Part::bytes(&[])
                    .mime_str("application/octet-stream")
                    .unwrap(),
            ),
        };
    }
    form
}

struct DummyNetWaker;
impl NetWaker for DummyNetWaker {
    fn wake(&self, _client_id: usize) {}
}
