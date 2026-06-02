//! Request pool allocator — bounded slot pool for request indices in a batch.

use std::fmt;
use std::collections::VecDeque;
use std::ptr::NonNull;

/// RAII slot handle. When dropped, the slot is returned to the pool.
pub struct ReqPoolIndex {
    pub slot: i32,
    allocator: Option<NonNull<ReqPoolAllocator>>,
}

impl ReqPoolIndex {
    /// Create a new ReqPoolIndex. Called by ReqPoolAllocator::allocate().
    pub(crate) fn new(slot: i32, allocator: &mut ReqPoolAllocator) -> Self {
        Self {
            slot,
            allocator: NonNull::new(allocator as *mut ReqPoolAllocator),
        }
    }

    /// Whether this index is valid (has been allocated).
    pub fn is_valid(&self) -> bool {
        self.allocator.is_some()
    }

    /// Create an invalid/empty index (for default initialization).
    pub fn invalid() -> Self {
        Self {
            slot: -1,
            allocator: None,
        }
    }
}

impl Drop for ReqPoolIndex {
    fn drop(&mut self) {
        if let Some(alloc) = self.allocator {
            // Safety: the allocator outlives this index by construction.
            unsafe { alloc.as_ptr().as_mut().unwrap() }.deallocate(self.slot);
        }
    }
}

impl fmt::Debug for ReqPoolIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReqPoolIndex")
            .field("slot", &self.slot)
            .field("has_allocator", &self.allocator.is_some())
            .finish()
    }
}

/// A bounded pool of integer slots (1..size).
///
/// Slots are allocated and freed via RAII ReqPoolIndex handles.
pub struct ReqPoolAllocator {
    size: i32,
    free_slots: VecDeque<i32>,
}

impl ReqPoolAllocator {
    /// Create a new pool with slots 1..size (0 is reserved).
    pub fn new(size: i32) -> Self {
        let mut free_slots = VecDeque::with_capacity(size as usize - 1);
        for i in 1..size {
            free_slots.push_back(i);
        }
        Self { size, free_slots }
    }

    /// Allocate a slot. Returns None if the pool is exhausted.
    pub fn allocate(&mut self) -> Option<ReqPoolIndex> {
        self.free_slots.pop_front().map(|slot| ReqPoolIndex::new(slot, self))
    }

    /// Return a slot to the pool.
    fn deallocate(&mut self, slot: i32) {
        self.free_slots.push_back(slot);
    }

    /// Total pool size.
    pub fn size(&self) -> i32 {
        self.size
    }

    /// Number of currently available slots.
    pub fn available_slots(&self) -> i32 {
        self.free_slots.len() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_and_free() {
        let mut pool = ReqPoolAllocator::new(10);
        assert_eq!(pool.available_slots(), 9);

        let idx = pool.allocate().unwrap();
        assert!(idx.is_valid());
        assert!(idx.slot >= 1 && idx.slot < 10);
        assert_eq!(pool.available_slots(), 8);

        drop(idx);
        assert_eq!(pool.available_slots(), 9);
    }

    #[test]
    fn test_exhaustion() {
        let mut pool = ReqPoolAllocator::new(3); // slots 1,2
        let _a = pool.allocate().unwrap();
        let _b = pool.allocate().unwrap();
        assert!(pool.allocate().is_none());
    }
}
