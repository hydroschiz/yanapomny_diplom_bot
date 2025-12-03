use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub enum AppState {
    #[default]
    Idle,
    AwaitingUtc,
    AwaitingSnoozeButtons,
    AwaitingAutoSnooze,
}
