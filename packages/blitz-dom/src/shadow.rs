//! Shadow DOM "flattened tree" and slot distribution.
//!
//! The flattened tree is the tree that is actually laid out and painted. It is
//! derived from the regular DOM tree by:
//!   - replacing a shadow host's light-DOM children with its shadow root's
//!     children, and
//!   - replacing each `<slot>` element with the light-DOM nodes assigned to it
//!     (falling back to the slot's own children when nothing is assigned).
//!
//! The result is cached on each node in [`Node::flattened_children`] and
//! consulted by box construction, inline layout and painting via
//! [`Node::layout_dom_children`].

use markup5ever::local_name;
use selectors::matching::QuirksMode;
use style::stylesheets::{CustomMediaMap, Origin};
use style::stylist::CascadeDataDifference;

use crate::BaseDocument;
use crate::node::ShadowStyleData;

impl BaseDocument {
    /// Recompute the flattened tree (shadow-root composition and `<slot>`
    /// distribution) for every shadow host in the document.
    ///
    /// This is cheap when there are no shadow hosts and is run once per
    /// `resolve`.
    pub(crate) fn compute_flattened_trees(&mut self) {
        if self.shadow_host_nodes.is_empty() {
            return;
        }

        let host_ids: Vec<usize> = self.shadow_host_nodes.iter().copied().collect();
        for host_id in host_ids {
            self.compute_flattened_tree_for_host(host_id);
        }
    }

    fn compute_flattened_tree_for_host(&mut self, host_id: usize) {
        let Some(shadow_root_id) = self
            .get_node(host_id)
            .and_then(|node| node.shadow_root_id())
        else {
            return;
        };

        // 1. Reset any previously-computed flattened state for this host's
        //    shadow tree and light children.
        self.nodes[host_id].flattened_children = None;
        self.clear_flattened_in_subtree(shadow_root_id);
        let light_children = self.nodes[host_id].children.clone();
        for &child_id in &light_children {
            if let Some(el) = self.nodes[child_id].element_data_mut() {
                el.assigned_slot = None;
            }
        }

        // 2. The host's flattened children are the shadow root's children.
        let shadow_children = self.nodes[shadow_root_id].children.clone();
        self.nodes[host_id].flattened_children = Some(shadow_children);

        // 3. Discover slots within the shadow tree.
        let mut default_slot: Option<usize> = None;
        let mut named_slots: Vec<(String, usize)> = Vec::new();
        self.collect_slots(shadow_root_id, &mut default_slot, &mut named_slots);

        // If there are no slots at all, there is nothing to distribute.
        if default_slot.is_none() && named_slots.is_empty() {
            return;
        }

        // 4. Assign each light-DOM child to a slot.
        let mut assignments: Vec<(usize, Vec<usize>)> = Vec::new();
        if let Some(slot_id) = default_slot {
            assignments.push((slot_id, Vec::new()));
        }
        for (_, slot_id) in &named_slots {
            assignments.push((*slot_id, Vec::new()));
        }

        for &child_id in &light_children {
            let slot_name = self.nodes[child_id]
                .element_data()
                .and_then(|el| el.attr(local_name!("slot")))
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());

            let target_slot = match slot_name {
                Some(name) => named_slots
                    .iter()
                    .find(|(slot_name, _)| *slot_name == name)
                    .map(|(_, id)| *id)
                    .or(default_slot),
                None => default_slot,
            };

            if let Some(slot_id) = target_slot {
                if let Some(el) = self.nodes[child_id].element_data_mut() {
                    el.assigned_slot = Some(slot_id);
                }
                if let Some((_, list)) = assignments.iter_mut().find(|(id, _)| *id == slot_id) {
                    list.push(child_id);
                }
            }
        }

        // 5. Apply assignments to slots. A slot with no assigned nodes falls
        //    back to rendering its own children (so `flattened_children` is left
        //    as None).
        for (slot_id, assigned) in assignments {
            if !assigned.is_empty() {
                self.nodes[slot_id].flattened_children = Some(assigned);
            }
        }
    }

    /// Recursively clear `flattened_children` on a shadow subtree, so stale slot
    /// assignments don't persist across recomputes. Does not descend into
    /// nested shadow roots.
    fn clear_flattened_in_subtree(&mut self, node_id: usize) {
        let children = self.nodes[node_id].children.clone();
        self.nodes[node_id].flattened_children = None;
        for child_id in children {
            self.clear_flattened_in_subtree(child_id);
        }
    }

    /// Walk the shadow subtree collecting `<slot>` elements. The first unnamed
    /// slot becomes the default slot; the first slot with a given `name`
    /// attribute wins for that name. Does not descend into nested shadow hosts'
    /// shadow trees.
    fn collect_slots(
        &self,
        node_id: usize,
        default_slot: &mut Option<usize>,
        named_slots: &mut Vec<(String, usize)>,
    ) {
        let node = &self.nodes[node_id];
        for &child_id in &node.children {
            let child = &self.nodes[child_id];
            if let Some(el) = child.element_data() {
                if el.name.local == local_name!("slot") {
                    match el.attr(local_name!("name")).filter(|s| !s.is_empty()) {
                        Some(name) => {
                            if !named_slots.iter().any(|(n, _)| n == name) {
                                named_slots.push((name.to_string(), child_id));
                            }
                        }
                        None => {
                            if default_slot.is_none() {
                                *default_slot = Some(child_id);
                            }
                        }
                    }
                }
            }
            // Recurse, but don't descend into nested shadow hosts' shadow trees
            // (those slots belong to the nested host).
            self.collect_slots(child_id, default_slot, named_slots);
        }
    }

    /// (Re)build the scoped author [`CascadeData`](style::stylist::CascadeData)
    /// for every shadow root whose cache has been invalidated. Run before style
    /// resolution so Stylo can apply encapsulated rules (including `:host`).
    pub(crate) fn rebuild_shadow_cascade_data(&mut self) {
        if self.shadow_host_nodes.is_empty() {
            return;
        }

        let host_ids: Vec<usize> = self.shadow_host_nodes.iter().copied().collect();
        for host_id in host_ids {
            let Some(shadow_root_id) = self
                .get_node(host_id)
                .and_then(|node| node.shadow_root_id())
            else {
                continue;
            };

            let needs_rebuild = self.nodes[shadow_root_id]
                .shadow_root_data()
                .map(|data| data.style_data.is_none())
                .unwrap_or(false);
            if needs_rebuild {
                self.build_shadow_cascade_data(shadow_root_id);
            }
        }
    }

    fn build_shadow_cascade_data(&mut self, shadow_root_id: usize) {
        // Collect <style> elements within the shadow tree, in document order.
        let mut style_nodes = Vec::new();
        self.collect_shadow_style_nodes(shadow_root_id, &mut style_nodes);

        let guard = self.guard.clone();
        let custom_media = CustomMediaMap::default();

        let mut style_data = ShadowStyleData::new();
        for node_id in style_nodes {
            let css = self.nodes[node_id].text_content();
            let css = html_escape::decode_html_entities(&css);
            let sheet = self.make_stylesheet(&css, Origin::Author);
            style_data
                .stylesheets
                .append_stylesheet(None, &custom_media, sheet, &guard.read());
        }
        style_data.stylesheets.force_dirty();

        // Flush the stylesheet set and rebuild the cascade data from it.
        let (flusher, _invalidations) = style_data.stylesheets.flush();
        let mut difference = CascadeDataDifference::default();
        let read_guard = guard.read();
        let _ = style_data.cascade_data.rebuild(
            self.stylist.device(),
            QuirksMode::NoQuirks,
            flusher.sheets,
            &read_guard,
            &mut difference,
        );
        drop(read_guard);

        if let Some(data) = self.nodes[shadow_root_id].shadow_root_data_mut() {
            data.style_data = Some(Box::new(style_data));
        }
    }

    /// Collect `<style>` element node ids within a shadow tree (descendants of
    /// the shadow root via real children, not descending into nested shadow
    /// hosts' shadow trees).
    fn collect_shadow_style_nodes(&self, node_id: usize, out: &mut Vec<usize>) {
        let children = self.nodes[node_id].children.clone();
        for child_id in children {
            let child = &self.nodes[child_id];
            if child
                .element_data()
                .is_some_and(|el| el.name.local == local_name!("style"))
            {
                out.push(child_id);
            }
            self.collect_shadow_style_nodes(child_id, out);
        }
    }

    /// Invalidate the scoped cascade data for the shadow root containing
    /// `node_id` (if any), so it is rebuilt on the next resolve.
    pub(crate) fn invalidate_containing_shadow_styles(&mut self, node_id: usize) {
        if let Some(shadow_root_id) = self.nodes[node_id].containing_shadow_root().map(|n| n.id) {
            if let Some(data) = self.nodes[shadow_root_id].shadow_root_data_mut() {
                data.style_data = None;
            }
        }
    }
}
