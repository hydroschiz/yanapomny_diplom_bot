//! Хранилище состояний FSM (Dialogue Store).
//!
//! Заменяет `teloxide::InMemStorage<AppState>` на собственную реализацию,
//! позволяющую использовать любой транспорт (Telegram / VK).
//!
//! Использует `DashMap` для thread-safe хранения состояний пользователей.

use std::sync::Arc;

use dashmap::DashMap;

use crate::bot::states::AppState;

/// Хранилище состояний диалога (FSM).
///
/// Позволяет получить, обновить и сбросить состояние пользователя.
/// Потокобезопасно (использует `DashMap`).
///
/// # Пример
///
/// ```ignore
/// let store = DialogueStore::new();
/// store.update(user_id, AppState::AwaitingUtc);
/// let state = store.get(user_id);
/// store.reset(user_id); // → Idle
/// ```
#[derive(Clone, Debug)]
pub struct DialogueStore {
    /// Map user_id → AppState
    states: Arc<DashMap<i64, AppState>>,
}

impl Default for DialogueStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DialogueStore {
    /// Создать новое хранилище.
    pub fn new() -> Self {
        Self {
            states: Arc::new(DashMap::new()),
        }
    }

    /// Получить текущее состояние пользователя.
    ///
    /// Если состояния нет, возвращает `AppState::Idle`.
    pub fn get(&self, user_id: i64) -> AppState {
        self.states
            .get(&user_id)
            .map(|entry| entry.clone())
            .unwrap_or_default()
    }

    /// Обновить состояние пользователя.
    pub fn update(&self, user_id: i64, state: AppState) {
        self.states.insert(user_id, state);
    }

    /// Сбросить состояние пользователя в `Idle`.
    pub fn reset(&self, user_id: i64) {
        self.states.insert(user_id, AppState::Idle);
    }

    /// Проверить, есть ли состояние для пользователя.
    pub fn has(&self, user_id: i64) -> bool {
        self.states.contains_key(&user_id)
    }

    /// Удалить состояние пользователя (если нужно).
    pub fn remove(&self, user_id: i64) {
        self.states.remove(&user_id);
    }

    /// Получить количество активных состояний (для метрик/отладки).
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Проверить, пусто ли хранилище.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_store_is_empty() {
        let store = DialogueStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_get_unknown_user_returns_idle() {
        let store = DialogueStore::new();
        assert_eq!(store.get(123), AppState::Idle);
    }

    #[test]
    fn test_update_and_get() {
        let store = DialogueStore::new();
        store.update(123, AppState::AwaitingUtc);
        assert_eq!(store.get(123), AppState::AwaitingUtc);
    }

    #[test]
    fn test_update_replaces_previous_state() {
        let store = DialogueStore::new();
        store.update(123, AppState::AwaitingUtc);
        store.update(123, AppState::Idle);
        assert_eq!(store.get(123), AppState::Idle);
    }

    #[test]
    fn test_reset_to_idle() {
        let store = DialogueStore::new();
        store.update(123, AppState::AwaitingUtc);
        store.reset(123);
        assert_eq!(store.get(123), AppState::Idle);
    }

    #[test]
    fn test_has() {
        let store = DialogueStore::new();
        assert!(!store.has(123));

        store.update(123, AppState::AwaitingUtc);
        assert!(store.has(123));
    }

    #[test]
    fn test_remove() {
        let store = DialogueStore::new();
        store.update(123, AppState::AwaitingUtc);
        store.remove(123);
        assert!(!store.has(123));
        assert_eq!(store.get(123), AppState::Idle);
    }

    #[test]
    fn test_clone_is_thread_safe() {
        let store = DialogueStore::new();
        store.update(1, AppState::AwaitingUtc);

        let store_clone = store.clone();
        store_clone.update(2, AppState::Idle);

        // Original should still have user 1
        assert_eq!(store.get(1), AppState::AwaitingUtc);
        // Clone should have both
        assert_eq!(store_clone.get(2), AppState::Idle);
    }

    #[test]
    fn test_multiple_users() {
        let store = DialogueStore::new();
        store.update(1, AppState::AwaitingUtc);
        store.update(2, AppState::Idle);
        store.update(
            3,
            AppState::AwaitingReminderConfirmation {
                pending: crate::bot::states::PendingReminder {
                    original_text: "test".to_string(),
                    description: "desc".to_string(),
                    time_display: "tomorrow".to_string(),
                    parsed_json: "{}".to_string(),
                },
            },
        );

        assert_eq!(store.len(), 3);
        assert_eq!(store.get(1), AppState::AwaitingUtc);
        assert_eq!(store.get(2), AppState::Idle);
        assert!(matches!(
            store.get(3),
            AppState::AwaitingReminderConfirmation { .. }
        ));
    }
}
