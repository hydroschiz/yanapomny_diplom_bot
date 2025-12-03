use teloxide::types::{CallbackQuery, ChatKind, Message};

pub fn private_chat_msg(msg: Message) -> bool {
    matches!(msg.chat.kind, ChatKind::Private(_))
}

pub fn private_chat_cq(cq: CallbackQuery) -> bool {
    cq.message
        .as_ref()
        .map(|m| matches!(m.chat().kind, ChatKind::Private(_)))
        .unwrap_or(false)
}
