use chrono::{DateTime, Utc};
use domain::{
    ChatId, DeliveryChannel, DeliveryEvent, Reminder, ReminderId, RetryPolicy, Schedule, TaskId,
};

use crate::{
    ApplicationError, ApplicationResult, Clock, DeliveryEventRepository, Notification, Notifier,
    ReminderPreferencesRepository, ReminderRepository,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateReminderCommand {
    pub task_id: Option<TaskId>,
    pub chat_id: ChatId,
    pub text: String,
    pub schedule: Schedule,
    pub next_at: DateTime<Utc>,
}

pub struct CreateReminderUseCase<'a, R> {
    reminders: &'a R,
}

impl<'a, R> CreateReminderUseCase<'a, R>
where
    R: ReminderRepository,
{
    pub const fn new(reminders: &'a R) -> Self {
        Self { reminders }
    }

    pub async fn execute(&self, command: CreateReminderCommand) -> ApplicationResult<Reminder> {
        let mut reminder = Reminder::new(
            command.chat_id,
            command.text,
            command.schedule,
            command.next_at,
        );
        if let Some(task_id) = command.task_id {
            reminder.attach_task(task_id);
        }
        self.reminders.create_reminder(reminder).await
    }
}

pub struct SnoozeReminderUseCase<'a, R, C> {
    reminders: &'a R,
    clock: &'a C,
}

impl<'a, R, C> SnoozeReminderUseCase<'a, R, C>
where
    R: ReminderRepository,
    C: Clock,
{
    pub const fn new(reminders: &'a R, clock: &'a C) -> Self {
        Self { reminders, clock }
    }

    pub async fn execute(
        &self,
        reminder_id: ReminderId,
        minutes: i64,
    ) -> ApplicationResult<Reminder> {
        let mut reminder = self
            .reminders
            .find_reminder(reminder_id)
            .await?
            .ok_or_else(|| ApplicationError::NotFound {
                entity: "reminder",
                id: reminder_id.to_string(),
            })?;
        reminder.snooze(self.clock.now(), minutes)?;
        self.reminders.save_reminder(&reminder).await?;
        Ok(reminder)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryReport {
    pub claimed: usize,
    pub delivered: usize,
    pub failed: usize,
}

pub struct DeliverDueRemindersUseCase<'a, R, D, P, N, C> {
    reminders: &'a R,
    delivery_events: &'a D,
    preferences: &'a P,
    notifier: &'a N,
    clock: &'a C,
    retry_policy: RetryPolicy,
    delivery_channel: DeliveryChannel,
}

impl<'a, R, D, P, N, C> DeliverDueRemindersUseCase<'a, R, D, P, N, C>
where
    R: ReminderRepository,
    D: DeliveryEventRepository,
    P: ReminderPreferencesRepository,
    N: Notifier,
    C: Clock,
{
    pub const fn new(
        reminders: &'a R,
        delivery_events: &'a D,
        preferences: &'a P,
        notifier: &'a N,
        clock: &'a C,
        retry_policy: RetryPolicy,
        delivery_channel: DeliveryChannel,
    ) -> Self {
        Self {
            reminders,
            delivery_events,
            preferences,
            notifier,
            clock,
            retry_policy,
            delivery_channel,
        }
    }

    pub async fn execute(&self, batch_size: usize) -> ApplicationResult<DeliveryReport> {
        let now = self.clock.now();
        let due = self.reminders.claim_due_reminders(now, batch_size).await?;
        let mut report = DeliveryReport {
            claimed: due.len(),
            ..DeliveryReport::default()
        };

        for mut reminder in due {
            let reminder_id = reminder.id.ok_or_else(|| {
                ApplicationError::Repository("claimed reminder has no id".to_string())
            })?;
            let mut event =
                DeliveryEvent::planned(reminder_id, self.delivery_channel, reminder.next_at);

            let result = self
                .notifier
                .notify(Notification::Text {
                    chat_id: reminder.chat_id,
                    text: reminder.text.clone(),
                })
                .await;

            match result {
                Ok(()) => {
                    let preferences = self
                        .preferences
                        .find_time_preferences_for_chat(reminder.chat_id)
                        .await?;
                    reminder.next_after_send(now, &preferences)?;
                    event.mark_sent(now);
                    report.delivered += 1;
                }
                Err(err) => {
                    let _ = reminder.schedule_retry(self.retry_policy, now);
                    event.mark_temporary_failure(Some(err.to_string()));
                    report.failed += 1;
                }
            }

            self.reminders.save_reminder(&reminder).await?;
            self.delivery_events.create_delivery_event(event).await?;
        }

        Ok(report)
    }
}
