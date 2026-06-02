//! Paged-cache snapshot attached to tree nodes.

use std::collections::{BTreeMap, BTreeSet};

use crate::resource::allocator::OwnedPages;

/// Per-group snapshot held by a TreeNode.
/// RAII returns pages to the allocator when dropped.
pub struct PagedCacheGroupSnapshot {
    pub pages: OwnedPages,
    pub base_logical_page: i32,
    pub raw_token_cursor: i32,
    pub sliding: bool,
}

/// Paged-cache family (History, State).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PagedCacheGroupFamily {
    History,
    State,
}

/// Snapshot for a TreeNode at a history-aligned raw-token boundary.
/// Completeness is tracked per family.
pub struct PagedCacheSnapshot {
    /// Number of prefix tokens covered by this snapshot.
    pub prefix_len_tokens: i32,
    /// Per-group snapshot data.
    pub groups: BTreeMap<String, PagedCacheGroupSnapshot>,
    /// Which families are complete in this snapshot.
    pub complete_families: BTreeSet<PagedCacheGroupFamily>,
}

impl PagedCacheSnapshot {
    pub fn is_complete_for(&self, family: PagedCacheGroupFamily) -> bool {
        self.complete_families.contains(&family)
    }
}
