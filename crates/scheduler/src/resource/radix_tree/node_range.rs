//! Tree traversal iterators: leaf-to-root and root-to-leaf.

use crate::resource::types::NodeKey;

use super::tree_node::Tree;

/// Iterate from a leaf node up to (but not including) the root.
pub fn leaf_to_root(tree: &Tree, leaf: NodeKey) -> LeafToRootIter<'_> {
    LeafToRootIter {
        tree,
        current: Some(leaf),
    }
}

pub struct LeafToRootIter<'a> {
    tree: &'a Tree,
    current: Option<NodeKey>,
}

impl<'a> Iterator for LeafToRootIter<'a> {
    type Item = NodeKey;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current?;
        let node = self.tree.get(current)?;

        if node.is_root() {
            self.current = None;
            return None;
        }

        self.current = node.parent;
        Some(current)
    }
}

/// Collect the path from root to leaf.
pub fn root_to_leaf(tree: &Tree, leaf: NodeKey) -> Vec<NodeKey> {
    let mut path: Vec<NodeKey> = leaf_to_root(tree, leaf).collect();
    path.reverse();
    path
}

/// Collect all device page IDs from root to the given node.
pub fn device_pages_from_root(tree: &Tree, node_key: NodeKey) -> Vec<i32> {
    let path = root_to_leaf(tree, node_key);
    let mut pages = Vec::new();
    for key in path {
        if let Some(node) = tree.get(key) {
            if let Some(ref resource) = node.device_resource {
                pages.extend_from_slice(resource.pages.ids());
            }
        }
    }
    pages
}

/// Collect all host page IDs from root to the given node.
pub fn host_pages_from_root(tree: &Tree, node_key: NodeKey) -> Vec<i32> {
    let path = root_to_leaf(tree, node_key);
    let mut pages = Vec::new();
    for key in path {
        if let Some(node) = tree.get(key) {
            if let Some(ref resource) = node.host_resource {
                pages.extend_from_slice(resource.pages.ids());
            }
        }
    }
    pages
}
