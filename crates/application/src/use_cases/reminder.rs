use chrono::{DateTime, Utc};
use domain::{
    ChatId, DeliveryChannel, DeliveryEvent, Reminder, ReminderId, RetryPolicy, Schedule, Task,
    TaskId, TaskPriority, User, UserId,
};

use crate::{
    ApplicationError, ApplicationResult, Clock, DeliveryEventRepository,
    NaturalLanguageInterpreter, Notification, Notifier, ReminderPreferencesRepository,
    ReminderRepository, TaskRepository, UserRepository,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateReminderFromTextCommand {
    pub user_id: UserId,
    pub chat_id: ChatId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedReminderFromText {
    pub task: Task,
    pub reminder: Reminder,
}

pub struct CreateReminderFromTextUseCase<'a, U, T, R, I, C> {
    users: &'a U,
    tasks: &'a T,
    reminders: &'a R,
    interpreter: &'a I,
    clock: &'a C,
}

impl<'a, U, T, R, I, C> CreateReminderFromTextUseCase<'a, U, T, R, I, C>
where
    U: UserRepository,
    T: TaskRepository,
    R: ReminderRepository,
    I: NaturalLanguageInterpreter,
    C: Clock,
{
    pub const fn new(
        users: &'a U,
        tasks: &'a T,
        reminders: &'a R,
        interpreter: &'a I,
        clock: &'a C,
    ) -> Self {
        Self {
            users,
            tasks,
            reminders,
            interpreter,
            clock,
        }
    }

    pub async fn execute(
        &self,
        command: CreateReminderFromTextCommand,
    ) -> ApplicationResult<CreatedReminderFromText> {
        let user = self.ensure_user(command.user_id).await?;
        let interpreted = self
            .interpreter
            .interpret_task(&command.text, &user)
            .await?;
        let now = self.clock.now();
        let mut task = Task::new(command.user_id, interpreted.title.clone(), now);
        task.description = interpreted.description;
        task.priority = TaskPriority::Normal;
        task.due_at = Some(interpreted.trigger_at);
        let task = self.tasks.create_task(task).await?;

        let mut reminder = Reminder::new(
            command.chat_id,
            interpreted.title,
            interpreted.schedule,
            interpreted.trigger_at,
        );
        if let Some(task_id) = task.id {
            reminder.attach_task(task_id);
        }
        let reminder = self.reminders.create_reminder(reminder).await?;

        Ok(CreatedReminderFromText { task, reminder })
    }

    async fn ensure_user(&self, user_id: UserId) -> ApplicationResult<User> {
        if let Some(user) = self.users.find_user(user_id).await? {
            return Ok(user);
        }

        let user = User::new(user_id);
        self.users.save_user(&user).await?;
        Ok(user)
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
