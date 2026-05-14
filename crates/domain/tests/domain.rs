use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc, Weekday as ChronoWeekday};
use domain::{
    scheduling::{calculate_from_time_spec, days_in_month},
    ChatId, DomainError, FreeState, Months, RecurrenceFilter, RecurrencePattern, RecurrenceRule,
    Reminder, ReminderStatus, RetryPolicy, Schedule, Subscription, SubscriptionPolicy,
    SubscriptionStatus, TimeOfDay, TimePreferences, TimeSpec, TimeSpecType, UserId, UtcOffset,
    Weekday,
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
    assert!(new_expiry > now + Duration::days(90));
    assert!(matches!(
        subscription.status(now),
        SubscriptionStatus::Active { .. }
    ));
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
