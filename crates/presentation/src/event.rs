#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingEvent {
    Message(IncomingMessage),
    Callback(IncomingCallback),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    pub peer_id: i64,
    pub user_id: i64,
    pub text: String,
    pub is_group: bool,
    pub group_title: Option<String>,
}

impl IncomingMessage {
    pub fn new(peer_id: i64, user_id: i64, text: impl Into<String>) -> Self {
        Self {
            peer_id,
            user_id,
            text: text.into(),
            is_group: false,
            group_title: None,
        }
    }

    pub fn group(mut self, title: impl Into<String>) -> Self {
        self.is_group = true;
        self.group_title = Some(title.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingCallback {
    pub event_id: String,
    pub peer_id: i64,
    pub user_id: i64,
    pub payload: String,
}

impl IncomingCallback {
    pub fn new(
        event_id: impl Into<String>,
        peer_id: i64,
        user_id: i64,
        payload: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            peer_id,
            user_id,
            payload: payload.into(),
        }
    }
}
