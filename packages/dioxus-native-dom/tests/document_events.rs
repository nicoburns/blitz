//! Tests for `addEventListener`-style event listeners: the `use_html_event`/
//! `use_body_event` hooks and `NodeHandle::add_event_listener`.

use blitz_dom::Document as _;
use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
use dioxus::prelude::*;
use dioxus_native_dom::{
    DioxusDocument, DocumentConfig, NodeHandle, use_body_event, use_html_event,
};
use keyboard_types::{Code, Key, Location, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Default)]
struct Counts {
    body: Rc<Cell<u32>>,
    html: Rc<Cell<u32>>,
    node: Rc<Cell<u32>>,
}

/// Records the order in which event handlers ran
#[derive(Clone, Default)]
struct Log(Rc<RefCell<Vec<&'static str>>>);

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

fn create_document<T: Clone + 'static>(app: fn() -> Element, context: &T) -> DioxusDocument {
    let vdom = VirtualDom::new(app);
    vdom.provide_root_context(context.clone());
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

#[test]
fn add_event_listener_supports_multiple_listeners_per_node() {
    fn app() -> Element {
        let counts = use_context::<Counts>();
        rsx! {
            div {
                id: "content",
                onmounted: move |evt: Event<MountedData>| {
                    let handle = evt.downcast::<NodeHandle>().unwrap().clone();
                    let counts1 = counts.clone();
                    handle.add_event_listener("keydown", move |_: Event<KeyboardData>| {
                        counts1.node.set(counts1.node.get() + 1);
                    });
                    let counts2 = counts.clone();
                    handle.add_event_listener("keydown", move |_: Event<KeyboardData>| {
                        counts2.node.set(counts2.node.get() + 10);
                    });
                },
                "hello"
            }
        }
    }

    let counts = Counts::default();
    let mut doc = create_document(app, &counts);

    // Both listeners fire when the node is the event target
    focus_content(&mut doc);
    doc.handle_ui_event(keydown());
    assert_eq!(counts.node.get(), 11);

    // Neither listener fires when the node is not in the bubble chain
    doc.inner.borrow_mut().clear_focus();
    doc.handle_ui_event(keydown());
    assert_eq!(counts.node.get(), 11);
}

#[test]
fn node_listeners_run_after_rsx_handler_and_survive_same_node_stop_propagation() {
    fn app() -> Element {
        let log = use_context::<Log>();
        use_body_event("keydown", {
            let log = log.clone();
            move |_: Event<KeyboardData>| log.0.borrow_mut().push("body")
        });
        let log_rsx = log.clone();
        let log_mount = log.clone();
        rsx! {
            div {
                id: "content",
                onkeydown: move |evt: Event<KeyboardData>| {
                    evt.stop_propagation();
                    log_rsx.0.borrow_mut().push("rsx");
                },
                onmounted: move |evt: Event<MountedData>| {
                    let handle = evt.downcast::<NodeHandle>().unwrap().clone();
                    let log = log_mount.clone();
                    handle.add_event_listener("keydown", move |_: Event<KeyboardData>| {
                        log.0.borrow_mut().push("listener");
                    });
                },
                "hello"
            }
        }
    }

    let log = Log::default();
    let mut doc = create_document(app, &log);

    focus_content(&mut doc);
    doc.handle_ui_event(keydown());

    // The rsx attribute handler runs first, then the add_event_listener listener
    // on the same node (despite the rsx handler calling stop_propagation, matching
    // browser stopPropagation semantics). The <body> handler does not run because
    // propagation was stopped at the div.
    assert_eq!(*log.0.borrow(), vec!["rsx", "listener"]);
}

#[test]
fn node_listeners_survive_detach_and_are_purged_on_drop() {
    #[component]
    fn Child() -> Element {
        let counts = use_context::<Counts>();
        rsx! {
            div {
                div {
                    id: "content",
                    onmounted: move |evt: Event<MountedData>| {
                        let handle = evt.downcast::<NodeHandle>().unwrap().clone();
                        let counts = counts.clone();
                        handle.add_event_listener("keydown", move |_: Event<KeyboardData>| {
                            counts.node.set(counts.node.get() + 1);
                        });
                    },
                    "child"
                }
            }
        }
    }

    fn app() -> Element {
        let mut show = use_signal(|| true);
        use_html_event("keyup", move |_: Event<KeyboardData>| show.toggle());
        rsx! {
            if show() {
                Child {}
            }
        }
    }

    let counts = Counts::default();
    let mut doc = create_document(app, &counts);

    // One hook listener (keyup on <html>) + one node listener (keydown on the div)
    assert_eq!(doc.event_listener_count(), 2);

    // Unmounting the Child component detaches its nodes from the document but does
    // not drop them, so the div's listener is retained
    doc.handle_ui_event(keyup());
    doc.poll(None);
    assert_eq!(doc.event_listener_count(), 2);

    // Remounting the Child component reuses the ElementIds of the unmounted nodes,
    // which drops the detached nodes and purges the old div's listener. The new
    // mount then registers a fresh listener, so the total count is unchanged.
    doc.handle_ui_event(keyup());
    doc.poll(None);
    assert_eq!(doc.event_listener_count(), 2);

    // Only the new listener fires: the old listener must not fire even though its
    // node id has been recycled (very likely for the remounted div itself)
    focus_content(&mut doc);
    doc.handle_ui_event(keydown());
    assert_eq!(counts.node.get(), 1);
}
