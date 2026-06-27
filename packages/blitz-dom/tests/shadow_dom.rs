//! Integration tests for Shadow DOM and Custom Element support.
//!
//! These only compile/run with the `shadow-dom` feature enabled:
//!   cargo test -p blitz-dom --features shadow-dom

#![cfg(feature = "shadow-dom")]

use std::cell::RefCell;
use std::rc::Rc;

use blitz_dom::node::{CustomElement, CustomElementCtx, CustomElementDefinition};
use blitz_dom::{BaseDocument, DocumentConfig, LocalName, QualName, ShadowRootMode, ns};

fn qname(local: &str) -> QualName {
    QualName {
        prefix: None,
        ns: ns!(html),
        local: LocalName::from(local),
    }
}

/// Build a document with `<html><body>` and return (doc, body_id).
fn doc_with_body() -> (BaseDocument, usize) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let mut mutator = doc.mutate();
    let html = mutator.create_element(qname("html"), Vec::new());
    let body = mutator.create_element(qname("body"), Vec::new());
    mutator.append_children(html, &[body]);
    mutator.append_children(0, &[html]);
    drop(mutator);
    (doc, body)
}

#[test]
fn attach_shadow_creates_shadow_root() {
    let (mut doc, body) = doc_with_body();
    let mut mutator = doc.mutate();
    let host = mutator.create_element(qname("my-host"), Vec::new());
    mutator.append_children(body, &[host]);
    let shadow_root = mutator.attach_shadow(host, ShadowRootMode::Open);
    drop(mutator);

    assert_eq!(doc.shadow_root_id(host), Some(shadow_root));
    assert!(doc.get_node(shadow_root).unwrap().is_shadow_root());
    assert_eq!(doc.get_node(shadow_root).unwrap().parent, Some(host));
}

#[test]
fn shadow_content_is_styled_and_laid_out() {
    let (mut doc, body) = doc_with_body();
    let mut mutator = doc.mutate();

    let host = mutator.create_element(qname("my-host"), Vec::new());
    mutator.set_attribute(host, qname("style"), "display:block");
    mutator.append_children(body, &[host]);

    let shadow_root = mutator.attach_shadow(host, ShadowRootMode::Open);
    let div = mutator.create_element(qname("div"), Vec::new());
    mutator.set_attribute(div, qname("style"), "width:50px;height:30px");
    mutator.append_children(shadow_root, &[div]);
    drop(mutator);

    doc.resolve(0.0);

    // The shadow div should have been styled (Stylo traversed the flattened
    // tree) and laid out at its specified size.
    let div_node = doc.get_node(div).unwrap();
    assert!(
        div_node.primary_styles().is_some(),
        "shadow content should be styled"
    );
    assert_eq!(div_node.final_layout.size.width, 50.0);
    assert_eq!(div_node.final_layout.size.height, 30.0);
}

#[test]
fn slot_projects_light_dom_children() {
    let (mut doc, body) = doc_with_body();
    let mut mutator = doc.mutate();

    let host = mutator.create_element(qname("my-host"), Vec::new());
    mutator.set_attribute(host, qname("style"), "display:block");
    // Light DOM child
    let light = mutator.create_element(qname("span"), Vec::new());
    mutator.set_attribute(
        light,
        qname("style"),
        "display:block;width:40px;height:20px",
    );
    mutator.append_children(host, &[light]);
    mutator.append_children(body, &[host]);

    // Shadow tree: a wrapper containing a default <slot>
    let shadow_root = mutator.attach_shadow(host, ShadowRootMode::Open);
    let wrapper = mutator.create_element(qname("div"), Vec::new());
    mutator.set_attribute(wrapper, qname("style"), "display:block");
    let slot = mutator.create_element(qname("slot"), Vec::new());
    mutator.append_children(wrapper, &[slot]);
    mutator.append_children(shadow_root, &[wrapper]);
    drop(mutator);

    doc.resolve(0.0);

    // The light-DOM span should be assigned to the slot and laid out.
    let light_node = doc.get_node(light).unwrap();
    assert_eq!(
        light_node.element_data().unwrap().assigned_slot,
        Some(slot),
        "light child should be assigned to the default slot"
    );
    assert!(light_node.primary_styles().is_some());
    assert_eq!(light_node.final_layout.size.width, 40.0);
    assert_eq!(light_node.final_layout.size.height, 20.0);
}

fn computed_color(doc: &BaseDocument, node_id: usize) -> (u8, u8, u8) {
    let styles = doc.get_node(node_id).unwrap().primary_styles().unwrap();
    let color = styles.clone_color().into_srgb_legacy();
    let components = color.components;
    (
        (components.0 * 255.0).round() as u8,
        (components.1 * 255.0).round() as u8,
        (components.2 * 255.0).round() as u8,
    )
}

#[test]
fn scoped_shadow_style_applies_and_is_encapsulated() {
    let (mut doc, body) = doc_with_body();
    let mut mutator = doc.mutate();

    // A global (document) author rule colouring all <p> red.
    let global_style = mutator.create_element(qname("style"), Vec::new());
    let global_css = mutator.create_text_node("p { color: rgb(255, 0, 0); }");
    mutator.append_children(global_style, &[global_css]);
    mutator.append_children(body, &[global_style]);

    // A light-DOM <p> that should be red (document rule applies).
    let light_p = mutator.create_element(qname("p"), Vec::new());
    mutator.append_children(body, &[light_p]);

    let host = mutator.create_element(qname("my-host"), Vec::new());
    mutator.set_attribute(host, qname("style"), "display:block");
    mutator.append_children(body, &[host]);

    // Shadow tree with a scoped rule colouring its <p> green.
    let shadow_root = mutator.attach_shadow(host, ShadowRootMode::Open);
    let shadow_style = mutator.create_element(qname("style"), Vec::new());
    let shadow_css = mutator.create_text_node("p { color: rgb(0, 128, 0); }");
    mutator.append_children(shadow_style, &[shadow_css]);
    let shadow_p = mutator.create_element(qname("p"), Vec::new());
    mutator.append_children(shadow_root, &[shadow_style, shadow_p]);
    drop(mutator);

    doc.resolve(0.0);

    // Light <p> is coloured by the document rule.
    assert_eq!(computed_color(&doc, light_p), (255, 0, 0));
    // Shadow <p> is coloured by the scoped shadow rule (green), demonstrating
    // that the scoped stylesheet applies.
    assert_eq!(computed_color(&doc, shadow_p), (0, 128, 0));
}

struct GreetWidget {
    log: Rc<RefCell<Vec<String>>>,
}

impl CustomElement for GreetWidget {
    fn connected(&mut self, ctx: &mut CustomElementCtx<'_, '_>) {
        self.log.borrow_mut().push("connected".to_string());
        let name = ctx.host_attr(LocalName::from("name")).unwrap_or_default();
        // Build the shadow tree via the mutator API (the default test config
        // has no HTML parser, so we can't use set_shadow_html here).
        let shadow_root = ctx.shadow_root_id();
        let div = ctx.mutator().create_element(qname("div"), Vec::new());
        let text = ctx.mutator().create_text_node(&format!("Hello {name}"));
        ctx.mutator().append_children(div, &[text]);
        ctx.mutator().append_children(shadow_root, &[div]);
    }

    fn attribute_changed(
        &mut self,
        _ctx: &mut CustomElementCtx<'_, '_>,
        name: &str,
        _old: Option<&str>,
        new: Option<&str>,
    ) {
        self.log
            .borrow_mut()
            .push(format!("attr:{name}={}", new.unwrap_or("")));
    }
}

#[test]
fn custom_element_registry_upgrades_on_insertion() {
    let (mut doc, body) = doc_with_body();

    let log = Rc::new(RefCell::new(Vec::new()));
    let log_clone = log.clone();
    doc.define_custom_element(
        LocalName::from("greet-box"),
        CustomElementDefinition::new(move || {
            Box::new(GreetWidget {
                log: log_clone.clone(),
            })
        }),
    );

    let mut mutator = doc.mutate();
    let host = mutator.create_element(qname("greet-box"), Vec::new());
    mutator.set_attribute(host, qname("name"), "World");
    mutator.append_children(body, &[host]);
    drop(mutator);

    // The element should have been upgraded: a shadow root attached and
    // `connected` run.
    assert!(doc.shadow_root_id(host).is_some(), "shadow root attached");
    assert!(
        log.borrow().iter().any(|s| s == "connected"),
        "connected callback ran"
    );

    doc.resolve(0.0);

    // The shadow content built by the controller should be styled.
    let shadow_root = doc.shadow_root_id(host).unwrap();
    let shadow_div = doc.get_node(shadow_root).unwrap().children[0];
    assert!(doc.get_node(shadow_div).unwrap().primary_styles().is_some());
}
