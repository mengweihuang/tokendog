//! Per-node resource holding allocated pages with reference counting.
//!
//! NodeResource uses `Cell<i32>` for the ref count to allow interior mutability
//! during lock/unlock operations (which happen during NodeRef creation/destruction).

use std::cell::Cell;
use std::ptr::NonNull;

use crate::resource::allocator::OwnedPages;
use crate::resource::types::NodeKey;

use super::resource_manager::ResourceManager;

/// Per-node page resource with RAII page ownership and reference counting.
///
/// The ref_count tracks how many NodeRefs currently hold a lock on this node.
/// When ref_count drops from 1 to 0, the resource manager is notified that
/// this node is now evictable.
pub struct NodeResource {
    /// Pages owned by this resource.
    pub pages: OwnedPages,
    /// Reference count (number of active NodeRefs locking this node).
    pub ref_count: Cell<i32>,
    /// Back-pointer to the resource manager (for eviction notification).
    pub evict_notifier: Option<NonNull<ResourceManager>>,
    /// The tree node this resource is attached to.
    pub owner_node: Option<NodeKey>,
}

impl NodeResource {
    /// Create a new NodeResource with the given pages.
    pub fn new(pages: OwnedPages) -> Self {
        Self {
            pages,
            ref_count: Cell::new(0),
            evict_notifier: None,
            owner_node: None,
        }
    }

    /// Create with an initial ref count.
    pub fn with_ref_count(pages: OwnedPages, ref_count: i32) -> Self {
        Self {
            pages,
            ref_count: Cell::new(ref_count),
            evict_notifier: None,
            owner_node: None,
        }
    }

    /// Lock this resource (increment ref count).
    pub fn lock(&self) {
        assert!(self.ref_count.get() >= 0, "ref_count must be >= 0");
        self.ref_count.set(self.ref_count.get() + 1);
    }

    /// Unlock this resource (decrement ref count).
    /// If ref_count drops to 0 and an evict notifier is bound, notify the manager.
    pub fn unlock(&self) {
        assert!(self.ref_count.get() >= 1, "ref_count must be >= 1");
        self.ref_count.set(self.ref_count.get() - 1);
        if self.ref_count.get() == 0 {
            if let Some(mgr) = self.evict_notifier {
                if let Some(node) = self.owner_node {
                    // Safety: the resource manager outlives this resource.
                    unsafe { mgr.as_ptr().as_mut().unwrap() }.on_node_evictable(node);
                }
            }
        }
    }

    /// Whether this resource is empty.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Number of pages.
    pub fn num_pages(&self) -> i32 {
        self.pages.size()
    }

    /// Whether the resource is evictable (ref count is 0).
    pub fn is_evictable(&self) -> bool {
        self.ref_count.get() == 0
    }

    /// Page IDs.
    pub fn pages_ids(&self) -> Vec<i32> {
        self.pages.ids().to_vec()
    }

    /// Take the pages out of the resource.
    pub fn take_pages(&mut self) -> OwnedPages {
        std::mem::replace(&mut self.pages, OwnedPages::empty())
    }

    /// Split first N pages off.
    pub fn split_first(&mut self, n: i32) -> OwnedPages {
        self.pages.take_first(n)
    }

    /// Bind the eviction notifier callback.
    ///
    /// # Safety
    ///
    /// The ResourceManager must outlive this NodeResource.
    pub fn bind_evictable_notifier(&mut self, mgr: &mut ResourceManager, node: NodeKey) {
        self.evict_notifier = NonNull::new(mgr as *mut ResourceManager);
        self.owner_node = Some(node);
    }

    /// Clear the eviction notifier.
    pub fn clear_evictable_notifier(&mut self) {
        self.evict_notifier = None;
        self.owner_node = None;
    }
}
