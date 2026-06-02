//! Accessibility support, provided by [AccessKit][accesskit].
//!
//! User-facing guide-level docs live in [`crate::_accessibility`].

use crate::{App, Bounds, FocusId, GlobalElementId, Pixels, Window};
use accesskit::{Action, NodeId, TreeUpdate};
use collections::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// The fixed AccessKit node ID used for the root of every window's a11y tree.
pub(crate) const ROOT_NODE_ID: NodeId = NodeId(0);

/// A listener for an accessibility action on a specific node.
pub(crate) type A11yActionListener =
    Box<dyn FnMut(Option<&accesskit::ActionData>, &mut Window, &mut App) + 'static>;

/// Per-window accessibility state.
pub(crate) struct A11y {
    force_disabled: bool,
    active_flag: Arc<AtomicBool>,
    active_this_frame: bool,
    node_ids: FxHashMap<GlobalElementId, NodeId>,
    visited_global_ids: FxHashSet<GlobalElementId>,
    next_node_id: u64,
    pub(crate) nodes: A11yNodeBuilder,
    pub(crate) focus_ids: FxHashMap<NodeId, FocusId>,
    pub(crate) node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    pub(crate) action_listeners: FxHashMap<NodeId, Vec<(Action, A11yActionListener)>>,
}

impl A11y {
    pub(crate) fn new(active_flag: Arc<AtomicBool>, force_disabled: bool) -> Self {
        Self {
            force_disabled,
            active_flag,
            active_this_frame: false,
            node_ids: FxHashMap::default(),
            visited_global_ids: FxHashSet::default(),
            next_node_id: ROOT_NODE_ID.0 + 1,
            nodes: A11yNodeBuilder::new(),
            focus_ids: FxHashMap::default(),
            node_bounds: FxHashMap::default(),
            action_listeners: FxHashMap::default(),
        }
    }

    pub(crate) fn sync_active_flag(&mut self) {
        self.active_this_frame = !self.force_disabled && self.active_flag.load(Ordering::SeqCst);
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active_this_frame
    }

    /// Force accessibility active/inactive for tests without going through a platform adapter.
    #[cfg(test)]
    pub(crate) fn set_active_for_test(&mut self, active: bool) {
        self.active_flag.store(active, Ordering::SeqCst);
        self.sync_active_flag();
    }

    pub(crate) fn begin_frame(&mut self) {
        self.focus_ids.clear();
        self.node_bounds.clear();
        self.action_listeners.clear();
        self.visited_global_ids.clear();
        self.nodes.begin_frame();
    }

    pub(crate) fn node_id_for(&mut self, global_id: &GlobalElementId) -> NodeId {
        self.visited_global_ids.insert(global_id.clone());

        if let Some(node_id) = self.node_ids.get(global_id) {
            return *node_id;
        }

        let node_id = NodeId(self.next_node_id);
        debug_assert_ne!(node_id, ROOT_NODE_ID);
        self.next_node_id += 1;
        self.node_ids.insert(global_id.clone(), node_id);
        node_id
    }

    pub(crate) fn node_id_for_existing(&self, global_id: &GlobalElementId) -> Option<NodeId> {
        self.node_ids.get(global_id).copied()
    }

    pub(crate) fn end_frame(&mut self) -> TreeUpdate {
        let update = self.nodes.finalize();
        let live_node_ids = update
            .nodes
            .iter()
            .map(|(node_id, _)| *node_id)
            .collect::<FxHashSet<_>>();

        self.node_ids
            .retain(|global_id, _| self.visited_global_ids.contains(global_id));
        self.focus_ids
            .retain(|node_id, _| live_node_ids.contains(node_id));
        self.node_bounds
            .retain(|node_id, _| live_node_ids.contains(node_id));
        self.action_listeners
            .retain(|node_id, _| live_node_ids.contains(node_id));

        update
    }

    pub(crate) fn prepaint_snapshot(&self) -> A11yPrepaintSnapshot {
        A11yPrepaintSnapshot {
            nodes: self.nodes.prepaint_snapshot(),
            node_ids: self.node_ids.clone(),
            visited_global_ids: self.visited_global_ids.clone(),
            next_node_id: self.next_node_id,
            focus_ids: self.focus_ids.clone(),
            node_bounds: self.node_bounds.clone(),
        }
    }

    pub(crate) fn restore_prepaint_snapshot(&mut self, snapshot: A11yPrepaintSnapshot) {
        self.nodes.restore_prepaint_snapshot(snapshot.nodes);
        self.node_ids = snapshot.node_ids;
        self.visited_global_ids = snapshot.visited_global_ids;
        self.next_node_id = snapshot.next_node_id;
        self.focus_ids = snapshot.focus_ids;
        self.node_bounds = snapshot.node_bounds;
        // `action_listeners` are populated during paint, while prepaint transactions only
        // roll back prepaint side effects.
    }
}

pub(crate) struct A11yNodeBuilder {
    ids_stack: SmallVec<[NodeId; 16]>,
    nodes_stack: SmallVec<[accesskit::Node; 16]>,
    suppression_stack: SmallVec<[bool; 16]>,
    ambient_suppression_depth: usize,
    all_nodes: Vec<(NodeId, accesskit::Node)>,
    seen_ids: FxHashSet<NodeId>,
    focus: NodeId,
    #[cfg(debug_assertions)]
    has_set_focus: bool,
}

pub(crate) struct A11yPrepaintSnapshot {
    nodes: A11yNodeBuilderPrepaintSnapshot,
    node_ids: FxHashMap<GlobalElementId, NodeId>,
    visited_global_ids: FxHashSet<GlobalElementId>,
    next_node_id: u64,
    focus_ids: FxHashMap<NodeId, FocusId>,
    node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
}

struct A11yNodeBuilderPrepaintSnapshot {
    ids_stack: SmallVec<[NodeId; 16]>,
    nodes_stack: SmallVec<[accesskit::Node; 16]>,
    suppression_stack: SmallVec<[bool; 16]>,
    ambient_suppression_depth: usize,
    all_nodes: Vec<(NodeId, accesskit::Node)>,
    seen_ids: FxHashSet<NodeId>,
    focus: NodeId,
    #[cfg(debug_assertions)]
    has_set_focus: bool,
}

impl A11yNodeBuilder {
    fn new() -> Self {
        Self {
            ids_stack: SmallVec::new(),
            nodes_stack: SmallVec::new(),
            suppression_stack: SmallVec::new(),
            ambient_suppression_depth: 0,
            all_nodes: Vec::new(),
            seen_ids: FxHashSet::default(),
            focus: ROOT_NODE_ID,
            #[cfg(debug_assertions)]
            has_set_focus: false,
        }
    }

    pub(crate) fn push(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        debug_assert!(!self.ids_stack.is_empty(), "push called before begin_frame");

        if self.is_suppressed() {
            return false;
        }

        if !self.seen_ids.insert(id) {
            debug_assert!(
                false,
                "duplicate a11y node id: {id:?}; release builds discard this node"
            );
            return false;
        }

        self.ids_stack.push(id);
        self.nodes_stack.push(node);
        self.suppression_stack.push(false);
        true
    }

    pub(crate) fn pop(&mut self) {
        debug_assert!(self.ids_stack.len() > 1, "pop would remove the root node");

        self.pop_any();
    }

    fn begin_frame(&mut self) {
        self.all_nodes.clear();
        self.ids_stack.clear();
        self.nodes_stack.clear();
        self.suppression_stack.clear();
        self.ambient_suppression_depth = 0;
        self.seen_ids.clear();
        self.seen_ids.insert(ROOT_NODE_ID);
        #[cfg(debug_assertions)]
        {
            self.has_set_focus = false;
        }

        self.ids_stack.push(ROOT_NODE_ID);
        self.nodes_stack
            .push(accesskit::Node::new(accesskit::Role::Window));
        self.suppression_stack.push(false);
        self.focus = ROOT_NODE_ID;
    }

    #[cfg(test)]
    fn has_node(&self, id: NodeId) -> bool {
        id == ROOT_NODE_ID || self.seen_ids.contains(&id)
    }

    pub(crate) fn has_current_node(&self, id: NodeId) -> bool {
        self.ids_stack.last().copied() == Some(id) && !self.is_suppressed()
    }

    pub(crate) fn set_focus(&mut self, id: NodeId) {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                !self.has_set_focus,
                "set_focus called more than once in a single frame"
            );
            self.has_set_focus = true;
        }
        self.focus = id;
    }

    fn prepaint_snapshot(&self) -> A11yNodeBuilderPrepaintSnapshot {
        A11yNodeBuilderPrepaintSnapshot {
            ids_stack: self.ids_stack.clone(),
            nodes_stack: self.nodes_stack.clone(),
            suppression_stack: self.suppression_stack.clone(),
            ambient_suppression_depth: self.ambient_suppression_depth,
            all_nodes: self.all_nodes.clone(),
            seen_ids: self.seen_ids.clone(),
            focus: self.focus,
            #[cfg(debug_assertions)]
            has_set_focus: self.has_set_focus,
        }
    }

    fn restore_prepaint_snapshot(&mut self, snapshot: A11yNodeBuilderPrepaintSnapshot) {
        self.ids_stack = snapshot.ids_stack;
        self.nodes_stack = snapshot.nodes_stack;
        self.suppression_stack = snapshot.suppression_stack;
        self.ambient_suppression_depth = snapshot.ambient_suppression_depth;
        self.all_nodes = snapshot.all_nodes;
        self.seen_ids = snapshot.seen_ids;
        self.focus = snapshot.focus;
        #[cfg(debug_assertions)]
        {
            self.has_set_focus = snapshot.has_set_focus;
        }
    }

    #[allow(dead_code)]
    pub(crate) fn update_current_node_bounds(
        &mut self,
        id: NodeId,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
    ) -> bool {
        let Some(node) = self.current_node_mut(id) else {
            return false;
        };

        let scale = scale_factor;
        node.set_bounds(accesskit::Rect {
            x0: (bounds.origin.x.0 * scale) as f64,
            y0: (bounds.origin.y.0 * scale) as f64,
            x1: ((bounds.origin.x.0 + bounds.size.width.0) * scale) as f64,
            y1: ((bounds.origin.y.0 + bounds.size.height.0) * scale) as f64,
        });
        true
    }

    #[allow(dead_code)]
    pub(crate) fn suppress_current_node(&mut self, id: NodeId) -> bool {
        if self.ids_stack.len() <= 1 {
            debug_assert!(false, "cannot suppress the root a11y node");
            return false;
        }

        if self.ids_stack.last().copied() != Some(id) {
            return false;
        }

        let Some(suppressed) = self.suppression_stack.last_mut() else {
            return false;
        };

        if *suppressed {
            return false;
        }

        *suppressed = true;
        self.prune_emitted_subtree(id);
        true
    }

    #[allow(dead_code)]
    pub(crate) fn begin_suppressing_descendants(&mut self) {
        self.ambient_suppression_depth += 1;
    }

    #[allow(dead_code)]
    pub(crate) fn end_suppressing_descendants(&mut self) {
        debug_assert!(
            self.ambient_suppression_depth > 0,
            "end_suppressing_descendants called without matching begin"
        );
        self.ambient_suppression_depth = self.ambient_suppression_depth.saturating_sub(1);
    }

    fn finalize(&mut self) -> TreeUpdate {
        debug_assert_eq!(self.ids_stack.len(), 1);
        debug_assert_eq!(self.ids_stack[0], ROOT_NODE_ID);
        debug_assert_eq!(self.ambient_suppression_depth, 0);

        if self.ids_stack.len() != 1 {
            log::error!(
                "a11y: stack imbalance at end of frame: expected 1 (root), got {}",
                self.ids_stack.len()
            );
        }
        if self.ambient_suppression_depth != 0 {
            log::error!(
                "a11y: ambient suppression imbalance at end of frame: got {}",
                self.ambient_suppression_depth
            );
            self.ambient_suppression_depth = 0;
        }

        while !self.ids_stack.is_empty() {
            self.pop_any();
        }

        let update = TreeUpdate {
            nodes: std::mem::take(&mut self.all_nodes),
            tree: Some(accesskit::Tree::new(ROOT_NODE_ID)),
            tree_id: accesskit::TreeId::ROOT,
            focus: self.focus,
        };

        Self::repair_tree_update(update)
    }

    #[allow(dead_code)]
    fn current_node_mut(&mut self, id: NodeId) -> Option<&mut accesskit::Node> {
        if self.ids_stack.len() <= 1
            || self.ids_stack.last().copied() != Some(id)
            || self.suppression_stack.last().copied().unwrap_or(true)
        {
            None
        } else {
            self.nodes_stack.last_mut()
        }
    }

    fn is_suppressed(&self) -> bool {
        self.ambient_suppression_depth > 0
            || self.suppression_stack.last().copied().unwrap_or_default()
    }

    fn prune_emitted_subtree(&mut self, id: NodeId) {
        let mut pruned_ids = FxHashSet::default();
        pruned_ids.insert(id);

        if let Some(current_node) = self.nodes_stack.last() {
            let mut pending = current_node.children().to_vec();
            if !pending.is_empty() {
                let emitted_nodes_by_id = self
                    .all_nodes
                    .iter()
                    .map(|(node_id, node)| (*node_id, node))
                    .collect::<FxHashMap<_, _>>();
                while let Some(child_id) = pending.pop() {
                    if !pruned_ids.insert(child_id) {
                        continue;
                    }

                    if let Some(child_node) = emitted_nodes_by_id.get(&child_id) {
                        pending.extend(child_node.children().iter().copied());
                    }
                }
            }
        }

        for node_id in &pruned_ids {
            self.seen_ids.remove(node_id);
        }

        if pruned_ids.contains(&self.focus) {
            self.focus = ROOT_NODE_ID;
        }

        self.all_nodes
            .retain(|(node_id, _)| !pruned_ids.contains(node_id));

        for (_, node) in &mut self.all_nodes {
            Self::remove_child_refs(node, &pruned_ids);
        }
        for node in &mut self.nodes_stack {
            Self::remove_child_refs(node, &pruned_ids);
        }
    }

    fn remove_child_refs(node: &mut accesskit::Node, removed_ids: &FxHashSet<NodeId>) {
        if node
            .children()
            .iter()
            .any(|child_id| removed_ids.contains(child_id))
        {
            let children = node
                .children()
                .iter()
                .copied()
                .filter(|child_id| !removed_ids.contains(child_id))
                .collect::<Vec<_>>();
            node.set_children(children);
        }
    }

    fn pop_any(&mut self) {
        if let (Some(id), Some(node), Some(suppressed)) = (
            self.ids_stack.pop(),
            self.nodes_stack.pop(),
            self.suppression_stack.pop(),
        ) {
            if suppressed {
                return;
            }

            if let (Some(parent), Some(parent_suppressed)) =
                (self.nodes_stack.last_mut(), self.suppression_stack.last())
            {
                if !*parent_suppressed {
                    parent.push_child(id);
                }
            }
            self.all_nodes.push((id, node));
        }
    }

    fn repair_tree_update(mut update: TreeUpdate) -> TreeUpdate {
        let node_ids: FxHashSet<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();

        if !node_ids.contains(&update.focus) {
            log::error!(
                "a11y: focused node {:?} is not in the tree ({} nodes); falling back to root",
                update.focus,
                update.nodes.len()
            );
            update.focus = ROOT_NODE_ID;
        }

        for (id, node) in &mut update.nodes {
            let has_invalid_child = node
                .children()
                .iter()
                .any(|child_id| !node_ids.contains(child_id));
            if has_invalid_child {
                let children = node.children();
                let invalid_count = children
                    .iter()
                    .filter(|child_id| !node_ids.contains(child_id))
                    .count();
                log::error!(
                    "a11y: node {:?} references {} children not present in the tree; stripping invalid child references",
                    id,
                    invalid_count
                );
                let valid = children
                    .iter()
                    .copied()
                    .filter(|child_id| node_ids.contains(child_id))
                    .collect::<Vec<_>>();
                node.set_children(valid);
            }
        }

        update
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ElementId, point, px, size};
    use slotmap::KeyData;
    use std::sync::{Arc, atomic::AtomicBool};

    fn global_id(id: impl Into<ElementId>) -> GlobalElementId {
        GlobalElementId(Arc::from([id.into()]))
    }

    fn node<'a>(update: &'a TreeUpdate, id: NodeId) -> &'a accesskit::Node {
        update
            .nodes
            .iter()
            .find_map(|(node_id, node)| (*node_id == id).then_some(node))
            .unwrap()
    }

    fn has_update_node(update: &TreeUpdate, id: NodeId) -> bool {
        update.nodes.iter().any(|(node_id, _)| *node_id == id)
    }

    #[test]
    fn preserves_unsuppressed_tree_shape() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
        builder.pop();
        builder.pop();

        let update = builder.finalize();
        assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
        assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(2)]);
        assert_eq!(node(&update, NodeId(2)).children(), &[]);
    }

    #[test]
    fn updates_current_non_root_node_bounds() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        assert!(builder.update_current_node_bounds(
            NodeId(1),
            Bounds::new(point(px(2.), px(3.)), size(px(4.), px(5.))),
            2.,
        ));
        builder.pop();

        let update = builder.finalize();
        assert_eq!(
            node(&update, NodeId(1)).bounds(),
            Some(accesskit::Rect {
                x0: 4.,
                y0: 6.,
                x1: 12.,
                y1: 16.,
            })
        );
    }

    #[test]
    fn suppresses_current_node_before_finalization() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        assert!(builder.suppress_current_node(NodeId(1)));
        assert!(!builder.has_node(NodeId(1)));
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.nodes.len(), 1);
        assert_eq!(node(&update, ROOT_NODE_ID).children(), &[]);
    }

    #[test]
    fn skips_descendants_while_current_node_is_suppressed() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        assert!(builder.suppress_current_node(NodeId(1)));
        assert!(!builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
        builder.pop();

        assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.nodes.len(), 2);
        assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(2)]);
        assert_eq!(node(&update, NodeId(2)).children(), &[]);
    }

    #[test]
    fn ambient_descendant_suppression_under_root_skips_child_push_without_suppressing_root() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        builder.begin_suppressing_descendants();
        assert!(!builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        builder.end_suppressing_descendants();

        assert!(builder.has_node(ROOT_NODE_ID));
        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.nodes.len(), 2);
        assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
    }

    #[test]
    fn id_specific_update_requires_current_node() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        assert!(!builder.update_current_node_bounds(
            NodeId(2),
            Bounds::new(point(px(2.), px(3.)), size(px(4.), px(5.))),
            2.,
        ));
        builder.pop();

        let update = builder.finalize();
        assert_eq!(node(&update, NodeId(1)).bounds(), None);
    }

    #[test]
    fn id_specific_suppress_requires_current_node() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        assert!(!builder.suppress_current_node(NodeId(2)));
        assert!(builder.has_node(NodeId(1)));
        assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
        builder.pop();
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.nodes.len(), 3);
        assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
        assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(2)]);
    }

    #[test]
    fn suppressing_focused_current_node_removes_focus_from_tree() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        builder.set_focus(NodeId(1));
        assert!(builder.suppress_current_node(NodeId(1)));
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.focus, ROOT_NODE_ID);
        assert_eq!(update.nodes.len(), 1);
    }

    #[test]
    fn suppressing_current_node_prunes_already_emitted_descendants() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
        assert!(builder.push(NodeId(3), accesskit::Node::new(accesskit::Role::TextInput)));
        builder.set_focus(NodeId(3));
        builder.pop();
        builder.pop();

        assert!(builder.suppress_current_node(NodeId(1)));
        assert!(!builder.has_node(NodeId(1)));
        assert!(!builder.has_node(NodeId(2)));
        assert!(!builder.has_node(NodeId(3)));
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.nodes.len(), 1);
        assert_eq!(update.focus, ROOT_NODE_ID);
        assert_eq!(node(&update, ROOT_NODE_ID).children(), &[]);
        assert!(!has_update_node(&update, NodeId(1)));
        assert!(!has_update_node(&update, NodeId(2)));
        assert!(!has_update_node(&update, NodeId(3)));
    }

    #[test]
    fn parent_children_do_not_reference_suppressed_descendants() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
        assert!(builder.suppress_current_node(NodeId(2)));
        builder.pop();

        assert!(builder.push(NodeId(3), accesskit::Node::new(accesskit::Role::Label)));
        builder.pop();
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.nodes.len(), 3);
        assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
        assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(3)]);
    }

    #[test]
    fn restores_prepaint_snapshot_for_builder_state() {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame();

        assert!(builder.push(NodeId(1), accesskit::Node::new(accesskit::Role::Button)));
        let snapshot = builder.prepaint_snapshot();

        assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
        builder.pop();
        builder.begin_suppressing_descendants();
        assert!(!builder.push(NodeId(3), accesskit::Node::new(accesskit::Role::Label)));
        builder.set_focus(NodeId(1));

        builder.restore_prepaint_snapshot(snapshot);

        assert!(builder.push(NodeId(2), accesskit::Node::new(accesskit::Role::Label)));
        builder.set_focus(NodeId(2));
        builder.pop();
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.nodes.len(), 3);
        assert_eq!(update.focus, NodeId(2));
        assert_eq!(node(&update, ROOT_NODE_ID).children(), &[NodeId(1)]);
        assert_eq!(node(&update, NodeId(1)).children(), &[NodeId(2)]);
    }

    #[test]
    fn restores_prepaint_snapshot_for_a11y_maps() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false);
        a11y.begin_frame();

        let focus_id = FocusId::from(KeyData::from_ffi(1));
        let original_bounds = Bounds::new(point(px(1.), px(2.)), size(px(3.), px(4.)));
        a11y.focus_ids.insert(NodeId(1), focus_id);
        a11y.node_bounds.insert(NodeId(1), original_bounds);
        let snapshot = a11y.prepaint_snapshot();

        a11y.focus_ids.remove(&NodeId(1));
        a11y.focus_ids
            .insert(NodeId(2), FocusId::from(KeyData::from_ffi(2)));
        a11y.node_bounds.insert(
            NodeId(1),
            Bounds::new(point(px(5.), px(6.)), size(px(7.), px(8.))),
        );
        a11y.node_bounds.insert(
            NodeId(2),
            Bounds::new(point(px(9.), px(10.)), size(px(11.), px(12.))),
        );

        a11y.restore_prepaint_snapshot(snapshot);

        assert_eq!(a11y.focus_ids.len(), 1);
        assert_eq!(a11y.focus_ids.get(&NodeId(1)), Some(&focus_id));
        assert_eq!(a11y.node_bounds.len(), 1);
        assert_eq!(a11y.node_bounds.get(&NodeId(1)), Some(&original_bounds));
    }

    #[test]
    fn allocates_stable_unique_node_ids_for_global_element_ids() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false);
        let first_id = global_id("first");
        let second_id = global_id("second");

        let first_node_id = a11y.node_id_for(&first_id);
        let second_node_id = a11y.node_id_for(&second_id);
        a11y.begin_frame();

        assert_ne!(first_node_id, ROOT_NODE_ID);
        assert_ne!(second_node_id, ROOT_NODE_ID);
        assert_ne!(first_node_id, second_node_id);
        assert_eq!(a11y.node_id_for(&first_id), first_node_id);
        assert_eq!(a11y.node_id_for_existing(&second_id), Some(second_node_id));
        assert_eq!(a11y.node_id_for_existing(&global_id("missing")), None);
    }

    #[test]
    fn retains_visited_suppressed_node_id_and_sweeps_omitted_node_id() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false);
        let hidden_id = global_id("hidden");

        a11y.begin_frame();
        let hidden_node_id = a11y.node_id_for(&hidden_id);
        assert!(a11y.nodes.push(
            hidden_node_id,
            accesskit::Node::new(accesskit::Role::Button)
        ));
        assert!(a11y.nodes.suppress_current_node(hidden_node_id));
        a11y.nodes.pop();
        let update = a11y.end_frame();

        assert!(!has_update_node(&update, hidden_node_id));
        assert_eq!(a11y.node_id_for_existing(&hidden_id), Some(hidden_node_id));

        a11y.begin_frame();
        a11y.end_frame();

        assert_eq!(a11y.node_id_for_existing(&hidden_id), None);
    }

    #[test]
    fn sweeps_per_node_maps_by_live_emitted_nodes_after_end_frame() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false);
        let live_bounds = Bounds::new(point(px(1.), px(2.)), size(px(3.), px(4.)));
        let stale_bounds = Bounds::new(point(px(5.), px(6.)), size(px(7.), px(8.)));
        let live_focus = FocusId::from(KeyData::from_ffi(1));
        let stale_focus = FocusId::from(KeyData::from_ffi(2));

        a11y.begin_frame();
        assert!(
            a11y.nodes
                .push(NodeId(1), accesskit::Node::new(accesskit::Role::Button))
        );
        a11y.nodes.pop();
        a11y.focus_ids.insert(NodeId(1), live_focus);
        a11y.focus_ids.insert(NodeId(2), stale_focus);
        a11y.node_bounds.insert(NodeId(1), live_bounds);
        a11y.node_bounds.insert(NodeId(2), stale_bounds);
        a11y.action_listeners
            .insert(NodeId(1), vec![(Action::Click, Box::new(|_, _, _| {}))]);
        a11y.action_listeners
            .insert(NodeId(2), vec![(Action::Focus, Box::new(|_, _, _| {}))]);

        let update = a11y.end_frame();

        assert!(has_update_node(&update, NodeId(1)));
        assert!(!has_update_node(&update, NodeId(2)));
        assert_eq!(a11y.focus_ids.len(), 1);
        assert_eq!(a11y.focus_ids.get(&NodeId(1)), Some(&live_focus));
        assert_eq!(a11y.node_bounds.len(), 1);
        assert_eq!(a11y.node_bounds.get(&NodeId(1)), Some(&live_bounds));
        assert_eq!(a11y.action_listeners.len(), 1);
        assert!(a11y.action_listeners.contains_key(&NodeId(1)));
    }

    #[test]
    fn restores_prepaint_snapshot_for_node_id_allocator() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false);
        let accepted_id = global_id("accepted");
        let rejected_id = global_id("rejected");
        let next_id = global_id("next");

        let accepted_node_id = a11y.node_id_for(&accepted_id);
        let snapshot = a11y.prepaint_snapshot();
        let rejected_node_id = a11y.node_id_for(&rejected_id);

        a11y.restore_prepaint_snapshot(snapshot);
        let next_node_id = a11y.node_id_for(&next_id);

        assert_eq!(
            a11y.node_id_for_existing(&accepted_id),
            Some(accepted_node_id)
        );
        assert_eq!(a11y.node_id_for_existing(&rejected_id), None);
        assert_eq!(next_node_id, rejected_node_id);
    }

    #[test]
    fn restores_prepaint_snapshot_for_visited_globals() {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(false)), false);
        let accepted_id = global_id("accepted");
        let rejected_id = global_id("rejected");

        a11y.begin_frame();
        let accepted_node_id = a11y.node_id_for(&accepted_id);
        let snapshot = a11y.prepaint_snapshot();
        let rejected_node_id = a11y.node_id_for(&rejected_id);
        assert!(a11y.nodes.push(
            accepted_node_id,
            accesskit::Node::new(accesskit::Role::Button)
        ));
        a11y.nodes.pop();

        a11y.restore_prepaint_snapshot(snapshot);
        let update = a11y.end_frame();

        assert!(!has_update_node(&update, accepted_node_id));
        assert!(!has_update_node(&update, rejected_node_id));
        assert_eq!(
            a11y.node_id_for_existing(&accepted_id),
            Some(accepted_node_id)
        );
        assert_eq!(a11y.node_id_for_existing(&rejected_id), None);
    }
}
