//! RAII wrapper for allocated pages.
//!
//! OwnedPages automatically returns pages to the allocator on drop.

use std::fmt;
use std::ptr::NonNull;

use super::page_allocator::PageAllocator;

/// RAII wrapper for a set of page IDs allocated from a PageAllocator.
///
/// When dropped, pages are returned to the allocator unless `detach()` was called.
pub struct OwnedPages {
    /// Pointer to the allocator (None if detached or empty).
    pub(crate) allocator: Option<NonNull<PageAllocator>>,
    /// Page IDs held by this instance.
    pub(crate) ids: Vec<i32>,
}

impl OwnedPages {
    /// Create a new OwnedPages with the given allocator and page IDs.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `allocator` outlives this OwnedPages.
    pub fn new(allocator: &mut PageAllocator, ids: Vec<i32>) -> Self {
        Self {
            allocator: NonNull::new(allocator as *mut PageAllocator),
            ids,
        }
    }

    /// Create an empty OwnedPages with no allocator.
    pub fn empty() -> Self {
        Self {
            allocator: None,
            ids: Vec::new(),
        }
    }

    /// The page IDs.
    pub fn ids(&self) -> &[i32] {
        &self.ids
    }

    /// Number of pages.
    pub fn size(&self) -> i32 {
        self.ids.len() as i32
    }

    /// Whether the container is empty.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Split off the first `n` pages into a new OwnedPages.
    /// Self keeps the remaining pages.
    pub fn take_first(&mut self, n: i32) -> OwnedPages {
        assert!(
            n >= 0 && n <= self.size(),
            "take_first: count {} out of range (size={})",
            n,
            self.size()
        );
        let n = n as usize;
        let taken: Vec<i32> = self.ids.drain(..n).collect();
        OwnedPages {
            allocator: self.allocator,
            ids: taken,
        }
    }

    /// Split off the last `n` pages into a new OwnedPages.
    /// Self keeps the remaining pages.
    pub fn take_last(&mut self, n: i32) -> OwnedPages {
        assert!(
            n >= 0 && n <= self.size(),
            "take_last: count {} out of range (size={})",
            n,
            self.size()
        );
        let n = n as usize;
        let split = self.ids.len() - n;
        let taken: Vec<i32> = self.ids.drain(split..).collect();
        OwnedPages {
            allocator: self.allocator,
            ids: taken,
        }
    }

    /// Absorb all pages from `other`. Both must share the same allocator.
    pub fn append(&mut self, other: &mut OwnedPages) {
        if other.ids.is_empty() {
            return;
        }
        if self.allocator.is_none() {
            self.allocator = other.allocator;
        } else {
            assert_eq!(
                self.allocator, other.allocator,
                "append: allocator mismatch"
            );
        }
        self.ids.append(&mut other.ids);
        other.allocator = None;
    }

    /// Drop ownership of specific page IDs without returning them to the allocator.
    pub fn release_ownership_by_id(&mut self, ids: &[i32]) {
        self.ids.retain(|id| !ids.contains(id));
    }

    /// Surrender all page IDs and allocator pointer without freeing.
    /// After this call, the OwnedPages is empty and will not deallocate on drop.
    pub fn detach(&mut self) -> Vec<i32> {
        self.allocator = None;
        std::mem::take(&mut self.ids)
    }
}

impl Drop for OwnedPages {
    fn drop(&mut self) {
        if let Some(alloc) = self.allocator {
            if !self.ids.is_empty() {
                // Safety: the allocator is guaranteed to outlive us by construction.
                unsafe { alloc.as_ptr().as_mut().unwrap() }.deallocate(&self.ids);
            }
        }
    }
}

impl fmt::Debug for OwnedPages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedPages")
            .field("ids", &self.ids)
            .field("has_allocator", &self.allocator.is_some())
            .finish()
    }
}

// OwnedPages is not Clone or Copy — it's move-only.
// The allocator pointer is NonNull and safe to send across threads if the
// allocator itself is Send. We don't implement Send/Sync here; the crate
// is single-threaded.

#[cfg(test)]
mod tests {
    use super::super::page_allocator::PageAllocator;
    use super::*;

    fn make_alloc() -> PageAllocator {
        PageAllocator::new(16, 100)
    }

    #[test]
    fn test_take_first() {
        let mut alloc = make_alloc();
        {
            let mut owned = alloc.allocate(5);
            assert_eq!(owned.size(), 5);
            let first = owned.take_first(2);
            assert_eq!(first.size(), 2);
            assert_eq!(owned.size(), 3);
        }
    }

    #[test]
    fn test_take_last() {
        let mut alloc = make_alloc();
        {
            let mut owned = alloc.allocate(5);
            let last = owned.take_last(2);
            assert_eq!(last.size(), 2);
            assert_eq!(owned.size(), 3);
        }
    }

    #[test]
    fn test_append() {
        let mut alloc = make_alloc();
        {
            let mut a = alloc.allocate(2);
            let mut b = alloc.allocate(3);
            a.append(&mut b);
            assert_eq!(a.size(), 5);
            assert!(b.is_empty());
        }
    }

    #[test]
    fn test_detach() {
        let mut alloc = make_alloc();
        assert_eq!(alloc.available_pages(), 100);
        {
            let mut owned = alloc.allocate(3);
            let ids = owned.detach();
            assert_eq!(ids.len(), 3);
            assert!(owned.is_empty());
        }
        // detached: pages not freed
        assert_eq!(alloc.available_pages(), 97);
    }

    #[test]
    fn test_release_ownership_by_id() {
        let mut alloc = make_alloc();
        {
            let mut owned = alloc.allocate(5);
            let ids = owned.ids().to_vec();
            let to_release = &ids[0..2];
            owned.release_ownership_by_id(to_release);
            assert_eq!(owned.size(), 3);
        }
        // Only 3 pages freed (2 were released)
    }
}
