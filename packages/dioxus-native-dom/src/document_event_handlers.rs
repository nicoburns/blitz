use blitz_traits::events::DomEventKind;
use dioxus_core::Event;
use dioxus_html::PlatformEventData;
use slotmap::{DefaultKey, Key, KeyData, SlotMap};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// The elements which exist outside of the Dioxus vdom that document event
/// handlers can be registered against.
///
/// Dioxus Native does not render the `<html>` or `<body>` elements with Dioxus
/// (for compatibility with the web backend), so event handlers cannot be attached
/// to them via `rsx!`. Handlers registered via [`use_html_event`](crate::use_html_event)
/// and [`use_body_event`](crate::use_body_event) are attached to these elements instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecialElement {
    /// The root `<html>` element
    Html,
    /// The `<body>` element
    Body,
}

/// The unique identifier of a document event handler. This can be used to later remove the handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentEventHandlerId(pub(crate) u64);

impl DocumentEventHandlerId {
    /// Unregister this event handler from the document
    pub fn remove(&self) {
        let handlers: Rc<DocumentEventHandlers> = dioxus_core::consume_context();
        handlers.remove(*self);
    }
}

struct DocumentEventHandlerInner {
    element: SpecialElement,
    kind: DomEventKind,
    #[allow(clippy::type_complexity)]
    handler: Box<dyn FnMut(Event<PlatformEventData>) + 'static>,
}

/// Event handlers registered against elements which are not managed by the
/// Dioxus vdom (the `<html>` and `<body>` elements).
#[derive(Default)]
pub(crate) struct DocumentEventHandlers {
    handlers: RefCell<SlotMap<DefaultKey, DocumentEventHandlerInner>>,
}

/// The aggregate result of dispatching an event to document event handlers.
pub(crate) struct DispatchResult {
    pub prevent_default: bool,
    pub stop_propagation: bool,
}

impl DocumentEventHandlers {
    pub(crate) fn add(
        &self,
        element: SpecialElement,
        kind: DomEventKind,
        handler: impl FnMut(Event<PlatformEventData>) + 'static,
    ) -> DocumentEventHandlerId {
        let key = self
            .handlers
            .borrow_mut()
            .insert(DocumentEventHandlerInner {
                element,
                kind,
                handler: Box::new(handler),
            });
        DocumentEventHandlerId(key.data().as_ffi())
    }

    pub(crate) fn remove(&self, id: DocumentEventHandlerId) {
        let key = DefaultKey::from(KeyData::from_ffi(id.0));
        self.handlers.borrow_mut().remove(key);
    }

    /// Whether any handler is registered for the given event kind.
    /// Used to preserve the "skip event kinds with no handlers" fast path.
    pub(crate) fn has_handlers(&self, kind: DomEventKind) -> bool {
        self.handlers.borrow().values().any(|h| h.kind == kind)
    }

    /// Dispatch an event to all handlers registered for `element` + `kind`
    pub(crate) fn dispatch(
        &self,
        element: SpecialElement,
        kind: DomEventKind,
        data: Rc<dyn Any>,
        bubbles: bool,
    ) -> DispatchResult {
        // `data` is always the `Rc<PlatformEventData>` built by DioxusEventHandler
        let data: Rc<PlatformEventData> = data.downcast().unwrap();
        let mut result = DispatchResult {
            prevent_default: false,
            stop_propagation: false,
        };
        for (_, entry) in self.handlers.borrow_mut().iter_mut() {
            if entry.element != element || entry.kind != kind {
                continue;
            }
            let event = Event::new(Rc::clone(&data), bubbles);
            (entry.handler)(event.clone());
            result.prevent_default |= !event.default_action_enabled();
            result.stop_propagation |= !event.propagates();
        }
        result
    }
}
