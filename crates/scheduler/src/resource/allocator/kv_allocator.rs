//! Per-request KV page allocator.
//!
//! Manages a tail partially-used page for efficient incremental allocation.

use std::ptr::NonNull;

use super::owned_pages::OwnedPages;
use super::page_allocator::PageAllocator;

/// Per-request allocator that manages pages for a single request's KV cache.
///
/// Maintains a tail partially-used page to minimize page waste when tokens
/// don't exactly fill pages.
pub struct LocalKVAllocator {
    allocator: Option<NonNull<PageAllocator>>,
    page_size: i32,
    pages: OwnedPages,
    tail_page_available_tokens: i32,
}

impl LocalKVAllocator {
    /// Create a new local allocator for a request with `num_tokens`.
    pub fn new(allocator: &mut PageAllocator, num_tokens: i32) -> Self {
        let page_size = allocator.page_size();
        let mut la = Self {
            allocator: NonNull::new(allocator as *mut PageAllocator),
            page_size,
            pages: OwnedPages::empty(),
            tail_page_available_tokens: 0,
        };
        la.acquire(num_tokens);
        la
    }

    /// Acquire pages for `num_tokens` additional tokens.
    pub fn acquire(&mut self, num_tokens: i32) {
        if num_tokens <= 0 {
            return;
        }
        // Consume from tail page first
        let needed = num_tokens - self.tail_page_available_tokens;
        if needed <= 0 {
            self.tail_page_available_tokens -= num_tokens;
            return;
        }
        self.tail_page_available_tokens = 0;

        let num_full_pages = needed / self.page_size;
        let remainder = needed % self.page_size;

        if num_full_pages > 0 {
            if let Some(alloc) = self.allocator {
                let alloc = unsafe { alloc.as_ptr().as_mut().unwrap() };
                let mut new_pages = alloc.allocate(num_full_pages);
                if !new_pages.is_empty() {
                    // Transfer ownership: detach prevents double-free on drop
                    let ids = new_pages.detach();
                    self.pages.append(&mut OwnedPages::new(alloc, ids));
                }
            }
        }

        if remainder > 0 {
            if let Some(alloc) = self.allocator {
                let alloc = unsafe { alloc.as_ptr().as_mut().unwrap() };
                let mut tail = alloc.allocate(1);
                if !tail.is_empty() {
                    self.tail_page_available_tokens = self.page_size - remainder;
                    let ids = tail.detach();
                    self.pages.append(&mut OwnedPages::new(alloc, ids));
                }
            }
        }
    }

    /// Take all fully-used pages out. Only the tail page remains.
    pub fn take_full_pages(&mut self) -> OwnedPages {
        if self.tail_page_available_tokens > 0 {
            // Keep the last page (tail)
            let n = self.pages.size() - 1;
            if n > 0 {
                self.pages.take_first(n)
            } else {
                OwnedPages::empty()
            }
        } else if self.pages.size() > 0 {
            // No tail page — take all pages via detach
            let all_ids = self.pages.detach();
            OwnedPages {
                allocator: self.allocator,
                ids: all_ids,
            }
        } else {
            OwnedPages::empty()
        }
    }

    /// Take the first `n` pages from the allocator.
    pub fn take_first(&mut self, n: i32) -> OwnedPages {
        self.pages.take_first(n)
    }

    /// Page IDs currently held.
    pub fn pages(&self) -> Vec<i32> {
        self.pages.ids().to_vec()
    }

    /// Number of available tokens in the tail page.
    pub fn tail_page_available_tokens(&self) -> i32 {
        self.tail_page_available_tokens
    }

    /// Release ownership of specific pages (don't return them to the pool).
    pub fn release_ownership_by_id(&mut self, pages: &[i32]) {
        self.pages.release_ownership_by_id(pages);
    }
}

#[cfg(test)]
mod tests {
    use super::super::page_allocator::PageAllocator;
    use super::*;

    #[test]
    fn test_acquire_small() {
        let mut pool = PageAllocator::new(16, 100);
        let la = LocalKVAllocator::new(&mut pool, 10);
        // 10 tokens fit in one page with 6 remaining
        assert_eq!(la.tail_page_available_tokens(), 6);
        assert_eq!(la.pages().len(), 1);
    }

    #[test]
    fn test_acquire_exact_page() {
        let mut pool = PageAllocator::new(16, 100);
        let la = LocalKVAllocator::new(&mut pool, 16);
        assert_eq!(la.tail_page_available_tokens(), 0);
        assert_eq!(la.pages().len(), 1);
    }

    #[test]
    fn test_acquire_multiple_pages() {
        let mut pool = PageAllocator::new(16, 100);
        let la = LocalKVAllocator::new(&mut pool, 50);
        // 50 tokens: 3 full pages (48 tokens) + 2 remaining on tail
        assert_eq!(la.tail_page_available_tokens(), 14); // 16 - 2 = 14
        assert_eq!(la.pages().len(), 4);
    }

    #[test]
    fn test_take_full_pages() {
        let mut pool = PageAllocator::new(16, 100);
        let mut la = LocalKVAllocator::new(&mut pool, 50);
        let full = la.take_full_pages();
        // 4 pages total, tail has 14 available -> 3 full pages taken
        assert_eq!(full.size(), 3);
    }
}
