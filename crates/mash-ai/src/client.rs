// crates/mash-ai/src/client.rs
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use futures_core::Stream;

use crate::types::{LlmRequest, LlmResponse, StreamEvent};

/// Unified LLM client trait.
pub trait LlmClient: Send + Sync {
    /// Non-streaming completion.
    fn complete(
        &self,
        request: &LlmRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse>> + Send + '_>>;

    /// Streaming completion.
    fn stream(
        &self,
        request: &LlmRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>>>
                + Send
                + '_,
        >,
    >;
}
