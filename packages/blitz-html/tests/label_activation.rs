//! Activating a checkbox via its <label> (todomvc's "toggle-all" pattern):
//! the label is absolutely positioned outside its parent's border box, the
//! parent is a hoisted stacking context (z-index), and the visible part of the
//! label is a transformed ::before pseudo-element. Clicking it must toggle the
//! bound checkbox and fire both `input` and `change` events on it.

use blitz_dom::{Document, DocumentConfig, EventDriver, EventHandler};
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_traits::{
    events::{
        BlitzPointerEvent, BlitzPointerId, DomEvent, EventState, MouseEventButton,
        MouseEventButtons, Point, PointerCoords, PointerDetails, UiEvent,
    },
    shell::{ColorScheme, Viewport},
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// A reduced version of todomvc's "toggle-all" checkbox. The label overflows
/// the top of `.main` (which stacks above the preceding header via z-index)
/// and is rendered via a rotated ::before pseudo-element.
const TOGGLE_ALL_HTML: &str = r#"<html><head><style>
    body { margin: 0; }
    .new-todo { display: block; width: 400px; height: 60px; }
    .main { position: relative; z-index: 2; }
    .toggle-all { opacity: 0; position: absolute; }
    .toggle-all + label {
        width: 60px;
        height: 34px;
        font-size: 0;
        position: absolute;
        top: -52px;
        left: 0;
        transform: rotate(90deg);
    }
    .toggle-all + label:before { content: 'v'; font-size: 22px; padding: 10px 27px; }
</style></head><body>
    <header><input class="new-todo" type="text"></header>
    <section class="main">
        <input id="toggle-all" class="toggle-all" type="checkbox">
        <label for="toggle-all"></label>
        <div style="height: 100px;"></div>
    </section>
</body></html>"#;

fn doc(html: &str) -> HtmlDocument {
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(400, 400, 1.0, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    doc
}

fn node_id(doc: &HtmlDocument, selector: &str) -> usize {
    doc.query_selector(selector).unwrap().expect(selector)
}

fn is_checked(doc: &HtmlDocument, selector: &str) -> bool {
    let id = node_id(doc, selector);
    doc.get_node(id)
        .and_then(|node| node.element_data())
        .and_then(|el| el.checkbox_input_checked())
        .unwrap()
}

fn pointer_event(x: f32, y: f32) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button: MouseEventButton::Main,
        buttons: MouseEventButtons::from(MouseEventButton::Main),
        mods: Default::default(),
        details: PointerDetails::default(),
        element: Point::default(),
        active_pointers: Default::default(),
    }
}

/// Records the (name, target) of every event exposed to "script"
struct RecordingHandler {
    events: Rc<RefCell<Vec<(&'static str, usize)>>>,
}
impl EventHandler for RecordingHandler {
    fn handle_event(
        &mut self,
        _chain: &[usize],
        event: &mut DomEvent,
        _doc: &mut dyn Document,
        _event_state: &mut EventState,
    ) {
        self.events.borrow_mut().push((event.name(), event.target));
    }
}

fn click(doc: &mut HtmlDocument, x: f32, y: f32) -> Vec<(&'static str, usize)> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let handler = RecordingHandler {
        events: Rc::clone(&events),
    };
    let mut driver = EventDriver::new(doc, handler);
    driver.handle_ui_event(UiEvent::PointerDown(pointer_event(x, y)));
    driver.handle_ui_event(UiEvent::PointerUp(pointer_event(x, y)));
    doc.resolve(0.0);
    let events = events.borrow();
    events.clone()
}

#[test]
fn clicking_overflowing_label_toggles_bound_checkbox() {
    let mut doc = doc(TOGGLE_ALL_HTML);

    assert!(!is_checked(&doc, "#toggle-all"));

    // The label's border box spans (0..60, 8..42) in page coordinates: it
    // overflows the top of `.main` (y=60), overlapping the header's text input.
    // `.main` is a hoisted stacking context (z-index: 2), so the label must
    // still be the hit target there, and activating it toggles the checkbox.
    let checkbox = node_id(&doc, "#toggle-all");
    let events = click(&mut doc, 30.0, 25.0);
    assert!(
        is_checked(&doc, "#toggle-all"),
        "checkbox checked after clicking its label"
    );
    assert!(
        events.contains(&("input", checkbox)),
        "input event fired on the checkbox: {events:?}"
    );
    assert!(
        events.contains(&("change", checkbox)),
        "change event fired on the checkbox: {events:?}"
    );

    // Clicking again unchecks it.
    click(&mut doc, 30.0, 25.0);
    assert!(!is_checked(&doc, "#toggle-all"));
}

#[test]
fn clicking_checkbox_fires_input_then_change() {
    let mut doc = doc(r#"<html><body style="margin:0">
        <input id="cb" type="checkbox" style="width:20px; height:20px;">
    </body></html>"#);

    let checkbox = node_id(&doc, "#cb");
    let events = click(&mut doc, 10.0, 10.0);
    assert!(is_checked(&doc, "#cb"));

    let form_events: Vec<_> = events
        .iter()
        .filter(|(name, _)| matches!(*name, "input" | "change"))
        .collect();
    assert_eq!(
        form_events,
        vec![&("input", checkbox), &("change", checkbox)],
        "input fires before change: {events:?}"
    );
}
