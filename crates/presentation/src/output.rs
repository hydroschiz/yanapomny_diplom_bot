use transport_core::MessageContent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub peer_id: i64,
    pub content: MessageContent,
}

impl OutgoingMessage {
    pub const fn new(peer_id: i64, content: MessageContent) -> Self {
        Self { peer_id, content }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingCallbackAnswer {
    pub event_id: String,
    pub user_id: i64,
    pub peer_id: i64,
    pub text: Option<String>,
}

impl OutgoingCallbackAnswer {
    pub fn new(
        event_id: impl Into<String>,
        user_id: i64,
        peer_id: i64,
        text: Option<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            user_id,
            peer_id,
            text,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderedResponse {
    pub messages: Vec<OutgoingMessage>,
    pub callback_answer: Option<OutgoingCallbackAnswer>,
}

impl RenderedResponse {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn message(peer_id: i64, content: MessageContent) -> Self {
        Self {
            messages: vec![OutgoingMessage::new(peer_id, content)],
            callback_answer: None,
        }
    }

    pub fn with_callback_answer(mut self, answer: OutgoingCallbackAnswer) -> Self {
        self.callback_answer = Some(answer);
        self
    }

    pub fn push_message(&mut self, peer_id: i64, content: MessageContent) {
        self.messages.push(OutgoingMessage::new(peer_id, content));
    }
}
