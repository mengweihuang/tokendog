//! Radix tree: prefix tree for token sequences, walked page-by-page.

use std::time::Instant;

use crate::resource::types::{MatchResult, NodeKey, WalkResult};
use crate::resource::types;

use super::tree_node::{Tree, TreeNode};

/// Radix tree for prefix-cached token sequences.
///
/// Tokens are stored in nodes and walked page-by-page. When a mismatch
/// occurs mid-node, the node is split into a prefix and suffix.
pub struct RadixTree {
    /// Page size for paging.
    pub page_size: i32,
    /// The arena holding all tree nodes.
    pub tree: Tree,
}

impl RadixTree {
    /// Create a new RadixTree with the given page size.
    pub fn new(page_size: i32) -> Self {
        Self {
            page_size,
            tree: Tree::new(),
        }
    }

    /// Walk down the tree matching tokens page by page.
    ///
    /// Returns the terminal node and remaining (unmatched) tokens.
    pub fn walk_down_util_mismatch(
        &mut self,
        tokens: &[i32],
        access_time: Instant,
        start_node: Option<NodeKey>,
    ) -> WalkResult {
        let current_key = start_node.unwrap_or(self.tree.root);

        let mut result = WalkResult {
            terminal: current_key,
            remaining_offset: 0,
            match_result: MatchResult::default(),
        };

        let mut device_alive = true;
        let mut host_alive = true;
        let mut remaining = tokens;
        let mut current = current_key;

        while remaining.len() >= self.page_size as usize {
            let walk_key: Vec<i32> = remaining[..self.page_size as usize].to_vec();

            // Find child by first-page key
            let child_key = {
                let cur_node = self.tree.get(current).unwrap();
                cur_node.children.get(&walk_key).copied()
            };

            let child_key = match child_key {
                Some(k) => k,
                None => break,
            };

            // Calculate how many pages match
            let matched_pages = {
                let child = self.tree.get(child_key).unwrap();
                calc_matched_pages(&child.tokens, remaining, self.page_size)
            };

            if matched_pages == 0 {
                break;
            }

            let child = self.tree.get(child_key).unwrap();
            let child_total_pages = child.tokens.len() / self.page_size as usize;

            let actual_child = if matched_pages != child_total_pages {
                // Partial match — need to split
                if child.has_paged_cache_snapshot() {
                    break; // Refuse to split snapshot-bearing nodes
                }
                let split = self.split_child(current, &walk_key, matched_pages);
                split.prefix
            } else {
                child_key
            };

            // Touch the node
            if let Some(node) = self.tree.get_mut(actual_child) {
                node.touch(access_time);
            }

            // Update match tiers
            let node = self.tree.get(actual_child).unwrap();
            if device_alive {
                if node.on_device() {
                    result.match_result.device.last_node = Some(actual_child);
                    result.match_result.device.page_size = self.page_size;
                } else {
                    device_alive = false;
                }
            }
            if host_alive {
                if node.on_host() {
                    result.match_result.host.last_node = Some(actual_child);
                    result.match_result.host.page_size = self.page_size;
                } else {
                    host_alive = false;
                }
            }

            current = actual_child;
            result.terminal = actual_child;
            let advance = matched_pages * self.page_size as usize;
            remaining = &remaining[advance..];
            result.remaining_offset = tokens.len() - remaining.len();
        }

        result
    }

    /// Split a child node at `prefix_pages` pages from the start.
    fn split_child(
        &mut self,
        parent_key: NodeKey,
        child_key: &[i32],
        prefix_pages: usize,
    ) -> types::SplitResult {
        // Remove old child from parent
        let old_child_key = {
            let parent = self.tree.get_mut(parent_key).unwrap();
            parent.remove_child(child_key)
        };

        let old_child_key = old_child_key.expect("child must exist");

        // Create a new prefix node
        let mut prefix_node = TreeNode::new(vec![], Instant::now());
        let suffix_first_page: Vec<i32> = {
            let old_child = self.tree.get_mut(old_child_key).unwrap();
            old_child.split_self_into(&mut prefix_node, prefix_pages, self.page_size);
            old_child.tokens[..self.page_size as usize].to_vec()
        };

        let prefix_key = self.tree.insert(prefix_node);
        let prefix_first_page: Vec<i32> = {
            let prefix = self.tree.get(prefix_key).unwrap();
            prefix.tokens[..self.page_size as usize].to_vec()
        };

        // Update prefix's parent to parent_key and depth FIRST
        // (suffix depth depends on prefix's corrected depth)
        let _prefix_tokens_len = {
            let parent_node = self.tree.get(parent_key).unwrap();
            let parent_depth = parent_node.depth_in_tokens;
            let prefix_node = self.tree.get(prefix_key).unwrap();
            let ptl = prefix_node.tokens.len();
            if let Some(pr) = self.tree.get_mut(prefix_key) {
                pr.parent = Some(parent_key);
                // depth_in_tokens = parent's end-of-node depth + prefix's own token count
                pr.depth_in_tokens = parent_depth + ptl;
            }
            ptl
        };

        // Update old child's parent to prefix and depth
        let prefix_depth = {
            let prefix_node = self.tree.get(prefix_key).unwrap();
            prefix_node.depth_in_tokens
        };
        {
            if let Some(old_child) = self.tree.get_mut(old_child_key) {
                old_child.parent = Some(prefix_key);
                // suffix depth = prefix's end-of-node depth + suffix's own token count
                old_child.depth_in_tokens = prefix_depth + old_child.tokens.len();
            }
        }

        // Add old child to prefix's children
        {
            let prefix = self.tree.get_mut(prefix_key).unwrap();
            prefix.children.insert(suffix_first_page, old_child_key);
        }

        // Add prefix to parent's children
        {
            let parent = self.tree.get_mut(parent_key).unwrap();
            parent.children.insert(prefix_first_page, prefix_key);
        }

        types::SplitResult {
            parent: parent_key,
            prefix: prefix_key,
            suffix: old_child_key,
        }
    }

    /// Prune empty nodes upward from the given node.
    pub fn prune_empty_by_node(&mut self, node_key: NodeKey) -> Option<NodeKey> {
        let mut current = node_key;

        loop {
            let should_break = {
                let node = self.tree.get(current)?;
                node.is_root()
                    || node.num_children() != 0
                    || node.on_device()
                    || node.on_host()
            };

            if should_break {
                break;
            }

            let (parent_key, first_page) = {
                let node = self.tree.get(current).unwrap();
                let parent = node.parent?;
                let fp: Vec<i32> = node.tokens[..self.page_size as usize].to_vec();
                (parent, fp)
            };

            {
                let parent = self.tree.get_mut(parent_key).unwrap();
                parent.remove_child(&first_page);
            }

            self.tree.remove(current);
            current = parent_key;
        }

        Some(current)
    }

    /// Find or create the node at `depth_in_tokens` on the descendant's root path.
    pub fn split_at(&mut self, descendant: NodeKey, depth_in_tokens: i32) -> Option<NodeKey> {
        if depth_in_tokens <= 0 || depth_in_tokens % self.page_size != 0 {
            return None;
        }

        let desc_depth = self.tree.get(descendant)?.depth_in_tokens;
        if depth_in_tokens as usize > desc_depth {
            return None;
        }

        let mut current = descendant;
        while !self.tree.get(current)?.is_root() {
            let node = self.tree.get(current)?;
            let this_depth = node.depth_in_tokens as i32;
            let parent_depth = this_depth - node.tokens.len() as i32;

            if depth_in_tokens == this_depth {
                return Some(current);
            }

            if depth_in_tokens > parent_depth && depth_in_tokens < this_depth {
                // Refuse to split a snapshot-bearing node
                if node.has_paged_cache_snapshot() {
                    return None;
                }

                let parent_key = node.parent?;
                let child_key: Vec<i32> =
                    node.tokens[..self.page_size as usize].to_vec();
                let prefix_pages =
                    (depth_in_tokens - parent_depth) as usize / self.page_size as usize;

                let split = self.split_child(parent_key, &child_key, prefix_pages);
                return Some(split.prefix);
            }

            current = node.parent?;
        }

        None
    }

    /// Get a reference to a tree node by key.
    pub fn get_node(&self, key: NodeKey) -> Option<&TreeNode> {
        self.tree.get(key)
    }

    /// Get a mutable reference to a tree node by key.
    pub fn get_node_mut(&mut self, key: NodeKey) -> Option<&mut TreeNode> {
        self.tree.get_mut(key)
    }

    /// Get the root node key.
    pub fn root(&self) -> NodeKey {
        self.tree.root
    }
}

/// Calculate how many pages of the child node match the remaining tokens.
fn calc_matched_pages(node_tokens: &[i32], remaining: &[i32], page_size: i32) -> usize {
    let comparable = node_tokens.len().min(remaining.len());
    if comparable == 0 {
        return 0;
    }

    let matched_tokens = node_tokens[..comparable]
        .iter()
        .zip(remaining[..comparable].iter())
        .take_while(|(a, b)| a == b)
        .count();

    matched_tokens / page_size as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calc_matched_pages_full() {
        let node = vec![1, 2, 3, 4];
        let remaining = vec![1, 2, 3, 4, 5, 6];
        assert_eq!(calc_matched_pages(&node, &remaining, 2), 2);
    }

    #[test]
    fn test_calc_matched_pages_partial() {
        let node = vec![1, 2, 5, 6];
        let remaining = vec![1, 2, 3, 4];
        assert_eq!(calc_matched_pages(&node, &remaining, 2), 1); // first page matches, second doesn't
    }

    #[test]
    fn test_calc_matched_pages_none() {
        let node = vec![5, 6];
        let remaining = vec![1, 2, 3, 4];
        assert_eq!(calc_matched_pages(&node, &remaining, 2), 0);
    }
}
