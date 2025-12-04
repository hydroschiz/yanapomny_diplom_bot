use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub enum AppState {
    #[default]
    Idle,
    AwaitingUtc,
    AwaitingSnoozeButtons,
    AwaitingAutoSnooze,
    /// User is in payment flow, storing selected months.
    AwaitingPayment { months: i32 },
}
