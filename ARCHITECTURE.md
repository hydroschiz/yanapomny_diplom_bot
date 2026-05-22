# YaPomnyu Bot - Архитектура

VK бот для создания и доставки напоминаний. После Phase 9.8 production runtime живёт в `bins/*`, reusable слои живут в `crates/*`, а root `src/*` оставлен на диске как legacy-only archive.

## Cutover Baseline

Root `Cargo.toml` является virtual workspace без `[package]`. `cargo check --workspace` больше не компилирует root `src/*`.

Целевой cutover зафиксирован в `refactoring_plan.md`, раздел 12:

```text
crates/*   reusable domain/application/infrastructure/presentation/transport layers
bins/*     production service composition roots
src/*      legacy-only archive, не импортируется и не компилируется workspace после cutover
```

Production commands:

```bash
cargo run -p bot
cargo run -p scheduler
cargo run -p webhook
```

Новые бизнес-сценарии, infrastructure adapters и transport adapters не должны развиваться в `src/*`; они должны добавляться в соответствующие `crates/*` и подключаться через `bins/*`.

## Runtime

```text
                         ┌──────────────────────────────┐
                         │          MongoDB             │
                         │ users/reminds/records/tx     │
                         └──────────────┬───────────────┘
                                        │
┌──────────────────────┐    ┌───────────▼───────────┐    ┌──────────────────────┐
│ VK long poll          │    │ bins/bot              │    │ bins/webhook         │
│ transport-vk          │◄──►│ presentation/use cases │    │ axum /yookassa       │
└──────────────────────┘    └───────────────────────┘    └──────────────────────┘
                                        ▲
                                        │
                         ┌──────────────┴───────────────┐
                         │ bins/scheduler                │
                         │ reminder delivery loop        │
                         └──────────────────────────────┘
```

Services are split. The bot process does not embed scheduler or webhook runtime after cutover.

## Entry Points

| Binary | Purpose |
|--------|---------|
| `bot-service` (`cargo run -p bot`) | VK long poll service. Wires VK transport, presentation, application use cases, Mongo/Redis/LLM/YooKassa adapters. |
| `scheduler-service` (`cargo run -p scheduler`) | Reminder scheduler service. Runs due reminder delivery through `DeliverDueRemindersUseCase`. |
| `webhook-service` (`cargo run -p webhook`) | YooKassa webhook service. Runs Axum server and calls `ProcessYooKassaWebhookUseCase`. |

## Workspace Layers

```text
crates/domain          Pure value objects and domain rules
crates/application     Use cases and ports
crates/infrastructure  Mongo/Redis/YooKassa/LLM/Twitch adapters
crates/transport-core  Transport-neutral message, keyboard, capability traits
crates/transport-vk    VK keyboard conversion and VK capability rules
crates/presentation    Command/payload parsing, router intents, rendering, keyboard builders
bins/                  Target service composition roots: bot, scheduler, webhook
src/                   Legacy-only archive, not compiled by workspace
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
| `src/*` legacy archive | not part of workspace build | new production logic |

The root `src/` tree is retained only as migration reference and is not a production build path.

## Target Service Layout

| Service | Target Package | Responsibility |
|---------|----------------|----------------|
| VK bot | `bins/bot` | Configures VK transport, presentation router, application facade, infrastructure adapters; runs VK long poll. |
| Scheduler | `bins/scheduler` | Runs due reminder delivery through application use cases. Subscription maintenance and all-channel Twitch polling require follow-up application maintenance ports. |
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
2. `transport-vk` normalizes raw VK events.
3. `presentation` classifies commands, text and callback payloads into routes.
4. `bins/bot` invokes application use cases through infrastructure adapters.
5. Replies are rendered by `presentation` and sent through `transport-vk`.
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

`bins/scheduler` runs this flow through `DeliverDueRemindersUseCase`. Subscription warning/purge and all-subscription Twitch polling are not wired until their application maintenance ports are added.

## Payment Flow

1. Payment creation is owned by application payment use cases plus `HttpYooKassaPaymentGateway`.
2. Pending payment metadata is cached through the Redis payment cache adapter.
3. YooKassa calls `POST /yookassa` on `bins/webhook`.
4. The route calls `ProcessYooKassaWebhookUseCase` and updates payment status in MongoDB.

The webhook route is available in both modes:

| Mode | Route Owner |
|------|-------------|
| standalone | `webhook-service` from `bins/webhook` |

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

Post-cutover local split run:

```bash
cargo run -p bot
cargo run -p scheduler
PAYMENTS_ENABLED=true cargo run -p webhook
```

Docker Compose split run uses profile `standalone`.

```bash
docker compose --profile standalone up -d
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
