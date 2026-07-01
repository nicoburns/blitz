//! Toolbar downloads button and dropdown menu.
//!
//! The button only appears once a download has occurred in the current session.
//! While a download is in progress it shows a circular progress bar: a
//! determinate ring when the size is known (`Content-Length`), otherwise an
//! indeterminate spinner. Clicking it opens a dropdown listing all session
//! downloads.

use dioxus_native::prelude::*;

use crate::downloads::{self, Download, DownloadStatus, Downloads, format_bytes};
use crate::icons;

/// Track color of the progress ring (the "unfilled" portion).
const RING_TRACK: &str = "#E0E0E0";
/// Filled color of the progress ring.
const RING_FILL: &str = "#5E9ED6";

#[component]
pub fn DownloadsButton() -> Element {
    let downloads = use_context::<Downloads>();
    let mut menu_open = use_signal(|| false);

    // Only present once something has been downloaded this session.
    if !downloads.has_any() {
        return rsx!();
    }

    let is_active = downloads.has_active();
    let progress = downloads.active_progress();
    // Most recent downloads first.
    let entries: Vec<Download> = downloads.items().read().iter().rev().cloned().collect();

    let idle_button_class = if menu_open() {
        "iconbutton active"
    } else {
        "iconbutton"
    };

    rsx!(
        div { class: "downloads-wrapper",
            if is_active {
                // A progress indicator is shown; use a plain (non-highlighting)
                // button so the ring's transparent-matching hole always sits on
                // the toolbar background.
                div {
                    class: "downloads-button",
                    onclick: move |_| menu_open.toggle(),
                    if let Some(fraction) = progress {
                        {progress_ring(fraction, "")}
                    } else {
                        div { class: "download-spinner" }
                    }
                }
            } else {
                div {
                    class: idle_button_class,
                    onclick: move |_| menu_open.toggle(),
                    img { class: "urlbar-icon", src: icons::DOWNLOAD_ICON }
                }
            }
            if menu_open() {
                div { class: "menu-dropdown downloads-dropdown",
                    div { class: "downloads-header", "Downloads" }
                    for item in entries {
                        DownloadRow { key: "{item.id}", item, menu_open }
                    }
                }
            }
        }
    )
}

/// A determinate circular progress ring rendered with a conic-gradient pie plus
/// an inner hole to punch out the centre. `extra` adds context-specific classes
/// (e.g. a size modifier).
fn progress_ring(fraction: f32, extra: &str) -> Element {
    let degrees = (fraction * 360.0).round().clamp(0.0, 360.0) as i32;
    let style =
        format!("background: conic-gradient({RING_FILL} {degrees}deg, {RING_TRACK} {degrees}deg);");
    rsx!(
        div { class: "download-progress {extra}", style: "{style}",
            div { class: "download-progress-hole" }
        }
    )
}

/// The leading indicator for a download row: a progress ring / spinner while
/// in progress (in place of the icon), otherwise the download icon.
fn row_indicator(status: &DownloadStatus) -> Element {
    match status {
        DownloadStatus::InProgress { .. } => match status.fraction() {
            Some(fraction) => progress_ring(fraction, "download-progress--sm"),
            None => rsx!(div {
                class: "download-spinner download-spinner--sm"
            }),
        },
        _ => rsx!(img {
            class: "menu-item-icon",
            src: icons::DOWNLOAD_ICON
        }),
    }
}

#[component]
fn DownloadRow(item: Download, menu_open: Signal<bool>) -> Element {
    let completed_path = match &item.status {
        DownloadStatus::Completed { path } => Some(path.clone()),
        _ => None,
    };

    let subtitle = match &item.status {
        DownloadStatus::InProgress {
            downloaded,
            total: Some(total),
        } => {
            let percent = item.status.fraction().unwrap_or(0.0) * 100.0;
            format!(
                "{} / {} ({:.0}%)",
                format_bytes(*downloaded),
                format_bytes(*total),
                percent
            )
        }
        DownloadStatus::InProgress {
            downloaded,
            total: None,
        } => format!("Downloading… {}", format_bytes(*downloaded)),
        DownloadStatus::Completed { path } => path.display().to_string(),
        DownloadStatus::Failed { error } => format!("Failed: {error}"),
    };

    let subtitle_class = if matches!(item.status, DownloadStatus::Failed { .. }) {
        "download-row-subtitle download-row-subtitle--error"
    } else {
        "download-row-subtitle"
    };

    // Clicking a completed row opens the file; the reveal action gets its own
    // control so it doesn't also trigger the open.
    let open_path = completed_path.clone();
    let on_open = move |_| {
        if let Some(path) = &open_path {
            downloads::open_file(path);
            menu_open.set(false);
        }
    };

    let reveal_path = completed_path.clone();
    let on_reveal = move |evt: Event<MouseData>| {
        evt.stop_propagation();
        if let Some(path) = &reveal_path {
            downloads::reveal_in_folder(path);
            menu_open.set(false);
        }
    };

    let row_class = if completed_path.is_some() {
        "menu-item download-row"
    } else {
        "menu-item download-row download-row--disabled"
    };

    rsx!(
        div { class: row_class, onclick: on_open,
            {row_indicator(&item.status)}
            div { class: "download-row-text",
                div { class: "download-row-title", "{item.filename}" }
                div { class: subtitle_class, "{subtitle}" }
            }
            if completed_path.is_some() {
                div {
                    class: "download-row-reveal",
                    title: "Show in folder",
                    onclick: on_reveal,
                    img { class: "menu-item-icon", src: icons::FOLDER_OPEN_ICON }
                }
            }
        }
    )
}
