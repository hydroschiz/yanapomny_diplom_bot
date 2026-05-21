use serde_json::Value;
use vk_bot_api::models::{Event, Message, MessageEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VkIncomingEvent {
    Message(VkIncomingMessage),
    Callback(VkIncomingCallback),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VkIncomingMessage {
    pub peer_id: i64,
    pub user_id: i64,
    pub text: String,
    pub is_group: bool,
    pub group_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VkIncomingCallback {
    pub event_id: String,
    pub peer_id: i64,
    pub user_id: i64,
    pub payload: String,
}

pub fn normalize_event(event: &Event) -> Option<VkIncomingEvent> {
    match event {
        Event::MessageNew(message) => Some(VkIncomingEvent::Message(normalize_message(message))),
        Event::MessageEvent(event) => normalize_callback(event).map(VkIncomingEvent::Callback),
        _ => None,
    }
}

pub fn normalize_message(message: &Message) -> VkIncomingMessage {
    VkIncomingMessage {
        peer_id: message.peer_id,
        user_id: message.from_id,
        text: message.text.clone(),
        is_group: is_group_peer(message.peer_id),
        group_title: message
            .action
            .as_ref()
            .and_then(|action| action.text.clone()),
    }
}

pub fn normalize_callback(event: &MessageEvent) -> Option<VkIncomingCallback> {
    Some(VkIncomingCallback {
        event_id: event.event_id.clone(),
        peer_id: event.peer_id,
        user_id: event.user_id,
        payload: callback_payload(event)?,
    })
}

pub fn callback_payload(event: &MessageEvent) -> Option<String> {
    let payload = event.payload.as_ref()?;

    payload
        .get("action")
        .or_else(|| payload.get("command"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .values()
                .find_map(|value| value.as_str().map(ToOwned::to_owned))
        })
}

pub fn is_group_peer(peer_id: i64) -> bool {
    peer_id >= 2_000_000_000
}
