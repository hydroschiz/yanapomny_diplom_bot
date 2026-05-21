# Cutover Checklist

This checklist tracks the migration from the transitional root `src/*` runtime to the target `crates/*` + `bins/*` architecture.

## Final Rule

`src/*` is a legacy-only archive after cutover. Production services must be built from workspace packages in `bins/*` and reusable crates in `crates/*`.

Target commands:

```bash
cargo run -p bot
cargo run -p scheduler
cargo run -p webhook
```

Root `cargo run` is not a production command after Phase 9.8.

## Forbidden Dependencies

After cutover these checks must return no matches:

```bash
grep -R "yanapomnyu_bot::" bins crates
grep -R "crate::api\|crate::bot\|crate::scheduler\|crate::transport" bins crates
```

Layer-specific bans:

| Layer | Forbidden |
|-------|-----------|
| `domain` | async runtime, MongoDB, Redis, VK SDK, YooKassa SDK, HTTP clients, persistence DTOs |
| `application` | MongoDB, Redis, VK SDK, YooKassa SDK, reqwest, axum, concrete adapters |
| `presentation` | VK SDK, MongoDB, Redis, YooKassa SDK, direct database access |
| `transport-vk` | application use cases, MongoDB, Redis, YooKassa SDK, domain business decisions |
| `bins/*` | imports from root `src/*` |

## Migration Sources in `src/*`

These modules are sources for migration, not target locations for new production logic.

| Source | Target |
|--------|--------|
| `src/api/db.rs` | `crates/infrastructure/src/mongo/*` DTOs, mappers, repositories |
| `src/api/cache.rs` | `crates/infrastructure/src/redis/*` payment cache and locks |
| `src/api/payments.rs` | `crates/infrastructure/src/yookassa/*` plus application payment use cases |
| `src/api/llm_client.rs` | `crates/infrastructure/src/llm/*` implementation of `NaturalLanguageInterpreter` |
| `src/api/llm_models.rs` | split into infrastructure provider DTOs and domain/application command models |
| `src/api/time_calculator.rs` | domain/application scheduling rules, without provider DTO coupling |
| `src/bot/router.rs` | `transport-vk` event adapter plus `presentation` router boundary |
| `src/bot/handlers/*` | `crates/application` use cases and `crates/presentation` rendering/routing |
| `src/bot/keyboards/*` | `crates/presentation` keyboard builders, capability-aware |
| `src/bot/states/*` | `application` dialog state model and `DialogStateStore` port |
| `src/scheduler/mod.rs` | `DeliverDueRemindersUseCase` and scheduler service loop in `bins/scheduler` |
| `src/scheduler/subscription.rs` | subscription warning/purge use cases |
| `src/scheduler/channels.rs` | channel check use case plus Twitch infrastructure adapter |
| `src/transport/vk.rs` | `crates/transport-vk` send/callback implementation |
| `src/config.rs` | service config modules in `bins/*` or shared config crate/module |
| `src/app.rs` | split into service composition roots in `bins/bot`, `bins/scheduler`, `bins/webhook` |

## Production Scenarios to Preserve

Interactive VK bot scenarios:

- `/start` creates or loads user/subscription state and renders welcome.
- `/help` renders help.
- `/utc` opens paginated UTC keyboard within VK limits.
- `/setup` opens settings flow.
- `/profile` renders preferences, subscription state, reminder counters.
- `/list` lists reminders/tasks or empty state.
- `/pay` creates/reuses YooKassa payment when payments are enabled.
- `/subs` handles external channel subscription menu.
- `/ref` handles current VK-safe referral behavior.
- Natural-language text starts reminder/task creation through LLM interpreter.
- Confirmation stores task/reminder and resets dialog state.
- Edit/delete flows keep existing behavior.
- Snooze updates reminder trigger time and preserves VK keyboard constraints.
- Group message flow still respects mentions and peer/user separation.

Background scenarios:

- Claim due reminders atomically.
- Deliver reminders through `Notifier`/`BotTransport`.
- Retry temporary delivery failures with exponential backoff.
- Mark permanent failures without endless retry.
- Recalculate recurring reminders.
- Warn users about expiring subscriptions.
- Purge or process expired subscriptions according to existing policy.
- Check Twitch channel live status when credentials are configured.

Payment scenarios:

- Create or reuse pending payment.
- Cache pending payment metadata in Redis.
- Process YooKassa `payment.succeeded` webhook idempotently.
- Process cancellation/waiting events without duplicate user notifications.
- Extend subscription and store transaction.
- Grant referral reward once.

## Target Domain Model

The target logical model must align with the architecture document section 6:

| Entity | Required Role |
|--------|---------------|
| `User` | User identity/status independent from transport-specific IDs |
| `PlatformIdentity` | VK or future platform external identity mapping |
| `UserPreferences` | Timezone, language, snooze and notification policy |
| `Task` | User-owned planning item, separate from reminder trigger |
| `Reminder` | Scheduled trigger for a task |
| `DeliveryEvent` | Delivery attempt/result log |
| `Subscription` | Paid/trial access state |
| `Payment` | Provider payment transaction |
| `Referral` | Referral relationship and reward state |
| `ExternalChannelSubscription` | Twitch/YouTube subscription state |

Legacy names such as `remID`, `freestate`, `records`, `reminds`, and `telegramPaymentChargeID` belong only in infrastructure mappers while legacy compatibility is required.

## Phase Gates

Every cutover phase must pass:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Runtime-changing phases should also pass release build in a user-writable target directory:

```bash
CARGO_TARGET_DIR=/tmp/opencode/yanapomnyu_bot_target cargo build --release --workspace
```

## Cutover Completion Criteria

- Root `Cargo.toml` is a virtual workspace without `[package]`.
- `.` is not a workspace member.
- `src/*` is not compiled by `cargo check --workspace`.
- `bins/bot`, `bins/scheduler`, `bins/webhook` are real packages.
- Dockerfile and compose build/run `bot`, `scheduler`, `webhook` packages.
- `crates/application` owns all business use cases.
- `crates/infrastructure` owns all concrete Mongo/Redis/YooKassa/LLM/Twitch adapters.
- `crates/presentation` owns routing/rendering/keyboards.
- `crates/transport-vk` owns VK API interaction only.
- Removing `src/*` would not break workspace build.
