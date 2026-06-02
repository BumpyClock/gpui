use super::ROOT_NODE_ID;
use crate::{Bounds, FocusId, GlobalElementId, Pixels};
use accesskit::{NodeId, TreeUpdate};
use collections::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

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
    pub(super) nodes: A11yNodeBuilderPrepaintSnapshot,
    pub(super) node_ids: FxHashMap<GlobalElementId, NodeId>,
    pub(super) visited_global_ids: FxHashSet<GlobalElementId>,
    pub(super) next_node_id: u64,
    pub(super) focus_ids: FxHashMap<NodeId, FocusId>,
    pub(super) node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
}

pub(super) struct A11yNodeBuilderPrepaintSnapshot {
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
    pub(super) fn new() -> Self {
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

    pub(super) fn begin_frame(&mut self) {
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

    pub(super) fn prepaint_snapshot(&self) -> A11yNodeBuilderPrepaintSnapshot {
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

    pub(super) fn restore_prepaint_snapshot(&mut self, snapshot: A11yNodeBuilderPrepaintSnapshot) {
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

    pub(super) fn finalize(&mut self) -> TreeUpdate {
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
#[path = "builder_tests.rs"]
mod tests;
