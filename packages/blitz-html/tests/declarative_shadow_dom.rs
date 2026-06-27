//! Tests for declarative shadow DOM (`<template shadowrootmode>`).
//!
//! Run with: cargo test -p blitz-html --features shadow-dom

#![cfg(feature = "shadow-dom")]

use std::sync::Arc;

use blitz_dom::DocumentConfig;
use blitz_html::{HtmlDocument, HtmlProvider};

fn parse(html: &str) -> HtmlDocument {
    HtmlDocument::from_html(
        html,
        DocumentConfig {
            html_parser_provider: Some(Arc::new(HtmlProvider)),
            ..Default::default()
        },
    )
}

fn computed_color(doc: &blitz_dom::BaseDocument, node_id: usize) -> (u8, u8, u8) {
    let styles = doc.get_node(node_id).unwrap().primary_styles().unwrap();
    let color = styles.clone_color().into_srgb_legacy();
    let c = color.components;
    (
        (c.0 * 255.0).round() as u8,
        (c.1 * 255.0).round() as u8,
        (c.2 * 255.0).round() as u8,
    )
}

#[test]
fn declarative_shadow_root_is_attached_and_scoped() {
    let mut doc = parse(
        r#"<!doctype html><html><body>
            <div id="host">
                <template shadowrootmode="open">
                    <style>p { color: rgb(0, 128, 0); }</style>
                    <p id="shadow-p">shadow content</p>
                    <slot></slot>
                </template>
                <p id="light-p">light content</p>
            </div>
        </body></html>"#,
    );

    let host = doc.query_selector("#host").unwrap().unwrap();

    // The host should have acquired a shadow root.
    let shadow_root = doc.shadow_root_id(host);
    assert!(shadow_root.is_some(), "declarative shadow root attached");

    doc.resolve(0.0);

    // Find the shadow <p> by navigating the shadow tree (query_selector only
    // traverses the light DOM).
    let shadow_root = shadow_root.unwrap();
    let shadow_p = doc
        .get_node(shadow_root)
        .unwrap()
        .children
        .iter()
        .copied()
        .find(|id| {
            doc.get_node(*id)
                .and_then(|n| n.element_data())
                .is_some_and(|el| el.name.local.as_ref() == "p")
        })
        .expect("shadow <p> exists");

    // The shadow <p> should be coloured by the scoped style.
    assert_eq!(computed_color(&doc, shadow_p), (0, 128, 0));

    // The light-DOM <p> should be slotted (assigned to the default slot).
    let light_p = doc.query_selector("#light-p").unwrap().unwrap();
    let light_node = doc.get_node(light_p).unwrap();
    assert!(light_node.element_data().unwrap().assigned_slot.is_some());
}
