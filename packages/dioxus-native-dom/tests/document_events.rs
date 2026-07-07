//! Tests for event handlers registered on the `<html>`/`<body>` elements
//! via the `use_html_event`/`use_body_event` hooks.

use blitz_dom::Document as _;
use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
use dioxus::prelude::*;
use dioxus_native_dom::{DioxusDocument, DocumentConfig, use_body_event, use_html_event};
use keyboard_types::{Code, Key, Location, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Default)]
struct Counts {
    body: Rc<Cell<u32>>,
    html: Rc<Cell<u32>>,
}

fn keydown() -> UiEvent {
    UiEvent::KeyDown(key_event())
}

fn keyup() -> UiEvent {
    UiEvent::KeyUp(key_event())
}

fn key_event() -> BlitzKeyEvent {
    BlitzKeyEvent {
        key: Key::Character("a".to_string()),
        code: Code::KeyA,
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: None,
    }
}

fn create_document(app: fn() -> Element, counts: &Counts) -> DioxusDocument {
    let vdom = VirtualDom::new(app);
    vdom.provide_root_context(counts.clone());
    let mut doc = DioxusDocument::new(vdom, DocumentConfig::default());
    doc.initial_build();
    doc
}

/// Focus the `<div id="content">` element rendered by the test apps
fn focus_content(doc: &mut DioxusDocument) {
    let node_id = doc.inner.borrow().get_element_by_id("content").unwrap();
    doc.inner.borrow_mut().set_focus_to(node_id);
}

#[test]
fn html_and_body_handlers_receive_events() {
    fn app() -> Element {
        let counts = use_context::<Counts>();
        use_body_event("keydown", {
            let counts = counts.clone();
            move |_: Event<KeyboardData>| counts.body.set(counts.body.get() + 1)
        });
        use_html_event("keydown", move |_: Event<KeyboardData>| {
            counts.html.set(counts.html.get() + 1)
        });
        rsx! { div { id: "content", "hello" } }
    }

    let counts = Counts::default();
    let mut doc = create_document(app, &counts);

    // With a focused element inside <body>, keydown events bubble through
    // both <body> and <html>
    focus_content(&mut doc);
    doc.handle_ui_event(keydown());
    assert_eq!(counts.body.get(), 1);
    assert_eq!(counts.html.get(), 1);

    // With no focused element, keydown events target the <body> element
    // (matching browsers) and bubble up to the <html> element
    doc.inner.borrow_mut().clear_focus();
    doc.handle_ui_event(keydown());
    assert_eq!(counts.body.get(), 2);
    assert_eq!(counts.html.get(), 2);
}

#[test]
fn stop_propagation_in_body_handler_prevents_html_handler() {
    fn app() -> Element {
        let counts = use_context::<Counts>();
        use_body_event("keydown", {
            let counts = counts.clone();
            move |event: Event<KeyboardData>| {
                event.stop_propagation();
                counts.body.set(counts.body.get() + 1);
            }
        });
        use_html_event("keydown", move |_: Event<KeyboardData>| {
            counts.html.set(counts.html.get() + 1)
        });
        rsx! { div { id: "content", "hello" } }
    }

    let counts = Counts::default();
    let mut doc = create_document(app, &counts);

    focus_content(&mut doc);
    doc.handle_ui_event(keydown());
    assert_eq!(counts.body.get(), 1);
    assert_eq!(counts.html.get(), 0);
}

#[test]
fn handlers_are_removed_on_unmount() {
    #[component]
    fn Child() -> Element {
        let counts = use_context::<Counts>();
        use_html_event("keydown", move |_: Event<KeyboardData>| {
            counts.html.set(counts.html.get() + 1)
        });
        rsx! { div { "child" } }
    }

    fn app() -> Element {
        let mut show = use_signal(|| true);
        use_html_event("keyup", move |_: Event<KeyboardData>| show.set(false));
        rsx! {
            div { id: "content",
                if show() {
                    Child {}
                }
            }
        }
    }

    let counts = Counts::default();
    let mut doc = create_document(app, &counts);

    doc.handle_ui_event(keydown());
    assert_eq!(counts.html.get(), 1);

    // The keyup handler unmounts the Child component, which should
    // unregister its keydown handler
    doc.handle_ui_event(keyup());
    doc.poll(None);
    doc.handle_ui_event(keydown());
    assert_eq!(counts.html.get(), 1);
}
