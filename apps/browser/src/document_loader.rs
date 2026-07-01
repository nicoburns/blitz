use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use blitz_dom::{DocumentConfig, FontContext};
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_traits::{
    net::{AbortController, AbortSignal, Request, Url},
    shell::ShellProvider,
};
use dioxus_native::{SubDocumentAttr, prelude::*};
use linebender_resource_handle::Blob;

use crate::StdNetProvider;
use crate::downloads;
use crate::favicon::favicon_candidate;
use crate::history::{BrowserNavProvider, History, SyncStore};

pub enum DocumentLoaderStatus {
    Loading,
    Idle,
}

/// The result of loading a URL: either a document to display, or a file that
/// was downloaded (because the response carried `Content-Disposition:
/// attachment`).
pub enum LoadOutcome {
    Document(LoadedDocument),
    Download(DownloadResult),
}

/// Outcome of saving a downloaded attachment to disk.
#[derive(Clone)]
pub struct DownloadResult {
    pub filename: String,
    /// `Ok(path)` when the file was written, `Err(message)` when saving failed.
    pub saved_to: Result<PathBuf, String>,
}

#[derive(Clone)]
pub struct LoadedDocument {
    pub document: SubDocumentAttr,
    pub html_source: String,
    pub title: String,
    // The favicon URL we'd probe for this page, computed without I/O. The
    // actual fetch+decode runs in the background after the tab applies the
    // load, so it doesn't block document swap-in. None means we couldn't even
    // form a candidate (e.g. an error page with no base URL).
    pub favicon_candidate: Option<Url>,
    // True for synthesized error/404 pages. Callers use this to gate side
    // effects that should only fire on real loads (e.g. recording history).
    pub is_error: bool,
}

pub struct DocumentLoader {
    pub font_ctx: FontContext,
    pub net_provider: Arc<StdNetProvider>,
    pub status: Signal<DocumentLoaderStatus>,
    pub history: SyncStore<History>,
    pub reload_generation: Signal<u64>,
    /// Most recent download outcome, surfaced in the status bar. Cleared after
    /// a short delay (see `download_seq`).
    pub download_notice: Signal<Option<DownloadResult>>,
    /// Monotonic token identifying the current download notice, so a delayed
    /// clear only fires when no newer download has replaced it.
    pub download_seq: AtomicU64,
    current_abort: Mutex<Option<AbortController>>,
}

pub fn make_doc_config(
    base_url: Option<String>,
    net_provider: Arc<StdNetProvider>,
    history: SyncStore<History>,
    font_ctx: FontContext,
    abort_signal: Option<AbortSignal>,
) -> DocumentConfig {
    DocumentConfig {
        viewport: None,
        base_url,
        ua_stylesheets: None,
        net_provider: Some(net_provider as _),
        navigation_provider: Some(Arc::new(BrowserNavProvider { history })),
        shell_provider: Some(consume_context::<Arc<dyn ShellProvider>>()),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        font_ctx: Some(font_ctx),
        media_type: None,
        abort_signal,
        ..Default::default()
    }
}

impl DocumentLoader {
    pub fn new(net_provider: Arc<StdNetProvider>, history: SyncStore<History>) -> Self {
        let mut font_ctx = FontContext::default();
        font_ctx
            .collection
            .register_fonts(Blob::new(Arc::new(blitz_dom::BULLET_FONT) as _), None);

        Self {
            font_ctx,
            net_provider,
            status: Signal::new(DocumentLoaderStatus::Idle),
            history,
            reload_generation: Signal::new(0),
            download_notice: Signal::new(None),
            download_seq: AtomicU64::new(0),
            current_abort: Mutex::new(None),
        }
    }

    pub fn reload(&self) {
        self.abort_current();
        let mut reload_generation = self.reload_generation;
        *reload_generation.write() += 1;
    }

    pub fn reload_generation(&self) -> u64 {
        *self.reload_generation.read()
    }

    pub fn abort_current(&self) {
        if let Some(controller) = self.current_abort.lock().unwrap().take() {
            controller.abort();
        }
    }

    pub async fn load_document(&self, req: Request) -> LoadOutcome {
        let net_provider = Arc::clone(&self.net_provider);
        let font_ctx = self.font_ctx.clone();
        let history = self.history;

        let controller = AbortController::default();
        let signal = controller.signal.clone();
        {
            let mut slot = self.current_abort.lock().unwrap();
            if let Some(prev) = slot.take() {
                prev.abort();
            }
            *slot = Some(controller);
        }

        let request_url = req.url.clone();
        let req = req.signal(signal.clone());

        let response = net_provider.fetch_async_with_headers(req).await;

        match response {
            Ok((resolved_url, headers, bytes)) => {
                tracing::info!("Loaded {}", resolved_url);

                // Responses flagged as attachments are saved to disk rather
                // than rendered.
                if downloads::is_attachment(&headers) {
                    let resolved = Url::parse(&resolved_url).unwrap_or(request_url);
                    let filename = downloads::download_filename(&headers, &resolved);
                    tracing::info!("Downloading {} ({} bytes)", filename, bytes.len());

                    let save_name = filename.clone();
                    let saved_to = tokio::task::spawn_blocking(move || {
                        downloads::save_to_downloads(&save_name, &bytes)
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|res| res.map_err(|e| e.to_string()));

                    if let Err(err) = &saved_to {
                        tracing::error!("Failed to save download {}: {}", filename, err);
                    }

                    return LoadOutcome::Download(DownloadResult { filename, saved_to });
                }

                let base_url = resolved_url.clone();
                let config = make_doc_config(
                    Some(resolved_url),
                    net_provider,
                    history,
                    font_ctx,
                    Some(signal.clone()),
                );

                let body_text;
                let (html, is_error) = if bytes.is_empty() {
                    (include_str!("../assets/404.html"), true)
                } else {
                    body_text = String::from_utf8_lossy(&bytes);
                    (&*body_text, false)
                };

                let document = HtmlDocument::from_html(html, config).into_inner();
                let parsed_title = document
                    .find_title_node()
                    .map(|n| n.text_content())
                    .unwrap_or_default();
                let favicon_candidate =
                    favicon_candidate(base_url.as_str(), document.favicon_url().as_deref());
                LoadOutcome::Document(LoadedDocument {
                    document: SubDocumentAttr::new(document),
                    html_source: html.to_string(),
                    title: parsed_title,
                    favicon_candidate,
                    is_error,
                })
            }
            Err(err) => {
                tracing::error!("Error loading document: {:?}", err);

                let error_msg = format!("{err:?}");
                let config =
                    make_doc_config(None, net_provider, history, font_ctx, Some(signal.clone()));

                let error_html = include_str!("../assets/error.html");
                let mut document = HtmlDocument::from_html(error_html, config).into_inner();
                if let Some(text_node) = document
                    .get_element_by_id("error")
                    .and_then(|el| document.get_node(el))
                    .and_then(|node| node.children.first().copied())
                {
                    document.mutate().set_node_text(text_node, &error_msg);
                }
                let parsed_title = document
                    .find_title_node()
                    .map(|n| n.text_content())
                    .unwrap_or_default();
                LoadOutcome::Document(LoadedDocument {
                    document: SubDocumentAttr::new(document),
                    html_source: error_html.to_string(),
                    title: parsed_title,
                    favicon_candidate: None,
                    is_error: true,
                })
            }
        }
    }
}

impl Drop for DocumentLoader {
    fn drop(&mut self) {
        self.abort_current();
    }
}
