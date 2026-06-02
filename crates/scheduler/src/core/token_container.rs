/// Stores tokens for a single request, with prefill/decode boundary tracking.
///
/// The first `num_prefill_tokens` tokens are the prompt; subsequent tokens
/// are decode-generated.
#[derive(Debug)]
pub struct TokenContainer {
    tokens: Vec<i32>,
    num_prefill_tokens: usize,
}

/// A view into a TokenContainer's tokens, yielding full pages as slices.
pub struct PagedTokenView<'a> {
    tokens: &'a [i32],
    page_size: usize,
    page_idx: usize,
    num_full_pages: usize,
}

impl<'a> Iterator for PagedTokenView<'a> {
    type Item = &'a [i32];

    fn next(&mut self) -> Option<Self::Item> {
        if self.page_idx >= self.num_full_pages {
            return None;
        }
        let start = self.page_idx * self.page_size;
        self.page_idx += 1;
        Some(&self.tokens[start..start + self.page_size])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.num_full_pages - self.page_idx;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for PagedTokenView<'a> {}

impl TokenContainer {
    /// Create a new TokenContainer from the given tokens.
    /// All initial tokens are considered prefill tokens.
    pub fn new(tokens: Vec<i32>) -> Self {
        let num_prefill_tokens = tokens.len();
        Self {
            tokens,
            num_prefill_tokens,
        }
    }

    /// Append decode-generated tokens.
    pub fn extend(&mut self, new_tokens: &[i32]) {
        self.tokens.extend_from_slice(new_tokens);
    }

    /// Return an iterator over full pages of tokens.
    ///
    /// If `except_last` is true, the last token is excluded from paging
    /// (used during decoding where the last token is the one being generated).
    pub fn get_full_paged_tokens(&self, page_size: usize, except_last: bool) -> PagedTokenView<'_> {
        let token_count = if except_last && !self.tokens.is_empty() {
            self.tokens.len().saturating_sub(1)
        } else {
            self.tokens.len()
        };
        let num_full_pages = token_count / page_size;
        PagedTokenView {
            tokens: &self.tokens,
            page_size,
            page_idx: 0,
            num_full_pages,
        }
    }

    /// Total number of tokens (prefill + decode).
    pub fn size(&self) -> usize {
        self.tokens.len()
    }

    /// Number of prefill (prompt) tokens.
    pub fn prefill_size(&self) -> usize {
        self.num_prefill_tokens
    }

    /// Get a slice of tokens by window.
    pub fn get_token_slice(&self, begin: usize, size: usize) -> &[i32] {
        &self.tokens[begin..begin + size]
    }

    /// The last token in the container.
    pub fn last_token(&self) -> i32 {
        self.tokens[self.tokens.len() - 1]
    }

    /// Borrow the raw token vector.
    pub fn tokens(&self) -> &[i32] {
        &self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_token_container() {
        let tc = TokenContainer::new(vec![1, 2, 3, 4, 5]);
        assert_eq!(tc.size(), 5);
        assert_eq!(tc.prefill_size(), 5);
    }

    #[test]
    fn test_extend() {
        let mut tc = TokenContainer::new(vec![1, 2, 3]);
        tc.extend(&[4, 5]);
        assert_eq!(tc.size(), 5);
        assert_eq!(tc.prefill_size(), 3); // prefill size unchanged
    }

    #[test]
    fn test_get_full_paged_tokens() {
        let tc = TokenContainer::new(vec![1, 2, 3, 4, 5, 6]);
        let pages: Vec<_> = tc.get_full_paged_tokens(2, false).collect();
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0], &[1, 2]);
        assert_eq!(pages[1], &[3, 4]);
        assert_eq!(pages[2], &[5, 6]);
    }

    #[test]
    fn test_get_full_paged_tokens_except_last() {
        let tc = TokenContainer::new(vec![1, 2, 3, 4, 5]);
        let pages: Vec<_> = tc.get_full_paged_tokens(2, true).collect();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0], &[1, 2]);
        assert_eq!(pages[1], &[3, 4]);
    }

    #[test]
    fn test_get_token_slice() {
        let tc = TokenContainer::new(vec![10, 20, 30, 40, 50]);
        let slice = tc.get_token_slice(1, 3);
        assert_eq!(slice, &[20, 30, 40]);
    }

    #[test]
    fn test_last_token() {
        let tc = TokenContainer::new(vec![7, 8, 9]);
        assert_eq!(tc.last_token(), 9);
    }

    #[test]
    fn test_empty_container_pages() {
        let tc = TokenContainer::new(vec![]);
        let pages: Vec<_> = tc.get_full_paged_tokens(4, false).collect();
        assert!(pages.is_empty());
    }

    #[test]
    fn test_partial_page_not_returned() {
        let tc = TokenContainer::new(vec![1, 2, 3]);
        let pages: Vec<_> = tc.get_full_paged_tokens(2, false).collect();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0], &[1, 2]);
    }
}
