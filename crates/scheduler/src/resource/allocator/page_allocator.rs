//! Free-list page allocator.
//!
//! Allocates page IDs from a fixed pool. Pages are allocated from the end
//! of the free list and deallocated by appending back.

use super::owned_pages::OwnedPages;

/// A simple free-list page allocator.
///
/// Page indices start at 1 (page 0 is reserved/not used).
pub struct PageAllocator {
    page_size: i32,
    total_pages: i32,
    free_pages: Vec<i32>,
}

impl PageAllocator {
    /// Create a new page allocator with `total_pages` pages.
    /// Page indices are 0..total_pages-1.
    pub fn new(page_size: i32, total_pages: i32) -> Self {
        let mut free_pages = Vec::with_capacity(total_pages as usize);
        for i in 0..total_pages {
            free_pages.push(i);
        }
        Self {
            page_size,
            total_pages,
            free_pages,
        }
    }

    /// Allocate `num_pages` pages. Returns empty OwnedPages if insufficient.
    pub fn allocate(&mut self, num_pages: i32) -> OwnedPages {
        if num_pages <= 0 || num_pages as usize > self.free_pages.len() {
            return OwnedPages::empty();
        }
        let mut pages = Vec::with_capacity(num_pages as usize);
        for _ in 0..num_pages {
            pages.push(self.free_pages.pop().unwrap());
        }
        OwnedPages::new(self, pages)
    }

    /// Return pages to the free list.
    pub fn deallocate(&mut self, pages: &[i32]) {
        self.free_pages.extend_from_slice(pages);
    }

    /// Size of each page in tokens.
    pub fn page_size(&self) -> i32 {
        self.page_size
    }

    /// Total number of pages in the pool.
    pub fn total_pages(&self) -> i32 {
        self.total_pages
    }

    /// Number of currently available (free) pages.
    pub fn available_pages(&self) -> i32 {
        self.free_pages.len() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_deallocate() {
        let mut alloc = PageAllocator::new(16, 10);
        assert_eq!(alloc.available_pages(), 10); // pages 0..9

        let owned = alloc.allocate(3);
        assert!(owned.size() > 0);
        assert_eq!(alloc.available_pages(), 7);

        // Pages are returned on drop
        drop(owned);
        assert_eq!(alloc.available_pages(), 10);
    }

    #[test]
    fn test_allocate_too_many() {
        let mut alloc = PageAllocator::new(16, 5);
        let owned = alloc.allocate(10);
        assert!(owned.is_empty());
    }

    #[test]
    fn test_page_size() {
        let alloc = PageAllocator::new(32, 100);
        assert_eq!(alloc.page_size(), 32);
        assert_eq!(alloc.total_pages(), 100);
    }
}
