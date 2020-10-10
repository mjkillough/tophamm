pub mod awaiting;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Atomic counter that generates u8 request IDs, wrapping on overflow.
#[derive(Clone, Default)]
pub struct IncrementingId(Arc<AtomicU8>);

impl IncrementingId {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn next(&self) -> u8 {
        self.0.fetch_add(1, Ordering::SeqCst)
    }
}
