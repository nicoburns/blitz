//! Toolbar downloads button and dropdown menu.
//!
//! The button only appears once a download has occurred in the current session.
//! It shows a spinner while any download is in progress, and opens a dropdown
//! listing all session downloads when clicked.

use dioxus_native::prelude::*;

use crate::downloads::{self, Download, DownloadStatus, Downloads};
use crate::icons;

#[component]
pub fn DownloadsButton() -> Element {
    let downloads = use_context::<Downloads>();
    let mut menu_open = use_signal(|| false);

    // Only present once something has been downloaded this session.
    if !downloads.has_any() {
        return rsx!();
    }

    let is_active = downloads.has_active();
    // Most recent downloads first.
    let entries: Vec<Download> = downloads.items().read().iter().rev().cloned().collect();

    let button_class = if menu_open() {
        "iconbutton active"
    } else {
        "iconbutton"
    };

    rsx!(
        div { class: "downloads-wrapper",
            div {
                class: button_class,
                onclick: move |_| menu_open.toggle(),
                if is_active {
                    div { class: "download-spinner" }
                } else {
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

#[component]
fn DownloadRow(item: Download, menu_open: Signal<bool>) -> Element {
    let completed_path = match &item.status {
        DownloadStatus::Completed { path } => Some(path.clone()),
        _ => None,
    };

    let subtitle = match &item.status {
        DownloadStatus::InProgress => "Downloading…".to_string(),
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
            img { class: "menu-item-icon", src: icons::DOWNLOAD_ICON }
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
