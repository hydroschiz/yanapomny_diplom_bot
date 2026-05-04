use serde::{Deserialize, Serialize};

/// User text awaiting confirmation BEFORE sending to LLM.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PendingText {
    /// Original user text.
    pub text: String,
}

/// Pending reminder data after LLM parsing, before final confirmation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PendingReminder {
    /// Original user text.
    pub original_text: String,
    /// Parsed description from LLM.
    pub description: String,
    /// Formatted time string for display.
    pub time_display: String,
    /// Serialized ParsedReminder JSON.
    pub parsed_json: String,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub enum AppState {
    #[default]
    Idle,
    AwaitingUtc,
    AwaitingSnoozeButtons,
    AwaitingAutoSnooze,
    /// User is in payment flow, storing selected months.
    AwaitingPayment { months: i32 },
    /// User sent text, awaiting confirmation BEFORE LLM parsing.
    AwaitingTextConfirmation { pending: PendingText },
    /// User confirmed, LLM parsed, awaiting final create/edit/cancel.
    AwaitingReminderConfirmation { pending: PendingReminder },
    /// User is editing reminder text before confirmation.
    AwaitingReminderEdit { pending: PendingReminder },
    /// User is in reminder deletion flow, awaiting reminder number.
    AwaitingReminderDeletion,
    /// User is in channel subscription deletion flow, awaiting subscription number.
    AwaitingSubDeleteNum,
}
