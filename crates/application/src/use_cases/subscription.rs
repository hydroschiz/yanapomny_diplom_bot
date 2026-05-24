use chrono::Duration;
use domain::{ChatId, Subscription, SubscriptionPolicy};

use crate::{
    ApplicationError, ApplicationResult, Clock, Notification, Notifier, ReminderRepository,
    SchedulerDeduplicationPort, SubscriptionMaintenanceRepository, SubscriptionRepository,
};

pub struct EnsureSubscriptionUseCase<'a, R, C> {
    subscriptions: &'a R,
    clock: &'a C,
    policy: SubscriptionPolicy,
}

impl<'a, R, C> EnsureSubscriptionUseCase<'a, R, C>
where
    R: SubscriptionRepository,
    C: Clock,
{
    pub const fn new(subscriptions: &'a R, clock: &'a C, policy: SubscriptionPolicy) -> Self {
        Self {
            subscriptions,
            clock,
            policy,
        }
    }

    pub async fn execute(&self, chat_id: ChatId) -> ApplicationResult<Subscription> {
        if let Some(subscription) = self.subscriptions.find_subscription(chat_id).await? {
            return Ok(subscription);
        }

        let subscription = Subscription::new_trial(chat_id, self.clock.now(), self.policy);
        self.subscriptions.save_subscription(&subscription).await?;
        Ok(subscription)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionMaintenanceReport {
    pub inspected: usize,
    pub notified: usize,
    pub purged: usize,
    pub reminders_cancelled: usize,
    pub failed: usize,
}

pub struct WarnExpiringSubscriptionsUseCase<'a, S, R, D, N, C> {
    subscriptions: &'a S,
    reminders: &'a R,
    dedupe: &'a D,
    notifier: &'a N,
    clock: &'a C,
    warning_days: i64,
}

impl<'a, S, R, D, N, C> WarnExpiringSubscriptionsUseCase<'a, S, R, D, N, C>
where
    S: SubscriptionMaintenanceRepository,
    R: ReminderRepository,
    D: SchedulerDeduplicationPort,
    N: Notifier,
    C: Clock,
{
    pub const fn new(
        subscriptions: &'a S,
        reminders: &'a R,
        dedupe: &'a D,
        notifier: &'a N,
        clock: &'a C,
        warning_days: i64,
    ) -> Self {
        Self {
            subscriptions,
            reminders,
            dedupe,
            notifier,
            clock,
            warning_days,
        }
    }

    pub async fn execute(&self) -> ApplicationResult<SubscriptionMaintenanceReport> {
        let now = self.clock.now();
        let until = now + Duration::days(self.warning_days.max(1));
        let subscriptions = self
            .subscriptions
            .list_expiring_subscriptions(now, until)
            .await?;
        let mut report = SubscriptionMaintenanceReport {
            inspected: subscriptions.len(),
            notified: 0,
            purged: 0,
            reminders_cancelled: 0,
            failed: 0,
        };

        for subscription in subscriptions {
            let key = format!(
                "subscription:expires:{}:{}",
                subscription.chat_id.value(),
                subscription.expires_at.date_naive()
            );
            let dedupe_until = subscription.expires_at + Duration::days(1);
            if !self.dedupe.once(&key, dedupe_until).await? {
                continue;
            }

            let reminder_count = self
                .reminders
                .list_reminders(subscription.chat_id)
                .await?
                .into_iter()
                .filter(|reminder| !reminder.status.is_terminal())
                .count();
            let text = format_expiring_subscription_message(&subscription, reminder_count, now);
            match self
                .notifier
                .notify(Notification::Text {
                    chat_id: subscription.chat_id,
                    text,
                })
                .await
            {
                Ok(()) => report.notified += 1,
                Err(ApplicationError::ExternalService(_)) => report.failed += 1,
                Err(error) => return Err(error),
            }
        }

        Ok(report)
    }
}

pub struct PurgeExpiredSubscriptionsUseCase<'a, S, R, N, C> {
    subscriptions: &'a S,
    reminders: &'a R,
    notifier: &'a N,
    clock: &'a C,
}

impl<'a, S, R, N, C> PurgeExpiredSubscriptionsUseCase<'a, S, R, N, C>
where
    S: SubscriptionMaintenanceRepository + SubscriptionRepository,
    R: ReminderRepository,
    N: Notifier,
    C: Clock,
{
    pub const fn new(
        subscriptions: &'a S,
        reminders: &'a R,
        notifier: &'a N,
        clock: &'a C,
    ) -> Self {
        Self {
            subscriptions,
            reminders,
            notifier,
            clock,
        }
    }

    pub async fn execute(&self) -> ApplicationResult<SubscriptionMaintenanceReport> {
        let now = self.clock.now();
        let subscriptions = self
            .subscriptions
            .list_expired_active_subscriptions(now)
            .await?;
        let mut report = SubscriptionMaintenanceReport {
            inspected: subscriptions.len(),
            notified: 0,
            purged: 0,
            reminders_cancelled: 0,
            failed: 0,
        };

        for mut subscription in subscriptions {
            let mut cancelled_count = 0;
            for mut reminder in self.reminders.list_reminders(subscription.chat_id).await? {
                if reminder.status.is_terminal() {
                    continue;
                }
                if reminder.cancel().is_ok() {
                    self.reminders.save_reminder(&reminder).await?;
                    cancelled_count += 1;
                }
            }

            subscription.active = false;
            self.subscriptions.save_subscription(&subscription).await?;
            report.purged += 1;
            report.reminders_cancelled += cancelled_count;

            if cancelled_count == 0 {
                continue;
            }

            let text = format_expired_subscription_message(cancelled_count);
            match self
                .notifier
                .notify(Notification::Text {
                    chat_id: subscription.chat_id,
                    text,
                })
                .await
            {
                Ok(()) => report.notified += 1,
                Err(ApplicationError::ExternalService(_)) => report.failed += 1,
                Err(error) => return Err(error),
            }
        }

        Ok(report)
    }
}

fn format_expiring_subscription_message(
    subscription: &Subscription,
    reminder_count: usize,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let days_left = (subscription.expires_at - now).num_days().max(0);
    format!(
        "⚠️ Подписка заканчивается через {}.\n\nАктивных напоминаний: {}.\nПродлите подписку в профиле, чтобы напоминания продолжили приходить.",
        plural_days(days_left),
        reminder_count
    )
}

fn format_expired_subscription_message(cancelled_count: usize) -> String {
    format!(
        "⛔ Подписка закончилась.\n\n{} удалено. Продлите подписку в профиле, чтобы снова создавать напоминания.",
        plural_reminders(cancelled_count)
    )
}

fn plural_days(days: i64) -> String {
    let word = match days % 100 {
        11..=14 => "дней",
        _ => match days % 10 {
            1 => "день",
            2..=4 => "дня",
            _ => "дней",
        },
    };
    format!("{days} {word}")
}

fn plural_reminders(count: usize) -> String {
    let word = match count % 100 {
        11..=14 => "напоминаний",
        _ => match count % 10 {
            1 => "напоминание",
            2..=4 => "напоминания",
            _ => "напоминаний",
        },
    };
    format!("{count} {word}")
}
