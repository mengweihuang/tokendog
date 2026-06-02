//! Request wrapper: token container + FSM state.

use crate::core::TokenContainer;
use crate::fsm::State;

/// A scheduled request combining token storage and FSM state.
pub struct Request {
    pub id: String,
    pub token_container: TokenContainer,
    pub state: State,
}

impl Request {
    /// Create a new request in Submitted state.
    pub fn new(id: String, tokens: Vec<i32>, page_size: i32) -> Self {
        let tc = TokenContainer::new(tokens);
        Self {
            id,
            token_container: tc,
            state: State::Submitted(crate::fsm::Submitted {
                token_container: TokenContainer::new(vec![]),  // placeholder
                page_size,
            }),
        }
    }

    /// Get the token count for this request.
    pub fn token_size(&self) -> i32 {
        self.token_container.size() as i32
    }

    /// Get the prefill token count.
    pub fn prefill_size(&self) -> i32 {
        self.token_container.prefill_size() as i32
    }
}
