use std::{collections::HashMap, sync::Mutex};

use application::{
    active_tasks, CheckTwitchStreamsUseCase, Clock, CompleteTaskUseCase,
    ConsumeReferralRewardUseCase, CreatePaymentCommand, CreatePaymentUseCase,
    CreateReferralUseCase, CreateReminderCommand, CreateReminderUseCase, CreateTaskFromTextUseCase,
    DeliverDueRemindersUseCase, DeliveryEventRepository, DialogState, DialogStateStore,
    ExternalChannelSubscriptionRepository, InterpretedTask, NaturalLanguageInterpreter,
    Notification, Notifier, PaymentGateway, PaymentRepository, ReferralRepository,
    ReminderPreferencesRepository, ReminderRepository, SnoozeReminderUseCase,
    StreamPlatformGateway, TaskRepository, UpdatePreferencesUseCase, UserPreferencesRepository,
    UserRepository,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use domain::{
    ChannelSubscription, ChatId, DeliveryChannel, DeliveryEvent, DeliveryEventId, DeliveryResult,
    Money, Payment, PaymentId, PaymentProvider, PaymentStatus, Platform, RecurrenceRule, Reminder,
    ReminderId, ReminderStatus, RetryPolicy, Schedule, Task, TaskId, TaskStatus, TimePreferences,
    TimeSpec, User, UserId, UserPreferences,
};

#[tokio::test]
async fn task_text_interpretation_and_lifecycle_use_cases_work() {
    let store = AppMemory::new(fixed_now());
    let user = User::new(UserId::new(7));
    store.save_user(&user).await.unwrap();
    store.set_interpretation(
        user.id,
        "напомни купить молоко",
        InterpretedTask {
            title: "купить молоко".to_string(),
            description: Some("из LLM".to_string()),
            schedule: Schedule::OneTime(TimeSpec::default()),
            trigger_at: fixed_now() + Duration::hours(1),
        },
    );

    let task = CreateTaskFromTextUseCase::new(&store, &store, &store, &store)
        .execute(user.id, "напомни купить молоко")
        .await
        .unwrap();

    assert_eq!(task.title, "купить молоко");
    assert_eq!(task.id, Some(TaskId::new(1)));

    let task = CompleteTaskUseCase::new(&store, &store)
        .execute(task.id.unwrap())
        .await
        .unwrap();

    assert_eq!(task.status, TaskStatus::Completed);
    assert!(active_tasks(store.list_tasks(user.id).await.unwrap()).is_empty());
}

#[tokio::test]
async fn reminder_snooze_and_delivery_use_cases_work() {
    let store = AppMemory::new(fixed_now());
    let reminder = CreateReminderUseCase::new(&store)
        .execute(CreateReminderCommand {
            task_id: Some(TaskId::new(1)),
            chat_id: ChatId::new(7),
            text: "купить молоко".to_string(),
            schedule: Schedule::Recurring {
                time: TimeSpec::default(),
                recurrence: RecurrenceRule::default(),
            },
            next_at: fixed_now() - Duration::minutes(1),
        })
        .await
        .unwrap();

    let snoozed = SnoozeReminderUseCase::new(&store, &store)
        .execute(reminder.id.unwrap(), 15)
        .await
        .unwrap();

    assert_eq!(snoozed.next_at, fixed_now() + Duration::minutes(15));

    let due = CreateReminderUseCase::new(&store)
        .execute(CreateReminderCommand {
            task_id: None,
            chat_id: ChatId::new(7),
            text: "due".to_string(),
            schedule: Schedule::OneTime(TimeSpec::default()),
            next_at: fixed_now() - Duration::minutes(1),
        })
        .await
        .unwrap();

    let report = DeliverDueRemindersUseCase::new(
        &store,
        &store,
        &store,
        &store,
        &store,
        RetryPolicy::default(),
        DeliveryChannel::Vk,
    )
    .execute(10)
    .await
    .unwrap();

    assert_eq!(report.claimed, 1);
    assert_eq!(report.delivered, 1);
    assert_eq!(store.notifications().len(), 1);
    assert_eq!(store.events_for(due.id.unwrap()).len(), 1);
    assert_eq!(store.reminder(due.id.unwrap()).status, ReminderStatus::Sent);
}

#[tokio::test]
async fn recurring_reminders_reschedule_after_successful_delivery() {
    let store = AppMemory::new(fixed_now());
    let recurring = CreateReminderUseCase::new(&store)
        .execute(CreateReminderCommand {
            task_id: None,
            chat_id: ChatId::new(7),
            text: "daily".to_string(),
            schedule: Schedule::Recurring {
                time: TimeSpec::default(),
                recurrence: RecurrenceRule::default(),
            },
            next_at: fixed_now() - Duration::minutes(1),
        })
        .await
        .unwrap();

    let report = DeliverDueRemindersUseCase::new(
        &store,
        &store,
        &store,
        &store,
        &store,
        RetryPolicy::default(),
        DeliveryChannel::Vk,
    )
    .execute(10)
    .await
    .unwrap();

    let stored = store.reminder(recurring.id.unwrap());
    let events = store.events_for(recurring.id.unwrap());

    assert_eq!(report.claimed, 1);
    assert_eq!(report.delivered, 1);
    assert_eq!(stored.status, ReminderStatus::Active);
    assert_eq!(stored.next_at, fixed_now() + Duration::days(1));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].result, DeliveryResult::Sent);
}

#[tokio::test]
async fn payment_channel_referral_and_preferences_use_cases_work() {
    let store = AppMemory::new(fixed_now());
    let user_id = UserId::new(7);
    let preferences = UserPreferences::new(user_id);

    let saved_preferences = UpdatePreferencesUseCase::new(&store)
        .execute(preferences.clone())
        .await
        .unwrap();
    assert_eq!(saved_preferences, preferences);

    let created = CreatePaymentUseCase::new(&store, &store, &store)
        .execute(CreatePaymentCommand {
            payment_id: PaymentId::new("payment-1"),
            provider: PaymentProvider::YooKassa,
            amount: Money::rub(195),
        })
        .await
        .unwrap();
    assert!(created.confirmation_url.contains("payment-1"));

    let payment = application::ProcessYooKassaWebhookUseCase::new(&store)
        .execute(&created.payment.id, PaymentStatus::Succeeded)
        .await
        .unwrap();
    assert_eq!(payment.status, PaymentStatus::Succeeded);

    store
        .save_external_channel_subscription(&ChannelSubscription::new(
            user_id,
            Platform::Twitch,
            "channel",
            "Channel",
            "https://twitch.tv/channel",
            1,
            fixed_now(),
        ))
        .await
        .unwrap();
    store.set_latest_content("channel", Some("stream-1".to_string()));
    let changed = CheckTwitchStreamsUseCase::new(&store, &store)
        .execute(user_id)
        .await
        .unwrap();
    assert_eq!(changed.len(), 1);

    let referral = CreateReferralUseCase::new(&store, &store)
        .execute(UserId::new(1), user_id)
        .await
        .unwrap();
    assert!(!referral.is_rewarded());
    assert!(ConsumeReferralRewardUseCase::new(&store, &store)
        .execute(UserId::new(1), user_id)
        .await
        .unwrap()
        .is_some());
    assert!(ConsumeReferralRewardUseCase::new(&store, &store)
        .execute(UserId::new(1), user_id)
        .await
        .unwrap()
        .is_none());
}

#[derive(Default)]
struct AppState {
    users: HashMap<UserId, User>,
    preferences: HashMap<UserId, UserPreferences>,
    tasks: HashMap<TaskId, Task>,
    next_task_id: i64,
    reminders: HashMap<ReminderId, Reminder>,
    next_reminder_id: i32,
    events: HashMap<ReminderId, Vec<DeliveryEvent>>,
    next_event_id: i64,
    payments: HashMap<PaymentId, Payment>,
    channels: HashMap<UserId, Vec<ChannelSubscription>>,
    latest_content: HashMap<String, Option<String>>,
    referrals: HashMap<(UserId, UserId), domain::Referral>,
    dialog_states: HashMap<UserId, DialogState>,
    interpretations: HashMap<(UserId, String), InterpretedTask>,
    notifications: Vec<Notification>,
}

struct AppMemory {
    now: DateTime<Utc>,
    state: Mutex<AppState>,
}

impl AppMemory {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            now,
            state: Mutex::new(AppState::default()),
        }
    }

    fn set_interpretation(&self, user_id: UserId, text: &str, interpretation: InterpretedTask) {
        self.state
            .lock()
            .unwrap()
            .interpretations
            .insert((user_id, text.to_string()), interpretation);
    }

    fn set_latest_content(&self, channel_id: &str, content_id: Option<String>) {
        self.state
            .lock()
            .unwrap()
            .latest_content
            .insert(channel_id.to_string(), content_id);
    }

    fn notifications(&self) -> Vec<Notification> {
        self.state.lock().unwrap().notifications.clone()
    }

    fn events_for(&self, reminder_id: ReminderId) -> Vec<DeliveryEvent> {
        self.state
            .lock()
            .unwrap()
            .events
            .get(&reminder_id)
            .cloned()
            .unwrap_or_default()
    }

    fn reminder(&self, reminder_id: ReminderId) -> Reminder {
        self.state
            .lock()
            .unwrap()
            .reminders
            .get(&reminder_id)
            .cloned()
            .unwrap()
    }
}

impl Clock for AppMemory {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

#[async_trait]
impl UserRepository for AppMemory {
    async fn find_user(&self, id: UserId) -> application::ApplicationResult<Option<User>> {
        Ok(self.state.lock().unwrap().users.get(&id).cloned())
    }

    async fn save_user(&self, user: &User) -> application::ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .users
            .insert(user.id, user.clone());
        Ok(())
    }
}

#[async_trait]
impl UserPreferencesRepository for AppMemory {
    async fn find_preferences(
        &self,
        user_id: UserId,
    ) -> application::ApplicationResult<Option<UserPreferences>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .preferences
            .get(&user_id)
            .cloned())
    }

    async fn save_preferences(
        &self,
        preferences: &UserPreferences,
    ) -> application::ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .preferences
            .insert(preferences.user_id, preferences.clone());
        Ok(())
    }
}

#[async_trait]
impl TaskRepository for AppMemory {
    async fn create_task(&self, mut task: Task) -> application::ApplicationResult<Task> {
        let mut state = self.state.lock().unwrap();
        state.next_task_id += 1;
        task.assign_id(TaskId::new(state.next_task_id));
        state.tasks.insert(task.id.unwrap(), task.clone());
        Ok(task)
    }

    async fn find_task(&self, id: TaskId) -> application::ApplicationResult<Option<Task>> {
        Ok(self.state.lock().unwrap().tasks.get(&id).cloned())
    }

    async fn list_tasks(&self, user_id: UserId) -> application::ApplicationResult<Vec<Task>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .tasks
            .values()
            .filter(|task| task.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn save_task(&self, task: &Task) -> application::ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .tasks
            .insert(task.id.unwrap(), task.clone());
        Ok(())
    }
}

#[async_trait]
impl ReminderRepository for AppMemory {
    async fn create_reminder(
        &self,
        mut reminder: Reminder,
    ) -> application::ApplicationResult<Reminder> {
        let mut state = self.state.lock().unwrap();
        state.next_reminder_id += 1;
        reminder.assign_id(ReminderId::new(state.next_reminder_id));
        state
            .reminders
            .insert(reminder.id.unwrap(), reminder.clone());
        Ok(reminder)
    }

    async fn find_reminder(
        &self,
        id: ReminderId,
    ) -> application::ApplicationResult<Option<Reminder>> {
        Ok(self.state.lock().unwrap().reminders.get(&id).cloned())
    }

    async fn save_reminder(&self, reminder: &Reminder) -> application::ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .reminders
            .insert(reminder.id.unwrap(), reminder.clone());
        Ok(())
    }

    async fn claim_due_reminders(
        &self,
        now: DateTime<Utc>,
        batch_size: usize,
    ) -> application::ApplicationResult<Vec<Reminder>> {
        let mut state = self.state.lock().unwrap();
        let mut claimed = Vec::new();
        for reminder in state.reminders.values_mut() {
            if claimed.len() >= batch_size {
                break;
            }
            if reminder.claim(now).is_ok() {
                claimed.push(reminder.clone());
            }
        }
        Ok(claimed)
    }
}

#[async_trait]
impl ReminderPreferencesRepository for AppMemory {
    async fn find_time_preferences_for_chat(
        &self,
        chat_id: ChatId,
    ) -> application::ApplicationResult<TimePreferences> {
        let state = self.state.lock().unwrap();
        let direct_user_id = UserId::new(chat_id.value());
        if let Some(preferences) = state.preferences.get(&direct_user_id) {
            return Ok(preferences.time_preferences.clone());
        }

        let user = state.users.values().find(|user| {
            user.id.value() == chat_id.value()
                || user
                    .identities
                    .iter()
                    .any(|identity| identity.chat_id == Some(chat_id))
        });

        Ok(user
            .and_then(|user| state.preferences.get(&user.id))
            .map(|preferences| preferences.time_preferences.clone())
            .or_else(|| user.map(|user| user.time_preferences.clone()))
            .unwrap_or_default())
    }
}

#[async_trait]
impl DeliveryEventRepository for AppMemory {
    async fn create_delivery_event(
        &self,
        mut event: DeliveryEvent,
    ) -> application::ApplicationResult<DeliveryEvent> {
        let mut state = self.state.lock().unwrap();
        state.next_event_id += 1;
        event.assign_id(DeliveryEventId::new(state.next_event_id));
        state
            .events
            .entry(event.reminder_id)
            .or_default()
            .push(event.clone());
        Ok(event)
    }

    async fn save_delivery_event(
        &self,
        event: &DeliveryEvent,
    ) -> application::ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .events
            .entry(event.reminder_id)
            .or_default()
            .push(event.clone());
        Ok(())
    }

    async fn list_delivery_events(
        &self,
        reminder_id: ReminderId,
    ) -> application::ApplicationResult<Vec<DeliveryEvent>> {
        Ok(self.events_for(reminder_id))
    }
}

#[async_trait]
impl PaymentRepository for AppMemory {
    async fn find_payment(
        &self,
        payment_id: &PaymentId,
    ) -> application::ApplicationResult<Option<Payment>> {
        Ok(self.state.lock().unwrap().payments.get(payment_id).cloned())
    }

    async fn save_payment(&self, payment: &Payment) -> application::ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .payments
            .insert(payment.id.clone(), payment.clone());
        Ok(())
    }
}

#[async_trait]
impl PaymentGateway for AppMemory {
    async fn create_payment(&self, payment: &Payment) -> application::ApplicationResult<String> {
        Ok(format!("https://pay.example/{}", payment.id))
    }
}

#[async_trait]
impl ExternalChannelSubscriptionRepository for AppMemory {
    async fn list_external_channel_subscriptions(
        &self,
        user_id: UserId,
    ) -> application::ApplicationResult<Vec<ChannelSubscription>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .channels
            .get(&user_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn save_external_channel_subscription(
        &self,
        subscription: &ChannelSubscription,
    ) -> application::ApplicationResult<()> {
        let mut state = self.state.lock().unwrap();
        let subscriptions = state.channels.entry(subscription.user_id).or_default();
        if let Some(existing) = subscriptions
            .iter_mut()
            .find(|existing| existing.sub_num == subscription.sub_num)
        {
            *existing = subscription.clone();
        } else {
            subscriptions.push(subscription.clone());
        }
        Ok(())
    }
}

#[async_trait]
impl StreamPlatformGateway for AppMemory {
    async fn latest_content_id(
        &self,
        subscription: &ChannelSubscription,
    ) -> application::ApplicationResult<Option<String>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .latest_content
            .get(&subscription.channel_id)
            .cloned()
            .unwrap_or_default())
    }
}

#[async_trait]
impl ReferralRepository for AppMemory {
    async fn find_referral(
        &self,
        referrer_id: UserId,
        invited_id: UserId,
    ) -> application::ApplicationResult<Option<domain::Referral>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .referrals
            .get(&(referrer_id, invited_id))
            .cloned())
    }

    async fn save_referral(
        &self,
        referral: &domain::Referral,
    ) -> application::ApplicationResult<()> {
        self.state.lock().unwrap().referrals.insert(
            (referral.referrer_id, referral.invited_id),
            referral.clone(),
        );
        Ok(())
    }
}

#[async_trait]
impl NaturalLanguageInterpreter for AppMemory {
    async fn interpret_task(
        &self,
        text: &str,
        user: &User,
    ) -> application::ApplicationResult<InterpretedTask> {
        self.state
            .lock()
            .unwrap()
            .interpretations
            .get(&(user.id, text.to_string()))
            .cloned()
            .ok_or_else(|| {
                application::ApplicationError::ExternalService("missing interpretation".to_string())
            })
    }
}

#[async_trait]
impl Notifier for AppMemory {
    async fn notify(&self, notification: Notification) -> application::ApplicationResult<()> {
        self.state.lock().unwrap().notifications.push(notification);
        Ok(())
    }
}

#[async_trait]
impl DialogStateStore for AppMemory {
    async fn get_state(&self, user_id: UserId) -> application::ApplicationResult<DialogState> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .dialog_states
            .get(&user_id)
            .cloned()
            .unwrap_or(DialogState::Idle))
    }

    async fn set_state(
        &self,
        user_id: UserId,
        state: DialogState,
    ) -> application::ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .dialog_states
            .insert(user_id, state);
        Ok(())
    }
}

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap()
}
