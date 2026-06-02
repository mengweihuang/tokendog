//! Paged-cache group configuration, allocator, and per-request table.

use crate::resource::allocator::{OwnedPages, PageAllocator};

/// Positive-only ceiling division.
pub fn ceil_div_positive(numer: i32, denom: i32) -> i32 {
    if numer <= 0 {
        return 0;
    }
    (numer + denom - 1) / denom
}

/// Paged-cache group family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PagedCacheGroupFamily {
    History,
    State,
}

/// State restore policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRestorePolicy {
    SnapshotRequired,
}

/// Retention mode for a paged-cache group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    FullHistory,
    SlidingWindow { window_tokens: i32 },
}

/// Configuration for one paged-cache group.
#[derive(Debug, Clone)]
pub struct PagedCacheGroupConfig {
    pub group_id: String,
    pub rows_per_page: i32,
    pub entry_stride_tokens: i32,
    pub total_pages: i32,
    pub retention: Retention,
    pub family: PagedCacheGroupFamily,
}

impl PagedCacheGroupConfig {
    pub fn raw_tokens_per_page(&self) -> i32 {
        self.rows_per_page * self.entry_stride_tokens
    }
}

/// Group-level allocator wrapping PageAllocator with counters.
pub struct PagedCacheGroupAllocator {
    config: PagedCacheGroupConfig,
    pool: PageAllocator,
    allocated_pages_total: i64,
    released_pages_total: i64,
    failed_alloc_count: i64,
}

impl PagedCacheGroupAllocator {
    pub fn new(config: PagedCacheGroupConfig) -> Self {
        let total_pages = config.total_pages;
        Self {
            pool: PageAllocator::new(config.raw_tokens_per_page(), total_pages),
            config,
            allocated_pages_total: 0,
            released_pages_total: 0,
            failed_alloc_count: 0,
        }
    }

    pub fn allocate(&mut self, num_pages: i32) -> Vec<i32> {
        let owned = self.pool.allocate(num_pages);
        let ids = owned.ids().to_vec();
        self.allocated_pages_total += ids.len() as i64;
        ids
    }

    pub fn deallocate(&mut self, pages: &[i32]) {
        self.released_pages_total += pages.len() as i64;
        self.pool.deallocate(pages);
    }

    pub fn acquire_owned(&mut self, num_pages: i32) -> OwnedPages {
        let owned = self.pool.allocate(num_pages);
        if owned.is_empty() && num_pages > 0 {
            self.failed_alloc_count += 1;
        } else {
            self.allocated_pages_total += owned.size() as i64;
        }
        owned
    }

    pub fn config(&self) -> &PagedCacheGroupConfig { &self.config }
    pub fn total_pages(&self) -> i32 { self.pool.total_pages() }
    pub fn available_pages(&self) -> i32 { self.pool.available_pages() }
    pub fn failed_alloc_count(&self) -> i64 { self.failed_alloc_count }
}

/// Per-request, per-group page table with borrowed + owned segments.
pub struct PagedCacheGroupTable {
    allocator: Option<std::ptr::NonNull<PagedCacheGroupAllocator>>,
    owned_pages: OwnedPages,
    borrowed_page_ids: Vec<i32>,
    raw_token_cursor: i32,
    base_logical_page: i32,
    committed_prefix_len_tokens: i32,
    page_ids_view: Vec<i32>,
}

impl PagedCacheGroupTable {
    pub fn new() -> Self {
        Self {
            allocator: None,
            owned_pages: OwnedPages::empty(),
            borrowed_page_ids: Vec::new(),
            raw_token_cursor: 0,
            base_logical_page: 0,
            committed_prefix_len_tokens: 0,
            page_ids_view: Vec::new(),
        }
    }

    pub fn with_allocator(allocator: &mut PagedCacheGroupAllocator) -> Self {
        Self {
            allocator: std::ptr::NonNull::new(allocator as *mut PagedCacheGroupAllocator),
            owned_pages: OwnedPages::empty(),
            borrowed_page_ids: Vec::new(),
            raw_token_cursor: 0,
            base_logical_page: 0,
            committed_prefix_len_tokens: 0,
            page_ids_view: Vec::new(),
        }
    }

    pub fn page_ids(&self) -> &[i32] { &self.page_ids_view }
    pub fn size(&self) -> i32 { self.borrowed_page_ids.len() as i32 + self.owned_pages.size() }
    pub fn base_logical_page(&self) -> i32 { self.base_logical_page }
    pub fn raw_token_cursor(&self) -> i32 { self.raw_token_cursor }
    pub fn committed_prefix_len_tokens(&self) -> i32 { self.committed_prefix_len_tokens }
    pub fn is_empty(&self) -> bool { self.allocator.is_none() || self.size() == 0 }

    fn refresh_page_ids_view(&mut self) {
        self.page_ids_view.clear();
        self.page_ids_view.extend_from_slice(&self.borrowed_page_ids);
        self.page_ids_view.extend_from_slice(self.owned_pages.ids());
    }

    /// Grow pages to cover target_raw_tokens_exclusive.
    pub fn acquire(&mut self, target_raw_tokens_exclusive: i32) {
        if target_raw_tokens_exclusive <= self.raw_token_cursor {
            return;
        }
        let alloc = match self.allocator {
            Some(a) => unsafe { a.as_ptr().as_mut().unwrap() },
            None => return,
        };
        let rtp = alloc.config().raw_tokens_per_page();
        let needed_pages = ceil_div_positive(target_raw_tokens_exclusive, rtp)
            - ceil_div_positive(self.raw_token_cursor, rtp);
        if needed_pages > 0 {
            let mut new_pages = alloc.acquire_owned(needed_pages);
            self.owned_pages.append(&mut new_pages);
        }
        self.raw_token_cursor = target_raw_tokens_exclusive;
        self.refresh_page_ids_view();
    }

    /// Release all — owned via RAII, borrowed by clearing.
    pub fn release_all(&mut self) -> Vec<i32> {
        let mut released = Vec::new();
        // Detach owned pages to prevent Drop from also deallocating them
        released.extend_from_slice(&self.owned_pages.detach());
        released.extend_from_slice(&self.borrowed_page_ids);
        self.borrowed_page_ids.clear();
        self.page_ids_view.clear();
        self.raw_token_cursor = 0;
        released
    }

    /// Import borrowed page ids from a prefix-cache hit.
    pub fn import_prefix_borrowed(&mut self, ids: Vec<i32>, base: i32, _raw_tokens_covered: i32) {
        self.borrowed_page_ids = ids;
        self.base_logical_page = base;
        self.refresh_page_ids_view();
    }

    /// Sliding-only: drop front pages below window_lower_bound.
    pub fn release_skipped(&mut self, window_lower_bound: i32) -> Vec<i32> {
        // Simple impl: if requested, just skip
        if window_lower_bound <= self.base_logical_page {
            return Vec::new();
        }
        let to_skip = window_lower_bound - self.base_logical_page;
        let skipped = self.borrowed_page_ids.iter().take(to_skip as usize).copied().collect();
        self.borrowed_page_ids.drain(..(to_skip as usize).min(self.borrowed_page_ids.len()));
        self.base_logical_page = window_lower_bound;
        self.refresh_page_ids_view();
        skipped
    }
}
