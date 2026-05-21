use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc, Weekday as ChronoWeekday};
use domain::{
    scheduling::{calculate_from_time_spec, days_in_month},
    ChatId, CommunicationPlatform, DeliveryChannel, DeliveryEvent, DeliveryResult, DomainError,
    FreeState, Language, Money, Months, Payment, PaymentId, PaymentProvider, PlatformIdentity,
    RecurrenceFilter, RecurrencePattern, RecurrenceRule, Reminder, ReminderId, ReminderStatus,
    RetryPolicy, Schedule, Subscription, SubscriptionId, SubscriptionPolicy, SubscriptionSource,
    SubscriptionStatus, Task, TaskId, TaskPriority, TaskStatus, TimeOfDay, TimePreferences,
    TimeSpec, TimeSpecType, User, UserId, UserStatus, UtcOffset, Weekday,
};

#[test]
fn utc_offset_parses_common_formats() {
    assert_eq!("+07:00".parse::<UtcOffset>().unwrap().seconds(), 7 * 3600);
    assert_eq!("UTC+7".parse::<UtcOffset>().unwrap().to_string(), "+07:00");
    assert_eq!("GMT-05:30".parse::<UtcOffset>().unwrap().seconds(), -19_800);
    assert_eq!("Z".parse::<UtcOffset>().unwrap(), UtcOffset::UTC);
}

#[test]
fn utc_offset_rejects_invalid_values() {
    assert!(matches!(
        "+14:30".parse::<UtcOffset>(),
        Err(DomainError::InvalidUtcOffset { .. })
    ));
    assert!(matches!(
        "+15:00".parse::<UtcOffset>(),
        Err(DomainError::InvalidUtcOffset { .. })
    ));
}

#[test]
fn scheduling_converts_user_local_time_to_utc() {
    let now = Utc.with_ymd_and_hms(2025, 12, 11, 3, 0, 0).unwrap();
    let prefs =
        TimePreferences::from_fixed_offset_strings("8:00", "14:00", "19:00", "+07:00").unwrap();
    let spec = TimeSpec {
        spec_type: TimeSpecType::Relative,
        anchor: Some("today".to_string()),
        offset_days: 1,
        time: Some("09:30".to_string()),
        ..Default::default()
    };

    let result = calculate_from_time_spec(&spec, now, &prefs).unwrap();

    assert_eq!(result.year(), 2025);
    assert_eq!(result.month(), 12);
    assert_eq!(result.day(), 12);
    assert_eq!(result.hour(), 2);
    assert_eq!(result.minute(), 30);
}

#[test]
fn scheduling_calculates_weekday_and_time_of_day() {
    let now = Utc.with_ymd_and_hms(2025, 5, 14, 10, 0, 0).unwrap();
    let prefs =
        TimePreferences::from_fixed_offset_strings("09:30", "14:00", "19:00", "+00:00").unwrap();
    let spec = TimeSpec {
        spec_type: TimeSpecType::Weekday,
        weekday: Some(Weekday::Monday),
        time_of_day: Some(TimeOfDay::Morning),
        ..Default::default()
    };

    let result = calculate_from_time_spec(&spec, now, &prefs).unwrap();

    assert_eq!(result.weekday(), ChronoWeekday::Mon);
    assert_eq!(result.hour(), 9);
    assert_eq!(result.minute(), 30);
}

#[test]
fn scheduling_helpers_handle_month_lengths() {
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(2023, 2), 28);
    assert_eq!(days_in_month(2023, 4), 30);
}

#[test]
fn recurrence_rule_preserves_legacy_delay_mapping() {
    let weekdays = RecurrenceRule {
        pattern: RecurrencePattern::Daily,
        filters: vec![RecurrenceFilter::Weekdays],
        ..Default::default()
    };
    assert_eq!(weekdays.to_legacy_delay(), "weekday");

    let monthly = RecurrenceRule {
        pattern: RecurrencePattern::Monthly,
        ..Default::default()
    };
    assert_eq!(monthly.to_legacy_delay(), "month");
}

#[test]
fn reminder_claim_retry_and_snooze_transitions() {
    let now = fixed_now();
    let schedule = Schedule::OneTime(TimeSpec::default());
    let mut reminder = Reminder::new(
        ChatId::new(10),
        "test",
        schedule,
        now - Duration::minutes(1),
    );

    reminder.claim(now).unwrap();
    assert_eq!(reminder.status, ReminderStatus::Processing);

    let retry_at = reminder
        .schedule_retry(RetryPolicy::default(), now)
        .unwrap();
    assert_eq!(retry_at, now + Duration::seconds(30));
    assert!(matches!(
        reminder.status,
        ReminderStatus::Retry { attempt: 1, .. }
    ));

    let snoozed_until = reminder.snooze(now, 15).unwrap();
    assert_eq!(snoozed_until, now + Duration::minutes(15));
    assert_eq!(reminder.status, ReminderStatus::Active);
    assert_eq!(reminder.retry_count, 0);
}

#[test]
fn reminder_rejects_claim_before_due_time() {
    let now = fixed_now();
    let schedule = Schedule::OneTime(TimeSpec::default());
    let mut reminder = Reminder::new(
        ChatId::new(10),
        "test",
        schedule,
        now + Duration::minutes(1),
    );

    assert!(matches!(
        reminder.claim(now),
        Err(DomainError::ReminderNotDue { .. })
    ));
}

#[test]
fn user_identity_and_preferences_match_target_model() {
    let now = fixed_now();
    let mut user = User::registered(UserId::new(42), now);
    let identity = PlatformIdentity::new(
        user.id,
        CommunicationPlatform::Vk,
        "vk-42",
        Some(ChatId::new(42)),
        now,
    );

    user.add_identity(identity.clone());
    user.add_identity(identity);

    assert_eq!(user.status, UserStatus::Active);
    assert_eq!(user.created_at, Some(now));
    assert_eq!(user.identities.len(), 1);

    let preferences = user.preferences();
    assert_eq!(preferences.user_id, user.id);
    assert_eq!(preferences.language, Language::Russian);
    assert!(preferences.notification_policy.enabled);
}

#[test]
fn task_reminder_and_delivery_event_lifecycle_are_separate() {
    let now = fixed_now();
    let due_at = now + Duration::hours(2);
    let mut task = Task::new(UserId::new(7), "buy milk", now);
    task.assign_id(TaskId::new(100));
    task.set_priority(TaskPriority::High, now);
    task.set_due_at(Some(due_at), now);

    let task_id = task.id.unwrap();
    let mut reminder = Reminder::new(
        ChatId::new(7),
        "buy milk",
        Schedule::OneTime(TimeSpec::default()),
        due_at,
    );
    reminder.assign_id(ReminderId::new(500));
    reminder.attach_task(task_id);

    let mut delivery =
        DeliveryEvent::planned(reminder.id.unwrap(), DeliveryChannel::Vk, reminder.next_at);
    delivery.mark_sent(due_at);

    assert_eq!(task.status, TaskStatus::Active);
    assert_eq!(task.priority, TaskPriority::High);
    assert_eq!(reminder.task_id, Some(task_id));
    assert_eq!(delivery.result, DeliveryResult::Sent);

    task.complete(now).unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    assert!(matches!(
        task.complete(now),
        Err(DomainError::InvalidStatusTransition { .. })
    ));
}

#[test]
fn subscription_trial_and_extension_follow_policy() {
    let now = fixed_now();
    let mut subscription =
        Subscription::new_trial(ChatId::new(10), now, SubscriptionPolicy { trial_days: 7 });

    assert!(subscription.is_active(now));
    assert!(matches!(
        subscription.status(now),
        SubscriptionStatus::Trial { .. }
    ));

    let new_expiry = subscription.extend(Months::THREE, now);

    assert_eq!(subscription.free_state, FreeState::Paid);
    assert_eq!(subscription.source, SubscriptionSource::Payment);
    assert!(new_expiry > now + Duration::days(90));
    assert!(matches!(
        subscription.status(now),
        SubscriptionStatus::Active { .. }
    ));
}

#[test]
fn subscription_and_payment_can_be_linked_without_provider_leakage() {
    let now = fixed_now();
    let mut subscription =
        Subscription::new_trial(ChatId::new(42), now, SubscriptionPolicy::default());
    subscription.assign_id(SubscriptionId::new(77));
    subscription.link_user(UserId::new(42));

    let mut payment = Payment::new(
        PaymentId::new("pay-1"),
        PaymentProvider::YooKassa,
        Money::rub(195),
        now,
    );
    payment.link_subscription(subscription.id.unwrap());
    payment.set_provider_payment_id("yk-1");

    assert_eq!(subscription.user_id, Some(UserId::new(42)));
    assert_eq!(payment.subscription_id, subscription.id);
    assert_eq!(payment.provider.to_string(), "yookassa");
}

#[test]
fn subscription_marks_group_owner() {
    let now = fixed_now();
    let mut subscription =
        Subscription::new_trial(ChatId::new(-100), now, SubscriptionPolicy::default());

    subscription.mark_group("Group", UserId::new(42));

    assert!(subscription.is_group);
    assert_eq!(subscription.owner_id, Some(UserId::new(42)));
}

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 5, 14, 12, 0, 0).unwrap()
}
