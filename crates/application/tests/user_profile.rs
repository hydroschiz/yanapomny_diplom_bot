use std::{collections::HashMap, sync::Mutex};

use application::{
    Clock, EnsureSubscriptionUseCase, EnsureUserUseCase, GetProfileUseCase, SetAutoSnoozeUseCase,
    SetSnoozeButtonsUseCase, SetUserTimezoneUseCase, SubscriptionRepository, UserRepository,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use domain::{
    ChatId, SnoozeDuration, Subscription, SubscriptionPolicy, SubscriptionStatus, TimePreferences,
    User, UserId,
};

#[tokio::test]
async fn ensure_user_creates_missing_user_once() {
    let users = InMemoryUsers::default();
    let use_case = EnsureUserUseCase::new(&users);
    let user_id = UserId::new(42);

    let first = use_case.execute(user_id).await.unwrap();
    let second = use_case.execute(user_id).await.unwrap();

    assert_eq!(first.id, user_id);
    assert_eq!(second.id, user_id);
    assert_eq!(users.len(), 1);
}

#[tokio::test]
async fn set_user_timezone_creates_or_updates_user_preferences() {
    let users = InMemoryUsers::default();
    let use_case = SetUserTimezoneUseCase::new(&users);
    let preferences =
        TimePreferences::from_fixed_offset_strings("9:00", "15:00", "20:00", "+03:00").unwrap();

    let user = use_case
        .execute(UserId::new(7), preferences.clone())
        .await
        .unwrap();

    assert_eq!(user.time_preferences, preferences);
    assert_eq!(
        users.get(UserId::new(7)).unwrap().time_preferences,
        preferences
    );
}

#[tokio::test]
async fn set_snooze_preferences_updates_existing_user() {
    let users = InMemoryUsers::default();
    EnsureUserUseCase::new(&users)
        .execute(UserId::new(1))
        .await
        .unwrap();

    let buttons = vec![SnoozeDuration::FIVE_MINUTES, SnoozeDuration::ONE_HOUR];
    let user = SetSnoozeButtonsUseCase::new(&users)
        .execute(UserId::new(1), buttons.clone())
        .await
        .unwrap();
    let user = SetAutoSnoozeUseCase::new(&users)
        .execute(user.id, SnoozeDuration::from_minutes(30))
        .await
        .unwrap();

    assert_eq!(user.snooze_buttons, buttons);
    assert_eq!(user.auto_snooze, SnoozeDuration::from_minutes(30));
}

#[tokio::test]
async fn ensure_subscription_creates_trial_with_policy() {
    let subscriptions = InMemorySubscriptions::default();
    let clock = FixedClock::new(fixed_now());
    let use_case = EnsureSubscriptionUseCase::new(
        &subscriptions,
        &clock,
        SubscriptionPolicy { trial_days: 14 },
    );

    let subscription = use_case.execute(ChatId::new(100)).await.unwrap();

    assert_eq!(subscription.expires_at, fixed_now() + Duration::days(14));
    assert_eq!(subscriptions.len(), 1);
}

#[tokio::test]
async fn get_profile_ensures_user_and_reads_subscription_status() {
    let users = InMemoryUsers::default();
    let subscriptions = InMemorySubscriptions::default();
    let clock = FixedClock::new(fixed_now());
    let subscription =
        Subscription::new_trial(ChatId::new(10), fixed_now(), SubscriptionPolicy::default());
    subscriptions
        .save_subscription(&subscription)
        .await
        .unwrap();

    let profile = GetProfileUseCase::new(&users, &subscriptions, &clock)
        .execute(UserId::new(10), ChatId::new(10))
        .await
        .unwrap();

    assert_eq!(profile.user.id, UserId::new(10));
    assert!(matches!(
        profile.subscription_status,
        Some(SubscriptionStatus::Trial { .. })
    ));
}

#[derive(Default)]
struct InMemoryUsers {
    users: Mutex<HashMap<UserId, User>>,
}

impl InMemoryUsers {
    fn len(&self) -> usize {
        self.users.lock().unwrap().len()
    }

    fn get(&self, id: UserId) -> Option<User> {
        self.users.lock().unwrap().get(&id).cloned()
    }
}

#[async_trait]
impl UserRepository for InMemoryUsers {
    async fn find_user(&self, id: UserId) -> application::ApplicationResult<Option<User>> {
        Ok(self.users.lock().unwrap().get(&id).cloned())
    }

    async fn save_user(&self, user: &User) -> application::ApplicationResult<()> {
        self.users.lock().unwrap().insert(user.id, user.clone());
        Ok(())
    }
}

#[derive(Default)]
struct InMemorySubscriptions {
    subscriptions: Mutex<HashMap<ChatId, Subscription>>,
}

impl InMemorySubscriptions {
    fn len(&self) -> usize {
        self.subscriptions.lock().unwrap().len()
    }
}

#[async_trait]
impl SubscriptionRepository for InMemorySubscriptions {
    async fn find_subscription(
        &self,
        chat_id: ChatId,
    ) -> application::ApplicationResult<Option<Subscription>> {
        Ok(self.subscriptions.lock().unwrap().get(&chat_id).cloned())
    }

    async fn save_subscription(
        &self,
        subscription: &Subscription,
    ) -> application::ApplicationResult<()> {
        self.subscriptions
            .lock()
            .unwrap()
            .insert(subscription.chat_id, subscription.clone());
        Ok(())
    }
}

struct FixedClock {
    now: DateTime<Utc>,
}

impl FixedClock {
    const fn new(now: DateTime<Utc>) -> Self {
        Self { now }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap()
}
