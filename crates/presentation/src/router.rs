use application::DialogState;

use crate::{parse_command, parse_payload, BotCommand, IncomingCallback, IncomingMessage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRoute {
    Start,
    Help,
    Yan,
    ShowUtc,
    ShowSetup,
    ShowPay,
    ListReminders,
    ShowSubscriptions,
    ShowProfile,
    ShowReferral,
    CreateReminderFromCommand(String),
    UnknownCommand(String),
    UtcInput(String),
    SnoozeButtonsInput(String),
    AutoSnoozeInput(String),
    ReminderText(String),
    Empty,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Router;

impl Router {
    pub fn route_message(&self, message: &IncomingMessage, state: DialogState) -> MessageRoute {
        let text = message.text.trim();
        if text.is_empty() {
            return MessageRoute::Empty;
        }

        if let Some(parsed) = parse_command(text) {
            return route_command(parsed.command);
        }

        match state {
            DialogState::AwaitingUtc => MessageRoute::UtcInput(text.to_string()),
            DialogState::AwaitingSnoozeButtons => {
                MessageRoute::SnoozeButtonsInput(text.to_string())
            }
            DialogState::AwaitingAutoSnooze => MessageRoute::AutoSnoozeInput(text.to_string()),
            DialogState::Idle => MessageRoute::ReminderText(text.to_string()),
        }
    }

    pub fn route_callback(&self, callback: &IncomingCallback) -> crate::CallbackPayload {
        parse_payload(&callback.payload)
    }
}

fn route_command(command: BotCommand) -> MessageRoute {
    match command {
        BotCommand::Start => MessageRoute::Start,
        BotCommand::Help => MessageRoute::Help,
        BotCommand::Yan => MessageRoute::Yan,
        BotCommand::Utc => MessageRoute::ShowUtc,
        BotCommand::Setup => MessageRoute::ShowSetup,
        BotCommand::Pay => MessageRoute::ShowPay,
        BotCommand::List => MessageRoute::ListReminders,
        BotCommand::Subs => MessageRoute::ShowSubscriptions,
        BotCommand::Profile => MessageRoute::ShowProfile,
        BotCommand::Ref => MessageRoute::ShowReferral,
        BotCommand::Remind(text) => MessageRoute::CreateReminderFromCommand(text),
        BotCommand::Unknown(name) => MessageRoute::UnknownCommand(name),
    }
}
