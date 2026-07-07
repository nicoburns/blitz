//! Key events fired while no element is focussed target the `<body>` element
//! (matching browser behaviour, where the `<body>` is the default "activeElement").

use blitz_dom::{Document, DocumentConfig, EventDriver, EventHandler};
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_traits::events::{BlitzKeyEvent, DomEvent, DomEventData, EventState, KeyState, UiEvent};
use keyboard_types::{Code, Key, Location, Modifiers};

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Default)]
struct RecordKeydownTargets(Rc<RefCell<Vec<usize>>>);

impl EventHandler for RecordKeydownTargets {
    fn handle_event(
        &mut self,
        _chain: &[usize],
        event: &mut DomEvent,
        _doc: &mut dyn Document,
        _event_state: &mut EventState,
    ) {
        if matches!(event.data, DomEventData::KeyDown(_)) {
            self.0.borrow_mut().push(event.target);
        }
    }
}

fn keydown() -> UiEvent {
    UiEvent::KeyDown(BlitzKeyEvent {
        key: Key::Character("a".to_string()),
        code: Code::KeyA,
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: None,
    })
}

#[test]
fn unfocussed_key_events_target_the_body_element() {
    let mut doc = HtmlDocument::from_html(
        r#"<html><body><input id="input" type="text"></body></html>"#,
        DocumentConfig {
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            ..Default::default()
        },
    );
    let body_id = doc.try_body_element().unwrap().id;
    let input_id = doc.get_element_by_id("input").unwrap();

    let recorder = RecordKeydownTargets::default();

    // With no focussed element, key events target the <body> element
    let mut driver = EventDriver::new(&mut doc, recorder.clone());
    driver.handle_ui_event(keydown());
    assert_eq!(*recorder.0.borrow(), vec![body_id]);

    // With a focussed element, key events target the focussed element
    doc.set_focus_to(input_id);
    let mut driver = EventDriver::new(&mut doc, recorder.clone());
    driver.handle_ui_event(keydown());
    assert_eq!(*recorder.0.borrow(), vec![body_id, input_id]);
}
