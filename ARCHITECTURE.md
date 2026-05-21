# YaPomnyu Bot - Архитектура

VK бот для создания и доставки напоминаний. После фаз 0-8 Telegram legacy удалён, основной runtime работает через VK long poll, а scheduler и YooKassa webhook можно запускать отдельно.

## Cutover Baseline

Текущее состояние после фаз 0-8 является переходным: root `src/*` всё ещё содержит рабочий VK runtime, handlers, production Mongo/YooKassa/LLM adapters и scheduler loops. Это больше не считается целевой архитектурой.

Целевой cutover зафиксирован в `refactoring_plan.md`, раздел 12:

```text
crates/*   reusable domain/application/infrastructure/presentation/transport layers
bins/*     production service composition roots
src/*      legacy-only archive, не импортируется и не компилируется workspace после cutover
```

До завершения Phase 9.8 `cargo run` всё ещё запускает transitional root runtime. После cutover production commands должны стать `cargo run -p bot`, `cargo run -p scheduler`, `cargo run -p webhook`.

Новые бизнес-сценарии, infrastructure adapters и transport adapters не должны развиваться в `src/*`; они должны добавляться в соответствующие `crates/*` и подключаться через `bins/*`.

## Runtime

```text
                         ┌──────────────────────────────┐
                         │          MongoDB             │
                         │ users/reminds/records/tx     │
                         └──────────────┬───────────────┘
                                        │
┌──────────────────────┐    ┌───────────▼───────────┐    ┌──────────────────────┐
│ VK long poll          │    │ yanapomnyu_bot runtime │    │ YooKassa webhook     │
│ vk-bot-api            │◄──►│ app/config/api/bot     │◄──►│ axum /payments       │
└──────────────────────┘    └───────────┬───────────┘    └──────────────────────┘
                                        │
                         ┌──────────────▼───────────────┐
                         │ schedulers                    │
                         │ reminders/subscriptions/chans │
                         └──────────────────────────────┘
```

The default `yanapomnyu_bot` binary is all-in-one: VK long poll, embedded schedulers, and embedded webhook server. For split deployments, run `scheduler` and `webhook` binaries separately and disable embedded components in the bot process.

## Entry Points

| Binary | Purpose |
|--------|---------|
| `yanapomnyu_bot` | Main VK bot process. Starts VK long poll and, by default, embedded schedulers/webhook. |
| `scheduler` | Standalone scheduler process. Runs reminder delivery, subscription expiry checks, and channel checks without VK long poll. |
| `webhook` | Standalone YooKassa webhook process. Runs Axum server without VK long poll or schedulers. |

Runtime switches:

| Env | Default | Effect |
|-----|---------|--------|
| `BOT_SCHEDULER_ENABLED` | `true` | Starts schedulers inside `yanapomnyu_bot`. Set `false` when `scheduler` runs separately. |
| `BOT_WEBHOOK_ENABLED` | `true` | Starts YooKassa webhook inside `yanapomnyu_bot`. Set `false` when `webhook` runs separately. |
| `PAYMENTS_ENABLED` | auto by YooKassa credentials | Enables payment creation and webhook processing. |

## Workspace Layers

```text
crates/domain          Pure value objects and domain rules
crates/application     Use cases and ports
crates/infrastructure  In-memory/test adapters and shared infrastructure primitives
crates/transport-core  Transport-neutral message, keyboard, capability traits
crates/transport-vk    VK keyboard conversion and VK capability rules
crates/presentation    Command/payload parsing, router intents, rendering, keyboard builders
bins/                  Target service composition roots: bot, scheduler, webhook
src/                   Transitional runtime now; legacy-only archive after cutover
```

Layering rules:

| Layer | May Depend On | Must Not Depend On |
|-------|---------------|--------------------|
| `domain` | std/chrono-like pure types | VK, MongoDB, Redis, YooKassa, HTTP clients |
| `application` | `domain`, ports | VK, MongoDB, Redis, YooKassa, concrete HTTP clients |
| `presentation` | `application` concepts, `transport-core` DTOs | VK API, MongoDB, Redis, YooKassa |
| `transport-core` | none of the runtime adapters | VK, Telegram, MongoDB |
| `transport-vk` | `transport-core` | business use cases, MongoDB |
| `bins/*` services | all crates and concrete adapters | business logic outside application use cases |
| `src/*` transitional runtime | legacy/root crate only until cutover | new production logic |

The root `src/` runtime still contains production adapters for the current deployment: MongoDB compatibility in `api/db.rs`, YooKassa in `api/payments.rs`, LLM HTTP in `api/llm_client.rs`, VK long poll routing in `bot/router.rs`, and scheduler loops in `scheduler/`. These modules are migration sources only; the target production path is `bins/*` using `crates/*`.

## Target Service Layout

| Service | Target Package | Responsibility |
|---------|----------------|----------------|
| VK bot | `bins/bot` | Configures VK transport, presentation router, application facade, infrastructure adapters; runs VK long poll. |
| Scheduler | `bins/scheduler` | Runs due reminder delivery, subscription expiry jobs, channel checks through application use cases. |
| Webhook | `bins/webhook` | Runs Axum YooKassa webhook endpoint and calls payment webhook use case. |

Target dependency direction:

```text
bins/* -> infrastructure -> application -> domain
bins/* -> presentation -> application -> domain
bins/* -> transport-vk -> transport-core
```

Forbidden after cutover:

```text
crates/* -> src/*
bins/* -> src/*
application -> MongoDB/Redis/VK/YooKassa/reqwest/axum
domain -> any I/O or persistence DTO
```

## Message Flow

1. `vk-bot-api` receives a long poll event.
2. `src/bot/router.rs` normalizes it into command, text, or callback handling.
3. `presentation` parsers classify commands and callback payloads.
4. Handlers use MongoDB/LLM/YooKassa adapters from `src/api/` and update `DialogueStore`.
5. Replies are sent through `BotTransport` implemented by `src/transport/vk.rs`.
6. Keyboard constraints are centralized through transport capabilities and VK sanitization rules.

## Scheduler Flow

Reminder delivery uses the same `BotTransport` abstraction as the interactive bot.

```text
claim_due_reminders(batch)
        │
        ▼
status active/retry -> processing, atomically in MongoDB
        │
        ▼
parallel send through BotTransport, max concurrency 20
        │
        ├── success: mark sent or schedule next recurrence
        ├── temporary error: exponential retry
        └── permanent error: mark failed
```

Standalone `scheduler` reuses the same `scheduler::start_scheduler`, `start_subscription_scheduler`, and `start_channel_scheduler` functions as the all-in-one process.

## Payment Flow

1. User opens `/pay` and selects a tariff.
2. `PaymentService::init_or_get_last` creates or reuses a pending YooKassa payment.
3. Pending payment metadata is cached in Redis.
4. YooKassa calls `POST /yookassa/webhook`.
5. `PaymentService::handle_webhook` deduplicates through Redis, extends subscription in MongoDB, stores transaction status, and notifies the user through `BotTransport`.

The webhook route is available in both modes:

| Mode | Route Owner |
|------|-------------|
| all-in-one | `yanapomnyu_bot` when `BOT_WEBHOOK_ENABLED=true` |
| standalone | `webhook` binary |

## Configuration

Required for the main bot:

| Env | Description |
|-----|-------------|
| `VK_ACCESS_TOKEN` | VK community access token |
| `VK_GROUP_ID` | VK community group ID |
| `MONGO_URI` or `MONGO_USER`/`MONGO_PASS`/`MONGO_HOST`/`MONGO_PORT`/`MONGO_DB` | MongoDB connection |
| `REDIS_URL` | Redis URL for payment cache |
| `LLM_API_URL` | LLM API URL |

Required for payments:

| Env | Description |
|-----|-------------|
| `PAYMENTS_ENABLED=true` | Enables payment flow and webhook processing |
| `YK_SHOP_ID` | YooKassa shop ID |
| `YK_SECRET_KEY` | YooKassa secret key |
| `YK_RETURN_URL` | Payment return URL, defaults to VK page |

Optional:

| Env | Description |
|-----|-------------|
| `BOT_USERNAME` | Bot short name for group mentions |
| `ADMINS` | Comma-separated admin VK IDs |
| `TWITCH_CLIENT_ID`, `TWITCH_ACCESS_TOKEN` | Enables Twitch channel scheduler |
| `IP`, `PORT` | Webhook server bind address |
| `RUST_LOG` | Tracing filter |

## Deployment Modes

Current transitional all-in-one local run:

```bash
cargo run
```

Current transitional split run:

```bash
BOT_SCHEDULER_ENABLED=false BOT_WEBHOOK_ENABLED=false cargo run --bin yanapomnyu_bot
cargo run --bin scheduler
PAYMENTS_ENABLED=true cargo run --bin webhook
```

Target post-cutover split run:

```bash
cargo run -p bot
cargo run -p scheduler
PAYMENTS_ENABLED=true cargo run -p webhook
```

Docker Compose split run uses profile `standalone` and the same env switches.

```bash
BOT_SCHEDULER_ENABLED=false BOT_WEBHOOK_ENABLED=false docker compose --profile standalone up -d
```

## Quality Gates

Run before committing runtime changes:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

`cargo deny check` is optional until `cargo-deny` is installed and configured for the workspace.

## VK Smoke Test

This is a manual production/staging check because it needs live VK, MongoDB, Redis, LLM, and optional YooKassa credentials.

| Scenario | Expected Result |
|----------|-----------------|
| `/start` | Bot creates or loads user profile and sends welcome/help keyboard. |
| `/help` | Bot sends help text without errors. |
| `/utc` | UTC keyboard is paginated and all buttons fit VK limits. |
| `/setup` | Settings flow opens and dialogue state changes correctly. |
| `/profile` | Profile displays timezone, subscription, and reminder counters. |
| `/list` | Reminder list is shown or empty state is returned. |
| `/pay` | Tariff selection opens; payment link works when payments are enabled. |
| `/subs` | Channel subscription menu opens. |
| `/ref` | Referral command returns current VK-safe placeholder/flow. |
| Create reminder | Natural-language text creates a pending reminder via LLM and confirmation stores it. |
| Snooze | Snooze buttons update reminder time and keep keyboard within VK limits. |
| Due delivery | Scheduler claims due reminder once and sends it through VK transport. |
