//! Enable the dom to participate in styling by servo
//!

use std::sync::atomic::Ordering;

use crate::node::Node;

use crate::node::NodeData;
use atomic_refcell::{AtomicRef, AtomicRefMut};
use html5ever::{local_name, LocalName, Namespace};
use selectors::{
    matching::{ElementSelectorFlags, MatchingContext, VisitedHandlingMode},
    sink::Push,
    Element, OpaqueElement,
};
use style::values::computed::Float;
// use slab::Slab;
use style::values::specified::box_::DisplayOutside;
use style::CaseSensitivityExt;
use style::{
    animation::DocumentAnimationSet,
    context::{
        QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters,
        SharedStyleContext, StyleContext,
    },
    dom::{LayoutIterator, NodeInfo, OpaqueNode, TDocument, TElement, TNode, TShadowRoot},
    global_style_data::GLOBAL_STYLE_DATA,
    properties::{
        style_structs::{Box as BoxStyle, Position},
        PropertyDeclarationBlock,
    },
    selector_parser::{NonTSPseudoClass, SelectorImpl},
    servo_arc::{Arc, ArcBorrow},
    shared_lock::{Locked, SharedRwLock, StylesheetGuards},
    thread_state::ThreadState,
    traversal::{DomTraversal, PerLevelTraversalData},
    traversal_flags::TraversalFlags,
    values::{AtomIdent, GenericAtomIdent},
    Atom,
};
use style_traits::dom::ElementState;
use taffy::prelude::Style;

use super::stylo_to_taffy;

impl crate::document::Document {
    /// Walk the whole tree, converting styles to layout
    pub fn flush_styles_to_layout(&mut self, children: Vec<usize>) {
        // make a floating element
        for child in children.iter() {
            let (display, mut children) = {
                let node = self.nodes.get_mut(*child).unwrap();
                let stylo_element_data = node.stylo_element_data.borrow();
                let primary_styles = stylo_element_data
                    .as_ref()
                    .and_then(|data| data.styles.get_primary());

                let Some(style) = primary_styles else {
                    continue;
                };

                // if let Some(style) = data.styles.get_primary() {
                let margin = style.get_margin();
                let padding = style.get_padding();
                let border = style.get_border();
                let Position {
                    top,
                    right,
                    bottom,
                    left,

                    width,
                    min_width,
                    max_width,
                    height,
                    min_height,
                    max_height,
                    aspect_ratio,

                    box_sizing,
                    // z_index,
                    // order,
                    column_gap,
                    row_gap,

                    justify_content,
                    justify_items,
                    justify_self,
                    align_content,
                    align_items,
                    align_self,

                    flex_direction,
                    flex_wrap,
                    flex_basis,
                    flex_grow,
                    flex_shrink,

                    grid_auto_flow,

                    grid_template_columns,
                    grid_template_rows,
                    grid_auto_columns,
                    grid_auto_rows,

                    grid_column_start,
                    grid_column_end,
                    grid_row_start,
                    grid_row_end,
                    ..
                } = style.get_position();

                let BoxStyle {
                    _servo_top_layer,
                    _servo_overflow_clip_box,
                    display: stylo_display,
                    position: stylo_position,
                    float,
                    // clear,
                    // vertical_align,
                    overflow_x,
                    overflow_y,
                    // transform,
                    // rotate,
                    // scale,
                    // translate,
                    // perspective,
                    // perspective_origin,
                    // backface_visibility,
                    // transform_style,
                    // transform_origin,
                    // container_type,
                    // container_name,
                    // original_display,
                    ..
                }: &BoxStyle = style.get_box();

                // HACK: Emulate float with 'position: absolute'
                let mut position = stylo_to_taffy::position(*stylo_position);
                let mut inset = taffy::Rect {
                    left: stylo_to_taffy::length_percentage_auto(left),
                    right: stylo_to_taffy::length_percentage_auto(right),
                    top: stylo_to_taffy::length_percentage_auto(top),
                    bottom: stylo_to_taffy::length_percentage_auto(bottom),
                };
                if position == taffy::Position::Relative && *float != Float::None {
                    position = taffy::Position::Absolute;
                    if *float == Float::Right {
                        inset.left = taffy::LengthPercentageAuto::Auto;
                        inset.right = taffy::LengthPercentageAuto::Length(0.0);
                    } else {
                        inset.left = taffy::LengthPercentageAuto::Length(0.0);
                        inset.right = taffy::LengthPercentageAuto::Auto;
                    }
                }

                let display = stylo_to_taffy::display(*stylo_display);
                node.style = Style {
                    box_sizing: stylo_to_taffy::box_sizing(*box_sizing),
                    display,
                    position,
                    overflow: taffy::Point {
                        x: stylo_to_taffy::overflow(*overflow_x),
                        y: stylo_to_taffy::overflow(*overflow_y),
                    },

                    // TODO: we'll eventually want to support visible scrollbars
                    // But we really ought to implement "overflow: auto" first
                    scrollbar_width: 0.0,

                    size: taffy::Size {
                        width: stylo_to_taffy::dimension(width),
                        height: stylo_to_taffy::dimension(height),
                    },
                    min_size: taffy::Size {
                        width: stylo_to_taffy::dimension(min_width),
                        height: stylo_to_taffy::dimension(min_height),
                    },
                    max_size: taffy::Size {
                        width: stylo_to_taffy::max_size_dimension(max_width),
                        height: stylo_to_taffy::max_size_dimension(max_height),
                    },
                    aspect_ratio: stylo_to_taffy::aspect_ratio(*aspect_ratio),

                    margin: stylo_to_taffy::margin(margin),
                    padding: stylo_to_taffy::padding(padding),
                    border: stylo_to_taffy::border(border),
                    inset,

                    // Alignment properties
                    justify_content: stylo_to_taffy::content_alignment(justify_content.0),
                    justify_items: stylo_to_taffy::item_alignment(justify_items.computed.0),
                    justify_self: stylo_to_taffy::item_alignment((justify_self.0).0),
                    align_content: stylo_to_taffy::content_alignment(align_content.0),
                    align_items: stylo_to_taffy::item_alignment(align_items.0),
                    align_self: stylo_to_taffy::item_alignment((align_self.0).0),

                    // Gap
                    gap: taffy::Size {
                        width: stylo_to_taffy::gap(column_gap),
                        height: stylo_to_taffy::gap(row_gap),
                    },

                    // Flexbox properties
                    flex_direction: stylo_to_taffy::flex_direction(*flex_direction),
                    flex_wrap: stylo_to_taffy::flex_wrap(*flex_wrap),
                    flex_grow: flex_grow.0,
                    flex_shrink: flex_shrink.0,
                    flex_basis: stylo_to_taffy::flex_basis(flex_basis),

                    // CSS Grid properties
                    grid_auto_flow: stylo_to_taffy::grid_auto_flow(*grid_auto_flow),
                    grid_template_rows: stylo_to_taffy::grid_template_tracks(grid_template_rows),
                    grid_template_columns: stylo_to_taffy::grid_template_tracks(
                        grid_template_columns,
                    ),
                    grid_auto_rows: stylo_to_taffy::grid_auto_tracks(grid_auto_rows),
                    grid_auto_columns: stylo_to_taffy::grid_auto_tracks(grid_auto_columns),
                    grid_row: taffy::Line {
                        start: stylo_to_taffy::grid_line(grid_row_start),
                        end: stylo_to_taffy::grid_line(grid_row_end),
                    },
                    grid_column: taffy::Line {
                        start: stylo_to_taffy::grid_line(grid_column_start),
                        end: stylo_to_taffy::grid_line(grid_column_end),
                    },
                };

                node.display_outer = match stylo_display.outside() {
                    DisplayOutside::None => crate::node::DisplayOuter::None,
                    DisplayOutside::Inline => crate::node::DisplayOuter::Inline,
                    DisplayOutside::Block => crate::node::DisplayOuter::Block,
                    DisplayOutside::TableCaption => crate::node::DisplayOuter::Block,
                    DisplayOutside::InternalTable => crate::node::DisplayOuter::Block,
                };

                // Clear Taffy cache
                // TODO: smarter cache invalidation
                node.cache.clear();

                // would like to change this not require a clone, but requires some refactoring
                (
                    display,
                    node.layout_children.borrow().as_ref().unwrap().clone(),
                )
            };

            if matches!(display, taffy::Display::Flex | taffy::Display::Grid) {
                // Reorder the children based on their flex order
                // Would like to not have to
                children.sort_by(|left, right| {
                    let left_node = self.nodes.get(*left).unwrap();
                    let right_node = self.nodes.get(*right).unwrap();
                    left_node.order().cmp(&right_node.order())
                });

                // Mutate source child array
                *self
                    .nodes
                    .get_mut(*child)
                    .unwrap()
                    .layout_children
                    .borrow_mut() = Some(children.clone());
            }

            self.flush_styles_to_layout(children);
        }
    }

    pub fn resolve_stylist(&mut self) {
        style::thread_state::enter(ThreadState::LAYOUT);

        let guard = &self.guard;
        let guards = StylesheetGuards {
            author: &guard.read(),
            ua_or_user: &guard.read(),
        };

        let root = TDocument::as_node(&&self.nodes[0])
            .first_element_child()
            .unwrap()
            .as_element()
            .unwrap();

        self.stylist
            .flush(&guards, Some(root), Some(&self.snapshots));

        // Build the style context used by the style traversal
        let context = SharedStyleContext {
            traversal_flags: TraversalFlags::empty(),
            stylist: &self.stylist,
            options: GLOBAL_STYLE_DATA.options.clone(),
            guards,
            visited_styles_enabled: false,
            animations: DocumentAnimationSet::default().clone(),
            current_time_for_animations: 0.0,
            snapshot_map: &self.snapshots,
            registered_speculative_painters: &RegisteredPaintersImpl,
        };

        // components/layout_2020/lib.rs:983
        let root = self.root_element();
        // dbg!(root);
        let token = RecalcStyle::pre_traverse(root, &context);

        if token.should_traverse() {
            // Style the elements, resolving their data
            let traverser = RecalcStyle::new(context);
            style::driver::traverse_dom(&traverser, token, None);
        }

        style::thread_state::exit(ThreadState::LAYOUT);
    }
}

/// A handle to a node that Servo's style traits are implemented against
///
/// Since BlitzNodes are not persistent (IE we don't keep the pointers around between frames), we choose to just implement
/// the tree structure in the nodes themselves, and temporarily give out pointers during the layout phase.
type BlitzNode<'a> = &'a Node;

impl<'a> TDocument for BlitzNode<'a> {
    type ConcreteNode = BlitzNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn is_html_document(&self) -> bool {
        true
    }

    fn quirks_mode(&self) -> QuirksMode {
        QuirksMode::NoQuirks
    }

    fn shared_lock(&self) -> &SharedRwLock {
        &self.guard
    }
}

impl<'a> NodeInfo for BlitzNode<'a> {
    fn is_element(&self) -> bool {
        Node::is_element(self)
    }

    fn is_text_node(&self) -> bool {
        Node::is_text_node(self)
    }
}

impl<'a> TShadowRoot for BlitzNode<'a> {
    type ConcreteNode = BlitzNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn host(&self) -> <Self::ConcreteNode as TNode>::ConcreteElement {
        todo!("Shadow roots not implemented")
    }

    fn style_data<'b>(&self) -> Option<&'b style::stylist::CascadeData>
    where
        Self: 'b,
    {
        todo!("Shadow roots not implemented")
    }
}

// components/styleaapper.rs:
impl<'a> TNode for BlitzNode<'a> {
    type ConcreteElement = BlitzNode<'a>;
    type ConcreteDocument = BlitzNode<'a>;
    type ConcreteShadowRoot = BlitzNode<'a>;

    fn parent_node(&self) -> Option<Self> {
        self.parent.map(|id| self.with(id))
    }

    fn first_child(&self) -> Option<Self> {
        self.children.first().map(|id| self.with(*id))
    }

    fn last_child(&self) -> Option<Self> {
        self.children.last().map(|id| self.with(*id))
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.backward(1)
    }

    fn next_sibling(&self) -> Option<Self> {
        self.forward(1)
    }

    fn owner_doc(&self) -> Self::ConcreteDocument {
        self.with(1)
    }

    fn is_in_document(&self) -> bool {
        true
    }

    // I think this is the same as parent_node only in the cases when the direct parent is not a real element, forcing us
    // to travel upwards
    //
    // For the sake of this demo, we're just going to return the parent node ann
    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        self.parent_node().and_then(|node| node.as_element())
    }

    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.id)
    }

    fn debug_id(self) -> usize {
        self.id
    }

    fn as_element(&self) -> Option<Self::ConcreteElement> {
        match self.raw_dom_data {
            NodeData::Element { .. } => Some(self),
            _ => None,
        }
    }

    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        match self.raw_dom_data {
            NodeData::Document { .. } => Some(self),
            _ => None,
        }
    }

    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        todo!("Shadow roots aren't real, yet")
    }
}

impl<'a> selectors::Element for BlitzNode<'a> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> selectors::OpaqueElement {
        // FIXME: this is wrong in the case where pushing new elements casuses reallocations.
        // We should see if selectors will accept a PR that allows creation from a usize
        OpaqueElement::new(self)
    }

    fn parent_element(&self) -> Option<Self> {
        TElement::traversal_parent(self)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    // These methods are implemented naively since we only threaded real nodes and not fake nodes
    // we should try and use `find` instead of this foward/backward stuff since its ugly and slow
    fn prev_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(node) = self.backward(n) {
            if node.is_element() {
                return Some(node);
            }
            n += 1;
        }

        None
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(node) = self.forward(n) {
            if node.is_element() {
                return Some(node);
            }
            n += 1;
        }

        None
    }

    fn first_element_child(&self) -> Option<Self> {
        let mut children = self.dom_children();
        children.find(|child| child.is_element())
    }

    fn is_html_element_in_html_document(&self) -> bool {
        true // self.has_namespace(ns!(html))
    }

    fn has_local_name(&self, local_name: &LocalName) -> bool {
        self.raw_dom_data.is_element_with_tag_name(local_name)
    }

    fn has_namespace(&self, ns: &Namespace) -> bool {
        self.element_data().expect("Not an element").name.ns == *ns
    }

    fn is_same_type(&self, _other: &Self) -> bool {
        // FIXME: implementing this correctly currently triggers a debug_assert ("Invalid cache") in selectors
        //self.local_name() == other.local_name() && self.namespace() == other.namespace()
        false
    }

    fn attr_matches(
        &self,
        _ns: &selectors::attr::NamespaceConstraint<
            &<Self::Impl as selectors::SelectorImpl>::NamespaceUrl,
        >,
        local_name: &<Self::Impl as selectors::SelectorImpl>::LocalName,
        _operation: &selectors::attr::AttrSelectorOperation<
            &<Self::Impl as selectors::SelectorImpl>::AttrValue,
        >,
    ) -> bool {
        // println!("attr matches  {}", self.id);
        let mut has_attr = false;
        self.each_attr_name(|f| {
            if f.as_ref() == local_name.as_ref() {
                has_attr = true;
            }
        });
        has_attr
    }

    fn match_non_ts_pseudo_class(
        &self,
        psuedo_class: &<Self::Impl as selectors::SelectorImpl>::NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *psuedo_class {
            NonTSPseudoClass::Active => false,
            NonTSPseudoClass::AnyLink => false,
            NonTSPseudoClass::Checked => false,
            NonTSPseudoClass::Valid => false,
            NonTSPseudoClass::Invalid => false,
            NonTSPseudoClass::Defined => false,
            NonTSPseudoClass::Disabled => false,
            NonTSPseudoClass::Enabled => false,
            NonTSPseudoClass::Focus => false,
            NonTSPseudoClass::Fullscreen => false,
            NonTSPseudoClass::Hover => self.is_hovered,
            NonTSPseudoClass::Indeterminate => false,
            NonTSPseudoClass::Lang(_) => false,
            NonTSPseudoClass::CustomState(_) => false,
            NonTSPseudoClass::Link => false,
            NonTSPseudoClass::PlaceholderShown => false,
            NonTSPseudoClass::ReadWrite => false,
            NonTSPseudoClass::ReadOnly => false,
            NonTSPseudoClass::ServoNonZeroBorder => false,
            NonTSPseudoClass::Target => false,
            NonTSPseudoClass::Visited => false,
        }
    }

    fn match_pseudo_element(
        &self,
        _pe: &<Self::Impl as selectors::SelectorImpl>::PseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {
        // unimplemented!()
    }

    fn is_link(&self) -> bool {
        self.raw_dom_data
            .is_element_with_tag_name(&local_name!("a"))
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(
        &self,
        id: &<Self::Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: selectors::attr::CaseSensitivity,
    ) -> bool {
        self.element_data()
            .and_then(|data| data.id.as_ref())
            .map(|id_attr| case_sensitivity.eq_atom(id_attr, id))
            .unwrap_or(false)
    }

    fn has_class(
        &self,
        search_name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: selectors::attr::CaseSensitivity,
    ) -> bool {
        let class_attr = self.raw_dom_data.attr(local_name!("class"));
        if let Some(class_attr) = class_attr {
            // split the class attribute
            for pheme in class_attr.split_ascii_whitespace() {
                let atom = Atom::from(pheme);
                if case_sensitivity.eq_atom(&atom, search_name) {
                    return true;
                }
            }
        }

        false
    }

    fn imported_part(
        &self,
        _name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
    ) -> Option<<Self::Impl as selectors::SelectorImpl>::Identifier> {
        None
    }

    fn is_part(&self, _name: &<Self::Impl as selectors::SelectorImpl>::Identifier) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.dom_children().next().is_none()
    }

    fn is_root(&self) -> bool {
        self.parent_node()
            .and_then(|parent| parent.parent_node())
            .is_none()
    }

    fn has_custom_state(
        &self,
        _name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
    ) -> bool {
        false
    }

    fn add_element_unique_hashes(&self, _filter: &mut selectors::bloom::BloomFilter) -> bool {
        false
    }
}

impl<'a> TElement for BlitzNode<'a> {
    type ConcreteNode = BlitzNode<'a>;

    type TraversalChildrenIterator = Traverser<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn traversal_children(&self) -> style::dom::LayoutIterator<Self::TraversalChildrenIterator> {
        LayoutIterator(Traverser {
            // dom: self.tree(),
            parent: self,
            child_index: 0,
        })
    }

    fn is_html_element(&self) -> bool {
        self.is_element()
    }

    // not implemented.....
    fn is_mathml_element(&self) -> bool {
        false
    }

    // need to check the namespace
    fn is_svg_element(&self) -> bool {
        false
    }

    fn style_attribute(&self) -> Option<ArcBorrow<Locked<PropertyDeclarationBlock>>> {
        self.element_data()
            .expect("Not an element")
            .style_attribute
            .as_ref()
            .map(|f| f.borrow_arc())
    }

    fn animation_rule(
        &self,
        _: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn transition_rule(
        &self,
        _context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn state(&self) -> ElementState {
        self.element_state
    }

    fn has_part_attr(&self) -> bool {
        false
    }

    fn exports_any_part(&self) -> bool {
        false
    }

    fn id(&self) -> Option<&style::Atom> {
        self.element_data().and_then(|data| data.id.as_ref())
    }

    fn each_class<F>(&self, mut callback: F)
    where
        F: FnMut(&style::values::AtomIdent),
    {
        let class_attr = self.raw_dom_data.attr(local_name!("class"));
        if let Some(class_attr) = class_attr {
            // split the class attribute
            for pheme in class_attr.split_ascii_whitespace() {
                let atom = Atom::from(pheme); // interns the string
                callback(AtomIdent::cast(&atom));
            }
        }
    }

    fn each_attr_name<F>(&self, mut callback: F)
    where
        F: FnMut(&style::LocalName),
    {
        if let Some(attrs) = self.raw_dom_data.attrs() {
            for attr in attrs.iter() {
                callback(&GenericAtomIdent(attr.name.local.clone()));
            }
        }
    }

    fn has_dirty_descendants(&self) -> bool {
        true
    }

    fn has_snapshot(&self) -> bool {
        self.has_snapshot
    }

    fn handled_snapshot(&self) -> bool {
        self.snapshot_handled.load(Ordering::SeqCst)
    }

    unsafe fn set_handled_snapshot(&self) {
        self.snapshot_handled.store(true, Ordering::SeqCst);
    }

    unsafe fn set_dirty_descendants(&self) {}

    unsafe fn unset_dirty_descendants(&self) {}

    fn store_children_to_process(&self, _n: isize) {
        unimplemented!()
    }

    fn did_process_child(&self) -> isize {
        unimplemented!()
    }

    unsafe fn ensure_data(&self) -> AtomicRefMut<style::data::ElementData> {
        let mut stylo_data = self.stylo_element_data.borrow_mut();
        if stylo_data.is_none() {
            *stylo_data = Some(Default::default());
        }
        AtomicRefMut::map(stylo_data, |sd| sd.as_mut().unwrap())
    }

    unsafe fn clear_data(&self) {
        *self.stylo_element_data.borrow_mut() = None;
    }

    fn has_data(&self) -> bool {
        self.stylo_element_data.borrow().is_some()
    }

    fn borrow_data(&self) -> Option<AtomicRef<style::data::ElementData>> {
        let stylo_data = self.stylo_element_data.borrow();
        if stylo_data.is_some() {
            Some(AtomicRef::map(stylo_data, |sd| sd.as_ref().unwrap()))
        } else {
            None
        }
    }

    fn mutate_data(&self) -> Option<AtomicRefMut<style::data::ElementData>> {
        let stylo_data = self.stylo_element_data.borrow_mut();
        if stylo_data.is_some() {
            Some(AtomicRefMut::map(stylo_data, |sd| sd.as_mut().unwrap()))
        } else {
            None
        }
    }

    fn skip_item_display_fixup(&self) -> bool {
        false
    }

    fn may_have_animations(&self) -> bool {
        false
    }

    fn has_animations(&self, _context: &SharedStyleContext) -> bool {
        false
    }

    fn has_css_animations(
        &self,
        _context: &SharedStyleContext,
        _pseudo_element: Option<style::selector_parser::PseudoElement>,
    ) -> bool {
        false
    }

    fn has_css_transitions(
        &self,
        _context: &SharedStyleContext,
        _pseudo_element: Option<style::selector_parser::PseudoElement>,
    ) -> bool {
        false
    }

    fn shadow_root(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

    fn containing_shadow(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

    fn lang_attr(&self) -> Option<style::selector_parser::AttrValue> {
        None
    }

    fn match_element_lang(
        &self,
        _override_lang: Option<Option<style::selector_parser::AttrValue>>,
        _value: &style::selector_parser::Lang,
    ) -> bool {
        false
    }

    fn is_html_document_body_element(&self) -> bool {
        // Check node is a <body> element
        let is_body_element = self
            .raw_dom_data
            .is_element_with_tag_name(&local_name!("body"));

        // If it isn't then return early
        if !is_body_element {
            return false;
        }

        // If it is then check if it is a child of the root (<html>) element
        let root_node = &self.tree()[0];
        let root_element = TDocument::as_node(&root_node)
            .first_element_child()
            .unwrap();
        root_element.children.contains(&self.id)
    }

    fn synthesize_presentational_hints_for_legacy_attributes<V>(
        &self,
        _visited_handling: VisitedHandlingMode,
        _hints: &mut V,
    ) where
        V: Push<style::applicable_declarations::ApplicableDeclarationBlock>,
    {
    }

    fn local_name(&self) -> &LocalName {
        &self.element_data().expect("Not an element").name.local
    }

    fn namespace(&self) -> &Namespace {
        &self.element_data().expect("Not an element").name.ns
    }

    fn query_container_size(
        &self,
        _display: &style::values::specified::Display,
    ) -> euclid::default::Size2D<Option<app_units::Au>> {
        // FIXME: Implement container queries. For now this effectively disables them without panicking.
        Default::default()
    }

    fn each_custom_state<F>(&self, _callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        todo!()
    }

    fn has_selector_flags(&self, _flags: ElementSelectorFlags) -> bool {
        todo!()
    }

    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        todo!()
    }

    // fn update_animations(
    //     &self,
    //     before_change_style: Option<Arc<ComputedValues>>,
    //     tasks: style::context::UpdateAnimationsTasks,
    // ) {
    //     todo!()
    // }

    // fn process_post_animation(&self, tasks: style::context::PostAnimationTasks) {
    //     todo!()
    // }

    // fn needs_transitions_update(
    //     &self,
    //     before_change_style: &ComputedValues,
    //     after_change_style: &ComputedValues,
    // ) -> bool {
    //     todo!()
    // }
}

pub struct Traverser<'a> {
    // dom: &'a Slab<Node>,
    parent: BlitzNode<'a>,
    child_index: usize,
}

impl<'a> Iterator for Traverser<'a> {
    type Item = BlitzNode<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.parent.children.get(self.child_index)?;

        let node = self.parent.with(*node);

        self.child_index += 1;

        Some(node)
    }
}

impl std::hash::Hash for BlitzNode<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_usize(self.id)
    }
}

/// Handle custom painters like images for layouting
///
/// todo: actually implement this
pub struct RegisteredPaintersImpl;
impl RegisteredSpeculativePainters for RegisteredPaintersImpl {
    fn get(&self, _name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
        None
    }
}

use style::traversal::recalc_style_at;

pub struct RecalcStyle<'a> {
    context: SharedStyleContext<'a>,
}

impl<'a> RecalcStyle<'a> {
    pub fn new(context: SharedStyleContext<'a>) -> Self {
        RecalcStyle { context }
    }
}

#[allow(unsafe_code)]
impl<'a, 'dom, E> DomTraversal<E> for RecalcStyle<'a>
where
    E: TElement,
    E::ConcreteNode: 'dom,
{
    fn process_preorder<F: FnMut(E::ConcreteNode)>(
        &self,
        traversal_data: &PerLevelTraversalData,
        context: &mut StyleContext<E>,
        node: E::ConcreteNode,
        note_child: F,
    ) {
        // Don't process textnodees in this traversal
        if node.is_text_node() {
            return;
        }

        let el = node.as_element().unwrap();
        // let mut data = el.mutate_data().unwrap();
        let mut data = unsafe { el.ensure_data() };
        recalc_style_at(self, traversal_data, context, el, &mut data, note_child);

        // Gets set later on
        unsafe { el.unset_dirty_descendants() }
    }

    #[inline]
    fn needs_postorder_traversal() -> bool {
        false
    }

    fn process_postorder(&self, _style_context: &mut StyleContext<E>, _node: E::ConcreteNode) {
        panic!("this should never be called")
    }

    #[inline]
    fn shared_context(&self) -> &SharedStyleContext {
        &self.context
    }
}

#[test]
fn assert_size_of_equals() {
    // use std::mem;

    // fn assert_layout<E>() {
    //     assert_eq!(
    //         mem::size_of::<SharingCache<E>>(),
    //         mem::size_of::<TypelessSharingCache>()
    //     );
    //     assert_eq!(
    //         mem::align_of::<SharingCache<E>>(),
    //         mem::align_of::<TypelessSharingCache>()
    //     );
    // }

    // let size = mem::size_of::<StyleSharingCandidate<BlitzNode>>();
    // dbg!(size);
}

#[test]
fn parse_inline() {
    // let attrs = style::attr::AttrValue::from_serialized_tokenlist(
    //     r#"visibility: hidden; left: 1306.5px; top: 50px; display: none;"#.to_string(),
    // );

    // let val = CSSInlineStyleDeclaration();
}
