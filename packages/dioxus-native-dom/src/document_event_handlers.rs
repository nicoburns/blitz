use blitz_traits::events::DomEventKind;
use dioxus_core::Event;
use dioxus_html::PlatformEventData;
use rustc_hash::FxHashMap;
use std::any::Any;
use std::cell::{Cell, RefCell};
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
pub struct DocumentEventHandlerId {
    pub(crate) node_id: usize,
    pub(crate) kind: DomEventKind,
    /// Uniquely identifies the listener within its `(node_id, kind)` bucket
    pub(crate) serial: u64,
}

impl DocumentEventHandlerId {
    /// Unregister this event listener.
    ///
    /// Must be called from within a Dioxus scope.
    pub fn remove(&self) {
        let handlers: Rc<DocumentEventHandlers> = dioxus_core::consume_context();
        handlers.remove(*self);
    }
}

struct ListenerEntry {
    serial: u64,
    #[allow(clippy::type_complexity)]
    handler: Box<dyn FnMut(Event<PlatformEventData>) + 'static>,
}

/// Event listeners registered imperatively against DOM nodes (the equivalent of
/// `addEventListener` in the browser) rather than declaratively via `rsx!`.
pub(crate) struct DocumentEventHandlers {
    /// Listeners keyed by (node id, event kind), giving O(1) lookup of the listeners
    /// for a given node when dispatching events. Listeners registered for the same
    /// key are stored (and run) in registration order.
    handlers: RefCell<FxHashMap<(usize, DomEventKind), Vec<ListenerEntry>>>,
    /// Count of listeners for each event kind (indexed by `DomEventKind` discriminant).
    /// Allows O(1) "is there any listener for this event kind?" checks.
    kind_counts: RefCell<[u32; 64]>,
    /// Monotonically increasing serial number used to uniquely identify listeners
    next_serial: Cell<u64>,
}

impl Default for DocumentEventHandlers {
    fn default() -> Self {
        Self {
            handlers: RefCell::new(FxHashMap::default()),
            kind_counts: RefCell::new([0; 64]),
            next_serial: Cell::new(0),
        }
    }
}

impl fmt::Debug for DocumentEventHandlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentEventHandlers")
            .field("len", &self.len())
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
        let serial = self.next_serial.get();
        self.next_serial.set(serial + 1);
        self.handlers
            .borrow_mut()
            .entry((node_id, kind))
            .or_default()
            .push(ListenerEntry {
                serial,
                handler: Box::new(handler),
            });
        self.kind_counts.borrow_mut()[kind.discriminant() as usize] += 1;
        DocumentEventHandlerId {
            node_id,
            kind,
            serial,
        }
    }

    pub(crate) fn remove(&self, id: DocumentEventHandlerId) {
        let mut handlers = self.handlers.borrow_mut();
        let Some(entries) = handlers.get_mut(&(id.node_id, id.kind)) else {
            return;
        };
        let old_len = entries.len();
        entries.retain(|entry| entry.serial != id.serial);
        let removed_count = old_len - entries.len();
        if entries.is_empty() {
            handlers.remove(&(id.node_id, id.kind));
        }
        self.kind_counts.borrow_mut()[id.kind.discriminant() as usize] -= removed_count as u32;
    }

    /// Whether any listener is registered for the given event kind.
    /// Used to preserve the "skip event kinds with no handlers" fast path.
    pub(crate) fn has_handlers(&self, kind: DomEventKind) -> bool {
        self.kind_counts.borrow()[kind.discriminant() as usize] > 0
    }

    /// Remove all listeners registered against node ids matching the `is_dropped`
    /// predicate. Called when nodes are dropped so that stale listeners cannot fire
    /// against an unrelated node which later reuses the same node id.
    pub(crate) fn remove_listeners_for_nodes(&self, is_dropped: impl Fn(usize) -> bool) {
        let mut kind_counts = self.kind_counts.borrow_mut();
        self.handlers
            .borrow_mut()
            .retain(|(node_id, kind), entries| {
                let keep = !is_dropped(*node_id);
                if !keep {
                    kind_counts[kind.discriminant() as usize] -= entries.len() as u32;
                }
                keep
            });
    }

    /// The total number of registered listeners
    pub(crate) fn len(&self) -> usize {
        self.handlers.borrow().values().map(Vec::len).sum()
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

        let mut handlers = self.handlers.borrow_mut();
        let Some(entries) = handlers.get_mut(&(node_id, kind)) else {
            return result;
        };

        // `data` is always the `Rc<PlatformEventData>` built by DioxusEventHandler
        let data: Rc<PlatformEventData> = data.downcast().unwrap();
        for entry in entries.iter_mut() {
            let event = Event::new(Rc::clone(&data), bubbles);
            (entry.handler)(event.clone());
            result.prevent_default |= !event.default_action_enabled();
            result.stop_propagation |= !event.propagates();
        }
        result
    }
}
