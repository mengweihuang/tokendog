//! RAII Mamba slot handle.
//!
//! When dropped, MambaSlot returns its slot index to the allocator via a
//! callback. This avoids lifetime coupling between the tree and allocators.

/// RAII handle for a Mamba cache slot.
///
/// The slot is returned to the allocator on drop via a releaser callback.
/// This design avoids storing raw pointers to the allocator, decoupling
/// lifetimes between the tree (which owns MambaSlots) and the allocator
/// (which lives in the Scheduler).
pub struct MambaSlot {
    index: i32,
    releaser: Option<Box<dyn FnOnce(i32)>>,
}

impl MambaSlot {
    /// Create a new MambaSlot with the given index and no releaser.
    /// The slot will NOT be returned on drop (useful for testing).
    pub fn new_noop(index: i32) -> Self {
        Self {
            index,
            releaser: None,
        }
    }

    /// Create a new MambaSlot with a releaser callback.
    pub fn new(index: i32, releaser: Box<dyn FnOnce(i32)>) -> Self {
        Self {
            index,
            releaser: Some(releaser),
        }
    }

    /// The slot index.
    pub fn index(&self) -> i32 {
        self.index
    }
}

impl Drop for MambaSlot {
    fn drop(&mut self) {
        if let Some(releaser) = self.releaser.take() {
            releaser(self.index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_mamba_slot_drop_calls_releaser() {
        let freed = Arc::new(AtomicI32::new(-1));
        let freed_clone = freed.clone();

        {
            let _slot = MambaSlot::new(42, Box::new(move |idx| {
                freed_clone.store(idx, Ordering::SeqCst);
            }));
        }

        assert_eq!(freed.load(Ordering::SeqCst), 42);
    }

    #[test]
    fn test_mamba_slot_noop() {
        // Should not panic on drop
        let _slot = MambaSlot::new_noop(7);
        assert_eq!(_slot.index(), 7);
    }
}
