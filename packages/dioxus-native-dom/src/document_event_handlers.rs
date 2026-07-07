use blitz_traits::events::DomEventKind;
use dioxus_core::Event;
use dioxus_html::PlatformEventData;
use slotmap::{DefaultKey, Key, KeyData, SlotMap};
use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Context providing the node ids of the pre-created elements which are not
/// rendered by Dioxus (for compatibility with the web backend), so that event
/// listeners can be registered against them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SpecialElementIds {
    /// The node id of the root `<html>` element
    pub(crate) html: usize,
    /// The node id of the `<body>` element
    pub(crate) body: usize,
}

/// The unique identifier of a document event listener. This can be used to later remove the listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentEventHandlerId(pub(crate) u64);

impl DocumentEventHandlerId {
    /// Unregister this event listener.
    ///
    /// Must be called from within a Dioxus scope.
    pub fn remove(&self) {
        let handlers: Rc<DocumentEventHandlers> = dioxus_core::consume_context();
        handlers.remove(*self);
    }
}

struct DocumentEventHandlerInner {
    node_id: usize,
    kind: DomEventKind,
    #[allow(clippy::type_complexity)]
    handler: Box<dyn FnMut(Event<PlatformEventData>) + 'static>,
}

/// Event listeners registered imperatively against DOM nodes (the equivalent of
/// `addEventListener` in the browser) rather than declaratively via `rsx!`.
#[derive(Default)]
pub(crate) struct DocumentEventHandlers {
    handlers: RefCell<SlotMap<DefaultKey, DocumentEventHandlerInner>>,
}

impl fmt::Debug for DocumentEventHandlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentEventHandlers")
            .field("len", &self.handlers.borrow().len())
            .finish()
    }
}

/// The aggregate result of dispatching an event to document event listeners.
pub(crate) struct DispatchResult {
    pub prevent_default: bool,
    pub stop_propagation: bool,
}

impl DocumentEventHandlers {
    pub(crate) fn add(
        &self,
        node_id: usize,
        kind: DomEventKind,
        handler: impl FnMut(Event<PlatformEventData>) + 'static,
    ) -> DocumentEventHandlerId {
        let key = self
            .handlers
            .borrow_mut()
            .insert(DocumentEventHandlerInner {
                node_id,
                kind,
                handler: Box::new(handler),
            });
        DocumentEventHandlerId(key.data().as_ffi())
    }

    pub(crate) fn remove(&self, id: DocumentEventHandlerId) {
        let key = DefaultKey::from(KeyData::from_ffi(id.0));
        self.handlers.borrow_mut().remove(key);
    }

    /// Whether any listener is registered for the given event kind.
    /// Used to preserve the "skip event kinds with no handlers" fast path.
    pub(crate) fn has_handlers(&self, kind: DomEventKind) -> bool {
        self.handlers.borrow().values().any(|h| h.kind == kind)
    }

    /// Remove all listeners registered against node ids matching the `is_dropped`
    /// predicate. Called when nodes are dropped so that stale listeners cannot fire
    /// against an unrelated node which later reuses the same node id.
    pub(crate) fn remove_listeners_for_nodes(&self, is_dropped: impl Fn(usize) -> bool) {
        self.handlers
            .borrow_mut()
            .retain(|_, h| !is_dropped(h.node_id));
    }

    /// The total number of registered listeners
    pub(crate) fn len(&self) -> usize {
        self.handlers.borrow().len()
    }

    /// Dispatch an event to all listeners registered against the `node_id` chain node
    /// whose event kind matches `kind`
    pub(crate) fn dispatch(
        &self,
        node_id: usize,
        kind: DomEventKind,
        data: Rc<dyn Any>,
        bubbles: bool,
    ) -> DispatchResult {
        let mut result = DispatchResult {
            prevent_default: false,
            stop_propagation: false,
        };
        if self.handlers.borrow().is_empty() {
            return result;
        }

        // `data` is always the `Rc<PlatformEventData>` built by DioxusEventHandler
        let data: Rc<PlatformEventData> = data.downcast().unwrap();
        for (_, entry) in self.handlers.borrow_mut().iter_mut() {
            if entry.node_id != node_id || entry.kind != kind {
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
