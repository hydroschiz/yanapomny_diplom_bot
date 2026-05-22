# MongoDB Schema

Phase 9.9 is a clean MongoDB schema reset. Production services read and write the collections described here; legacy collection names and field names from root `src/*` are migration-source material only.

The physical schema is allowed to differ from the domain structs when that makes MongoDB access safer or more direct. Domain/application still own the logical model; infrastructure owns BSON shape, indexes, and persistence-specific fields.

## Collections

### `users`

User identity, status, transport identities, and embedded preferences.

```javascript
{
  _id: NumberLong,                  // domain UserId
  status: "active|blocked|deleted",
  created_at: Date | null,
  identities: [
    {
      platform: "vk",
      external_id: String,
      chat_id: NumberLong | null,
      connected_at: Date
    }
  ],
  preferences: {
    time_preferences: {
      morning: "HH:MM",
      afternoon: "HH:MM",
      evening: "HH:MM",
      utc_offset_seconds: NumberInt,
      time_zone: String | null
    },
    language: "ru",
    snooze_policy: {
      buttons: [NumberInt],
      auto_snooze: NumberInt
    },
    notification_policy: {
      enabled: Boolean
    }
  },
  payment_info: String | null
}
```

`user_preferences` is not a separate production collection after the reset. The `UserPreferencesRepository` maps to `users.preferences`.

Indexes:

- `_id` unique by MongoDB.
- Unique sparse `{ "identities.platform": 1, "identities.external_id": 1 }`.
- Sparse `{ "identities.chat_id": 1 }`.

### `chats`

Reserved slot for chat-scoped delivery preferences once application exposes a `ChatRepository`.

```javascript
{
  _id: NumberLong,                  // domain ChatId / peer id
  platform: "vk",
  kind: "direct|group",
  title: String | null,
  owner_user_id: NumberLong | null,
  preferences: { ... },
  created_at: Date,
  updated_at: Date
}
```

Current Phase 9.9 runtime does not write this collection because no application port owns chat aggregates yet.

### `tasks`

Optional user-owned planning items. Reminders are the primary delivery aggregate; tasks remain separate only for task-manager scenarios already represented in application ports.

```javascript
{
  _id: NumberLong,                  // domain TaskId
  user_id: NumberLong,
  title: String,
  description: String | null,
  status: "active|completed|deleted",
  priority: "low|normal|high",
  due_at: Date | null,
  created_at: Date,
  updated_at: Date
}
```

Indexes:

- `_id` unique by MongoDB.
- `{ user_id: 1, status: 1 }`.
- `{ due_at: 1, status: 1 }`.

### `reminders`

Reminder-first scheduled delivery aggregate.

```javascript
{
  _id: NumberInt,                   // domain ReminderId
  task_id: NumberLong | null,
  chat_id: NumberLong,
  text: String,
  schedule: {
    kind: "one_time|recurring",
    time: {
      type: "relative|weekday|absolute|monthly|yearly|daily",
      anchor: String | null,
      offset_minutes: NumberInt,
      offset_hours: NumberInt,
      offset_days: NumberInt,
      offset_weeks: NumberInt,
      offset_months: NumberInt,
      offset_years: NumberInt,
      offset_direction: "after|before" | null,
      weekday: String | null,
      date: String | null,
      day_of_month: NumberInt,
      week_of_month: NumberInt,
      day_position: String | null,
      time: String | null,
      time_of_day: String | null
    },
    recurrence: {
      pattern: "daily|weekly|monthly|yearly|custom",
      interval: NumberInt,
      filters: [String],
      interval_unit: "days|weeks|months|years" | null,
      week_of_month: NumberInt,
      day_position: String | null
    } | null
  },
  next_at: Date,
  status: "active|processing|retry|sent|failed",
  message_id: NumberInt | null,
  snooze_until: Date | null,
  retry_count: NumberInt,
  retry_at: Date | null
}
```

Indexes:

- `_id` unique by MongoDB.
- `{ status: 1, next_at: 1 }`.
- `{ status: 1, retry_at: 1 }`.
- `{ chat_id: 1, status: 1 }`.

### `delivery_events`

Append-only delivery attempt/result log.

```javascript
{
  _id: NumberLong,                  // domain DeliveryEventId
  reminder_id: NumberInt,
  channel: "vk",
  planned_at: Date,
  sent_at: Date | null,
  result: "planned|sent|temporary_failure|permanent_failure",
  error_code: String | null
}
```

Indexes:

- `_id` unique by MongoDB.
- `{ reminder_id: 1, planned_at: 1 }`.

### `subscriptions`

Access state keyed by subject. Current runtime writes only `subject_type: "chat"` because `SubscriptionRepository` is chat-based.

```javascript
{
  subscription_id: NumberLong | null,
  subject_type: "chat|user",
  subject_id: NumberLong,
  user_id: NumberLong | null,
  plan: "basic",
  source: "trial|payment|referral_reward|admin_grant",
  is_group: Boolean,
  group_name: String,
  owner_user_id: NumberLong | null,
  expires_at: Date,
  active: Boolean,
  free_state: "none|trial|paid|bonus_week"
}
```

Indexes:

- Unique `{ subject_type: 1, subject_id: 1 }`.
- `{ expires_at: 1, active: 1 }`.

### `payments`

Single payment document for provider payment state and transaction fulfillment metadata.

```javascript
{
  _id: String,                      // domain PaymentId
  provider: "yookassa" | null,
  provider_payment_id: String | null,
  subscription_id: NumberLong | null,
  user_id: NumberLong | null,
  amount: NumberLong,
  currency: "RUB",
  months: NumberInt | null,
  status: "pending|waiting_for_capture|succeeded|canceled|failed|...",
  confirmation_url: String | null,
  idempotence_key: String | null,
  fulfilled: Boolean,
  fulfilled_at: Date | null,
  created_at: Date,
  updated_at: Date | null
}
```

`PaymentRepository` and `PaymentTransactionRepository` both map to this collection. Repository writes update only the fields owned by their domain type, so webhook status updates do not erase fulfillment metadata.

Indexes:

- `_id` unique by MongoDB.
- Unique sparse `{ provider_payment_id: 1 }`.
- `{ user_id: 1, status: 1 }`.

### `referrals`

Referral relationship and reward state.

```javascript
{
  referrer_user_id: NumberLong,
  invited_user_id: NumberLong,
  created_at: Date,
  rewarded_at: Date | null
}
```

Indexes:

- Unique `{ invited_user_id: 1 }`.
- `{ referrer_user_id: 1, invited_user_id: 1 }`.

### `external_channel_subscriptions`

External content subscriptions. `sub_num` is not persisted; the repository assigns display numbers when listing subscriptions for a subject.

```javascript
{
  subject_type: "user|chat",
  subject_id: NumberLong,
  platform: "twitch|youtube",
  channel_id: String,
  channel_name: String,
  url: String,
  created_at: Date,
  last_content_id: String | null,
  is_live: Boolean
}
```

Indexes:

- Unique `{ subject_type: 1, subject_id: 1, platform: 1, channel_id: 1 }`.
- `{ platform: 1, channel_id: 1 }`.

## Migration Policy

Phase 9.9 does not keep runtime compatibility with legacy collections. If existing production data must be preserved, add a one-shot importer that reads legacy collections from root `src/*` era and writes the reset schema before switching services to the new database.
