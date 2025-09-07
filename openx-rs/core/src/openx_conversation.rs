use crate::error::Result as CodexResult;
use crate::openx::OpenX;
use crate::protocol::Event;
use crate::protocol::Op;
use crate::protocol::Submission;

pub struct OpenXConversation {
    codex: OpenX,
}

/// Conduit for the bidirectional stream of messages that compose a conversation
/// in OpenX.
impl OpenXConversation {
    pub(crate) fn new(codex: OpenX) -> Self {
        Self { codex }
    }

    pub async fn submit(&self, op: Op) -> CodexResult<String> {
        self.codex.submit(op).await
    }

    /// Use sparingly: this is intended to be removed soon.
    pub async fn submit_with_id(&self, sub: Submission) -> CodexResult<()> {
        self.codex.submit_with_id(sub).await
    }

    pub async fn next_event(&self) -> CodexResult<Event> {
        self.codex.next_event().await
    }
}
