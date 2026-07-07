use crate::document_event_handlers::{
    DocumentEventHandlerId, DocumentEventHandlers, SpecialElement,
};
use blitz_traits::events::DomEventKind;
use dioxus_core::{Event, Runtime, consume_context, current_scope_id, use_hook_with_cleanup};
use dioxus_html::PlatformEventData;
use std::rc::Rc;
use std::str::FromStr as _;

/// Register an event handler that runs when the given DOM event (e.g. "click" or
/// "keydown") reaches the `<body>` element, either by bubbling up from a descendant
/// or by targeting the `<body>` element directly.
///
/// The `<body>` element is not rendered by Dioxus (for compatibility with the web
/// backend), so event handlers cannot be attached to it via `rsx!`. This hook
/// provides the equivalent capability.
///
/// Handlers on descendant elements run first: if one of them calls
/// `stop_propagation` then the `<body>` handler will not run. Calling
/// `prevent_default`/`stop_propagation` on the passed event behaves as it would
/// for a regular element event handler.
///
/// Note: keyboard events fired while no element is focused target the `<body>`
/// element (matching browsers), so this hook can be used for "global" key handling.
///
/// Returns a [`DocumentEventHandlerId`] which can be used to remove the handler.
///
/// ### Example
///
/// ```rust,ignore
/// use dioxus_native_dom::use_body_event;
/// use dioxus_html::MouseData;
///
/// use_body_event("click", |event: dioxus_core::Event<MouseData>| {
///     println!("body clicked at {:?}", event.client_coordinates());
/// });
/// ```
pub fn use_body_event<T>(
    event: &str,
    handler: impl FnMut(Event<T>) + 'static,
) -> DocumentEventHandlerId
where
    T: 'static,
    for<'a> T: From<&'a PlatformEventData>,
{
    use_special_element_event(SpecialElement::Body, event, handler)
}

/// As [`use_body_event`], but for the root `<html>` element.
///
/// Events which target the `<body>` element (or its descendants) also bubble up
/// to the `<html>` element, so handlers registered with this hook run for those
/// events too (unless a `<body>`-or-lower handler calls `stop_propagation`).
pub fn use_html_event<T>(
    event: &str,
    handler: impl FnMut(Event<T>) + 'static,
) -> DocumentEventHandlerId
where
    T: 'static,
    for<'a> T: From<&'a PlatformEventData>,
{
    use_special_element_event(SpecialElement::Html, event, handler)
}

fn use_special_element_event<T>(
    element: SpecialElement,
    event: &str,
    mut handler: impl FnMut(Event<T>) + 'static,
) -> DocumentEventHandlerId
where
    T: 'static,
    for<'a> T: From<&'a PlatformEventData>,
{
    // `DomEventKind::from_str` accepts both "click" and "onclick" style names
    let kind = DomEventKind::from_str(event)
        .unwrap_or_else(|()| panic!("Unknown DOM event name: {event}"));
    let runtime = Runtime::current();
    let scope_id = current_scope_id();

    use_hook_with_cleanup(
        move || {
            let handlers: Rc<DocumentEventHandlers> = consume_context();
            handlers.add(element, kind, move |event| {
                runtime.in_scope(scope_id, || handler(event.map(|data| data.into())))
            })
        },
        move |handler_id| handler_id.remove(),
    )
}
