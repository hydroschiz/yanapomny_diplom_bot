use application::DialogState;

use crate::{
    parse_command, parse_payload, BotCommand, CallbackPayload, IncomingCallback, IncomingMessage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelPlatform {
    Twitch,
    Youtube,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChannelLink {
    pub platform: ChannelPlatform,
    pub channel_id: String,
    pub channel_name: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteContext {
    pub bot_username: Option<String>,
}

impl RouteContext {
    pub fn for_bot(bot_username: impl Into<String>) -> Self {
        Self {
            bot_username: Some(bot_username.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationState {
    Idle,
    AwaitingUtc,
    AwaitingSnoozeButtons,
    AwaitingAutoSnooze,
    AwaitingPayment,
    AwaitingTextConfirmation,
    AwaitingReminderConfirmation,
    AwaitingReminderEdit,
    AwaitingReminderDeletion,
    AwaitingSubDeleteNum,
}

impl From<DialogState> for ConversationState {
    fn from(value: DialogState) -> Self {
        match value {
            DialogState::Idle => Self::Idle,
            DialogState::AwaitingUtc => Self::AwaitingUtc,
            DialogState::AwaitingSnoozeButtons => Self::AwaitingSnoozeButtons,
            DialogState::AwaitingAutoSnooze => Self::AwaitingAutoSnooze,
        }
    }
}

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
    GroupReminderText(String),
    ChannelSubscriptionUrl(ParsedChannelLink),
    ReminderEditText(String),
    ReminderDeletionInput(String),
    ChannelDeletionInput(String),
    Ignored,
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackRoute {
    ShowSetupMenu,
    StartSnoozeSetup,
    StartAutoSnoozeSetup,
    StartUtcSetup,
    CancelUtc,
    ShowUtcPage(usize),
    SetUtc(String),
    ShowPayMenu,
    CancelPayment,
    SelectPaymentPeriod(i32),
    StartYooKassaPayment(i32),
    CheckPayment(i32),
    ConfirmText,
    CancelText,
    ConfirmReminder,
    EditReminder,
    CancelReminder,
    StartReminderDeletion,
    BackFromReminderDeletion,
    ListReminders,
    SnoozeReminder { reminder_id: i32, code: String },
    CompleteReminder(i32),
    StartSubscriptionDeletion,
    ShowSubscriptions,
    ShowProfile,
    ShowReferral,
    BackMain,
    Unknown(String),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Router;

impl Router {
    pub fn route_message(&self, message: &IncomingMessage, state: DialogState) -> MessageRoute {
        self.route_message_with_context(message, state, RouteContext::default())
    }

    pub fn route_message_state(
        &self,
        message: &IncomingMessage,
        state: ConversationState,
    ) -> MessageRoute {
        self.route_message_with_context(message, state, RouteContext::default())
    }

    pub fn route_message_with_context(
        &self,
        message: &IncomingMessage,
        state: impl Into<ConversationState>,
        context: RouteContext,
    ) -> MessageRoute {
        let text = message.text.trim();
        if text.is_empty() {
            return MessageRoute::Empty;
        }

        if let Some(parsed) = parse_command(text) {
            return route_command(parsed.command);
        }

        match state.into() {
            ConversationState::AwaitingUtc => MessageRoute::UtcInput(text.to_string()),
            ConversationState::AwaitingSnoozeButtons => {
                MessageRoute::SnoozeButtonsInput(text.to_string())
            }
            ConversationState::AwaitingAutoSnooze => {
                MessageRoute::AutoSnoozeInput(text.to_string())
            }
            ConversationState::AwaitingReminderEdit => {
                MessageRoute::ReminderEditText(text.to_string())
            }
            ConversationState::AwaitingReminderDeletion => {
                MessageRoute::ReminderDeletionInput(text.to_string())
            }
            ConversationState::AwaitingSubDeleteNum => {
                MessageRoute::ChannelDeletionInput(text.to_string())
            }
            ConversationState::AwaitingPayment
            | ConversationState::AwaitingTextConfirmation
            | ConversationState::AwaitingReminderConfirmation => MessageRoute::Ignored,
            ConversationState::Idle => route_idle_message(message, text, &context),
        }
    }

    pub fn route_callback(&self, callback: &IncomingCallback) -> crate::CallbackPayload {
        parse_payload(&callback.payload)
    }

    pub fn route_callback_action(&self, callback: &IncomingCallback) -> CallbackRoute {
        route_callback_payload(parse_payload(&callback.payload))
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

fn route_idle_message(
    message: &IncomingMessage,
    text: &str,
    context: &RouteContext,
) -> MessageRoute {
    if message.is_group {
        return context
            .bot_username
            .as_deref()
            .and_then(|username| extract_group_mention_text(text, username))
            .map(MessageRoute::GroupReminderText)
            .unwrap_or(MessageRoute::Ignored);
    }

    if let Some(channel) = parse_channel_url(text) {
        return MessageRoute::ChannelSubscriptionUrl(channel);
    }

    MessageRoute::ReminderText(text.to_string())
}

fn route_callback_payload(payload: CallbackPayload) -> CallbackRoute {
    match payload {
        CallbackPayload::SetupMenu | CallbackPayload::ProfileSetup => CallbackRoute::ShowSetupMenu,
        CallbackPayload::SetupSnooze => CallbackRoute::StartSnoozeSetup,
        CallbackPayload::SetupAuto => CallbackRoute::StartAutoSnoozeSetup,
        CallbackPayload::SetupUtc => CallbackRoute::StartUtcSetup,
        CallbackPayload::UtcCancel => CallbackRoute::CancelUtc,
        CallbackPayload::UtcPage(page) => CallbackRoute::ShowUtcPage(page),
        CallbackPayload::UtcSet(offset) => CallbackRoute::SetUtc(offset),
        CallbackPayload::PayMenu | CallbackPayload::ProfilePay => CallbackRoute::ShowPayMenu,
        CallbackPayload::PayCancel => CallbackRoute::CancelPayment,
        CallbackPayload::PaySelect(months) => CallbackRoute::SelectPaymentPeriod(months),
        CallbackPayload::PayYooKassa(months) => CallbackRoute::StartYooKassaPayment(months),
        CallbackPayload::PayCheck(months) => CallbackRoute::CheckPayment(months),
        CallbackPayload::TextConfirm => CallbackRoute::ConfirmText,
        CallbackPayload::TextCancel => CallbackRoute::CancelText,
        CallbackPayload::ReminderConfirm => CallbackRoute::ConfirmReminder,
        CallbackPayload::ReminderEdit => CallbackRoute::EditReminder,
        CallbackPayload::ReminderCancel => CallbackRoute::CancelReminder,
        CallbackPayload::ReminderDeleteStart => CallbackRoute::StartReminderDeletion,
        CallbackPayload::ReminderDeleteBack => CallbackRoute::BackFromReminderDeletion,
        CallbackPayload::ReminderList | CallbackPayload::ProfileList => {
            CallbackRoute::ListReminders
        }
        CallbackPayload::Snooze { reminder_id, code } => {
            CallbackRoute::SnoozeReminder { reminder_id, code }
        }
        CallbackPayload::ReminderDone(reminder_id) => CallbackRoute::CompleteReminder(reminder_id),
        CallbackPayload::SubDelete => CallbackRoute::StartSubscriptionDeletion,
        CallbackPayload::Subs | CallbackPayload::ProfileSubs => CallbackRoute::ShowSubscriptions,
        CallbackPayload::Profile => CallbackRoute::ShowProfile,
        CallbackPayload::ProfileReferral => CallbackRoute::ShowReferral,
        CallbackPayload::BackMain => CallbackRoute::BackMain,
        CallbackPayload::Unknown(value) => CallbackRoute::Unknown(value),
    }
}

pub fn extract_group_mention_text(text: &str, bot_username: &str) -> Option<String> {
    let username = bot_username.trim_start_matches('@');

    for (at_index, _) in text.match_indices('@') {
        let after_at = &text[at_index + 1..];
        if after_at.len() < username.len() {
            continue;
        }

        let candidate = &after_at[..username.len()];
        if !candidate.eq_ignore_ascii_case(username) {
            continue;
        }

        let boundary = after_at[username.len()..].chars().next();
        if matches!(boundary, Some(ch) if ch.is_ascii_alphanumeric() || ch == '_') {
            continue;
        }

        let mut suffix = &after_at[username.len()..];
        suffix = suffix.trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, ',' | ':' | ';' | '-' | '!' | '?')
        });

        let prefix = text[..at_index].trim_end();
        let cleaned = if prefix.is_empty() {
            suffix.trim().to_string()
        } else if suffix.trim().is_empty() {
            prefix.to_string()
        } else {
            format!("{} {}", prefix, suffix.trim())
        };

        let cleaned = cleaned.trim().to_string();
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }

    None
}

pub fn parse_channel_url(url: &str) -> Option<ParsedChannelLink> {
    let normalized = normalize_url_input(url.trim());
    if normalized.is_empty() {
        return None;
    }

    if let Some(path) = strip_prefix_ci(&normalized, "twitch.tv/") {
        let username = first_path_segment(path)?.to_ascii_lowercase();
        if username.is_empty() {
            return None;
        }
        return Some(ParsedChannelLink {
            platform: ChannelPlatform::Twitch,
            channel_id: username.clone(),
            channel_name: username.clone(),
            url: format!("https://twitch.tv/{username}"),
        });
    }

    if let Some(path) = strip_prefix_ci(&normalized, "youtube.com/@") {
        let handle = first_path_segment(path)?;
        if handle.is_empty() {
            return None;
        }
        return Some(ParsedChannelLink {
            platform: ChannelPlatform::Youtube,
            channel_id: format!("@{handle}"),
            channel_name: handle.to_string(),
            url: format!("https://youtube.com/@{handle}"),
        });
    }

    if let Some(path) = strip_prefix_ci(&normalized, "youtube.com/channel/") {
        let channel_id = first_path_segment(path)?;
        if channel_id.is_empty() {
            return None;
        }
        return Some(ParsedChannelLink {
            platform: ChannelPlatform::Youtube,
            channel_id: channel_id.to_string(),
            channel_name: channel_id.to_string(),
            url: format!("https://youtube.com/channel/{channel_id}"),
        });
    }

    if let Some(path) = strip_prefix_ci(&normalized, "youtube.com/c/") {
        let name = first_path_segment(path)?;
        if name.is_empty() {
            return None;
        }
        return Some(ParsedChannelLink {
            platform: ChannelPlatform::Youtube,
            channel_id: format!("c/{name}"),
            channel_name: name.to_string(),
            url: format!("https://youtube.com/c/{name}"),
        });
    }

    None
}

fn normalize_url_input(input: &str) -> String {
    let without_scheme = strip_prefix_ci(input, "https://")
        .or_else(|| strip_prefix_ci(input, "http://"))
        .unwrap_or(input);
    strip_prefix_ci(without_scheme, "www.")
        .unwrap_or(without_scheme)
        .to_string()
}

fn strip_prefix_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = value.get(..prefix.len())?;
    if candidate.eq_ignore_ascii_case(prefix) {
        value.get(prefix.len()..)
    } else {
        None
    }
}

fn first_path_segment(path: &str) -> Option<&str> {
    path.split(['/', '?', '#']).next().map(str::trim)
}
