# Архитектурный рефакторинг yanapomnyu_bot

## 1. Цели

- Реализовать чистую архитектуру и DDD в существующем Rust-проекте.
- Полностью отделить бизнес-логику от транспорта (VK / Telegram / тесты).
- Перевести проект на монорепо Cargo workspace c набором "микросервисов" (бинарников), которые повторно используют общие крейты.
- Сохранить существующее поведение и схему MongoDB на каждом этапе.
- В default build остаемся на VK; Telegram остаётся легаси и затем убирается.

## 2. Принципы

- Зависимости направлены строго внутрь: Presentation → Application → Domain. Infrastructure реализует порты, объявленные в Application, и не известна внутренним слоям.
- Domain не знает про MongoDB, VK, Telegram, HTTP, JSON LLM, axum.
- Application не знает про конкретные технологии. Только через порты (трейты).
- Композиционный корень (бинарник) — единственное место, где известны конкретные реализации.
- Каждый use case — отдельный объект/функция с явными входами и выходами.
- Состояние диалога FSM в памяти как реализация порта `DialogStateStore`.
- Ошибки преобразуются на границе слоёв; внутренний слой не возвращает ошибки внешнего.

## 3. Анализ текущего состояния

### 3.1 Что уже сделано хорошо

- Транспортная абстракция: `transport::traits::BotTransport`, `TransportKeyboard`, `TransportButton`.
- VK-адаптер вынесен в `transport::vk::VkTransport`.
- FSM диалога вынесен из teloxide в `transport::dialogue_store::DialogueStore`.
- `app.rs` уже близок к композиционному корню: только конфиг, БД, payment service, transport, scheduler, VK bot.
- `Cargo.toml` уже содержит `feature = "telegram-legacy"`, default build чистый VK.
- Юнит-тесты для transport-слоя, llm_models, time_calculator, timezone.

### 3.2 Проблемы (по слоям)

#### Перепутаны слои

- `src/api/db.rs` (1282 строки): MongoDB-репозиторий + доменные сущности (`User`, `Reminder`, `UserRecord`, `ChannelSubscription`, `Referral`, `Tariff`-related поля) + бизнес-инварианты (`is_active`, `extend_subscription`, `claim_due_reminders`).
- `src/api/llm_models.rs` (807 строк): DTO LLM-провайдера + доменные value objects (`TimeOfDay`, `Weekday`, `RecurrencePattern`, `RecurrenceFilter`, `IntervalUnit`, `DayPosition`).
- `src/api/time_calculator.rs` (912 строк): чистая бизнес-логика, лежит в инфраструктурном модуле `api`.
- `src/api/payments.rs` (518 строк): YooKassa SDK + Redis cache + axum router + бизнес-логика fulfillment + рассылка пользовательских сообщений.
- `src/scheduler/*`: фоновые задачи + формирование пользовательских сообщений + знание о клавиатурах + прямой вызов `Db` и transport.
- `src/bot/handlers/*` (особенно `reminder.rs` 1295 строк, `text.rs` 723, `commands.rs` 684): прямые вызовы `Db`, LLM, форматирование HTML, клавиатуры, FSM, валидация, отправка через transport.
- `src/bot/handlers/text.rs` хранит `CITY_MAP` (доменное правило), `OFFSET_RE`, парсинг + обработчики ввода + Telegram-legacy функции.
- Кросс-слойная связь: `scheduler::send_reminder` использует `bot::keyboards::reminder_snooze_keyboard` напрямую.

#### Отсутствующие слои

- Нет `domain` крейта/модуля: сущности, value objects, агрегаты, доменные сервисы.
- Нет `application` крейта: use cases, порты, команды/запросы.
- Нет отдельного presentation-слоя: рендеринг сообщений и роутинг событий смешаны с use case-логикой.

#### Двойной источник истины состояния

- `User.state` в Mongo (`waiting_for_message` / `waiting_for_time`) и `DialogueStore` в памяти описывают одно и то же; оба пишутся, ни один не является single source of truth.

#### Связанность с конкретными платформами

- `Bot::from_env` уже не используется, но `transport::adapters` всё ещё держит legacy `TelegramTransport` и `impl BotTransport for teloxide::Bot` под `telegram-legacy`.
- `bot/handlers/*` содержат legacy Telegram entrypoints за `#[cfg(feature = "telegram-legacy")]`.
- `bot/router.rs` смешивает: VK long-poll handler, парсинг команд, диспатчинг callback, рендер сообщений, бизнес-проверки (timezone, subscription).

#### Связанность с MongoDB BSON

- Доменные структуры (`User`, `Reminder`, `UserRecord`) деривят `Serialize/Deserialize` с `#[serde(rename = "...")]` под BSON. Это блокирует свободное изменение доменной модели.

#### Тестируемость

- Use case-уровневых тестов нет: чтобы проверить создание напоминания, нужен Mongo, Redis и LLM API.
- В тестах нет in-memory репозиториев и моков для портов.

### 3.3 Текущая структура (упрощённо)

```
src/
├── lib.rs / main.rs / app.rs / config.rs
├── api/
│   ├── db.rs            # репозиторий + сущности + бизнес-правила
│   ├── llm_client.rs    # HTTP client
│   ├── llm_models.rs    # DTO + value objects
│   ├── time_calculator.rs  # доменная логика
│   ├── cache.rs         # Redis cache
│   └── payments.rs      # YooKassa + axum router + fulfillment
├── bot/
│   ├── router.rs                  # VK AppHandler + диспатчинг
│   ├── states/                    # AppState
│   ├── handlers/{reminder,text,commands,pay,channels,profile,referral,callbacks}.rs
│   └── keyboards/*.rs             # клавиатуры
├── scheduler/{mod,subscription,channels}.rs
├── transport/
│   ├── traits.rs / dialogue_store.rs / text_format.rs / vk.rs
│   └── adapters/mod.rs            # Telegram-legacy
└── utils/timezone.rs
```

## 4. Целевая архитектура

### 4.1 Логические уровни

- Domain — сущности, value objects, агрегаты, доменные сервисы, инварианты, доменные ошибки. Без I/O.
- Application — use cases, командные/запросные DTO, порты (трейты репозиториев и внешних сервисов), application-сервисы (FSM координация). Без I/O напрямую.
- Infrastructure — реализации портов: MongoDB, Redis, YooKassa, HTTP LLM, Twitch, системные часы, in-memory FSM store.
- Presentation / Transport — приём событий платформы (VK long poll), нормализация в команды, выбор use case, рендер ответов через `BotTransport`. Знает про конкретные ограничения VK (число кнопок и т.п.) только тут.
- Композиционный корень — бинарник, который связывает реализации и запускает компоненты.

### 4.2 Пять основных подсистем (соответствие тексту требования)

1. Подсистема интеграции с коммуникационными платформами → `transport-core` + `transport-vk` + presentation routing.
2. Подсистема диалоговой интерпретации → порт `NaturalLanguageReminderParser` + реализация `LlmHttpReminderParser` (infrastructure).
3. Подсистема прикладных сценариев → `application` крейт (use cases).
4. Подсистема планирования и доставки напоминаний → use cases в `application` + `infrastructure` для часов/репозиториев + бинарник, который запускает их периодически.
5. Подсистема хранения данных → порты-репозитории в `application` + Mongo-реализации в `infrastructure`.

### 4.3 Монорепо (Cargo workspace) и "микросервисы"

```
yanapomnyu_bot/
├── Cargo.toml                      # [workspace]
├── crates/
│   ├── shared/                     # общие утилиты: типы, ошибки, логирование, время
│   ├── domain/                     # чистая доменная модель
│   ├── application/                # use cases, порты, DTO
│   ├── infrastructure/             # MongoDB / Redis / YooKassa / LLM HTTP / Twitch / System clock
│   ├── transport-core/             # BotTransport, Keyboard, Button, MessageContent
│   ├── transport-vk/               # VK long-poll адаптер
│   ├── transport-telegram/         # (опционально) Telegram-legacy за feature
│   └── presentation/               # рендер сообщений, FSM router, парсер команд
├── bins/
│   ├── bot/                        # композиционный корень: VK + webhook + schedulers
│   ├── scheduler/                  # (опционально) standalone worker для напоминаний/подписок/каналов
│   └── webhook/                    # (опционально) standalone YooKassa webhook receiver
├── services/
│   └── llm_api/                    # внешний Go-сервис, остаётся как есть
├── ops/
│   ├── docker-compose.yml
│   └── docker-compose.prod.yml
└── tests/                          # workspace integration tests
```

Бинарник `bot` остаётся монолитным по умолчанию (как сейчас). Бинарники `scheduler` и `webhook` опциональны: они переиспользуют те же крейты и могут быть развёрнуты отдельно. Это и есть форма "микросервисов" в монорепо без дробления бизнес-логики.

### 4.4 Граф зависимостей крейтов

```
shared              ← всё остальное может его использовать
domain              → shared
application         → domain, shared
infrastructure      → application, domain, shared
transport-core      → shared
transport-vk        → transport-core, shared
transport-telegram  → transport-core, shared
presentation        → application, transport-core, shared
bins/bot            → presentation, transport-vk, infrastructure, application, domain, shared
bins/scheduler      → application, infrastructure, shared
bins/webhook        → application, infrastructure, shared
```

Проверить дисциплину можно `cargo deny` или `cargo modules` + правила в CI.

## 5. Контракты слоёв

### 5.1 Domain (`crates/domain`)

Зависимости: только `chrono`, `thiserror`, `uuid`. Без `serde`, `mongodb`, `tokio` в публичном API.

Сущности и агрегаты:

- `User` (aggregate root): `UserId`, предпочтения времени (`MorningTime`, `AfternoonTime`, `EveningTime`), таймзона, кнопки snooze, авто-snooze.
- `Reminder` (aggregate root): `ReminderId`, `ChatId`, текст, расписание, статус, retry-state, snooze-state. Методы: `claim()`, `mark_sent()`, `schedule_retry(now, policy)`, `mark_failed()`, `snooze(now, minutes)`, `recompute_next(now)`.
- `Subscription` (aggregate root): `ChatId`, `is_group`, `owner`, `expiry`, `free_state`. Методы: `is_active(now)`, `extend(months, now)`, `mark_warned()`, `reset_flags()`.
- `ChannelSubscription` (aggregate root): `UserId`, `Platform`, `ChannelId`, `ChannelName`, `Url`, `SubNumber`, `LastContentId`, `IsLive`.
- `Referral`: `referrer`, `invited`, `created_at`, `rewarded_at`.
- `PaymentTransaction`: `payment_id`, `user`, `amount`, `currency`, `months`, `status`, `fulfilled`.

Value Objects:

- `UserId(i64)`, `ChatId(i64)`, `ReminderId(i32)`, `PaymentId(String)`, `Months(u8)`, `Money { amount: i64, currency: Currency }`.
- `UtcOffset` с парсером строки и форматированием.
- `TimeZone` (IANA или фиксированный offset).
- `RecurrenceRule`, `Schedule`, `TimeOfDay`, `Weekday`, `DayPosition`. Метод `Schedule::next_at(now: DateTime<Utc>, prefs: &TimePreferences) -> Result<DateTime<Utc>, DomainError>` инкапсулирует то, что сейчас в `time_calculator.rs`.
- `ReminderStatus`: `Active`, `Processing`, `Retry { attempt, retry_at }`, `Sent`, `Failed`.
- `SubscriptionStatus`: `Trial { until }`, `Active { until }`, `Expired`.

Доменные сервисы:

- `RetryPolicy` — exponential backoff и max retries.
- `SubscriptionPolicy` — расчёт триала и продления.
- `ReferralPolicy` — правила вознаграждения.

Ошибки: `DomainError` через `thiserror`, без `anyhow`.

### 5.2 Application (`crates/application`)

Зависимости: `domain`, `shared`, `async_trait`, `thiserror`. Без транспорта и БД.

Порты (трейты):

```text
UserRepository
ReminderRepository
SubscriptionRepository
ChannelSubscriptionRepository
ReferralRepository
PaymentTransactionRepository
PaymentCachePort
PaymentGatewayPort
NaturalLanguageReminderParser
StreamPlatformGateway
DialogStateStore
Notifier
Clock
IdGenerator
```

Все методы async (через `async_trait`), возвращают `Result<_, ApplicationError>`. Внутри ошибок не торчат типы инфраструктуры.

Use cases (по одному файлу/типу на use case):

Reminder:
- `CreateReminderRequest` (после подтверждения LLM-парсинга).
- `RequestReminderConfirmationUseCase` (текст → `Notifier::ask_confirmation`).
- `ConfirmTextAndParseUseCase` (вызов LLM-парсера → результат для подтверждения).
- `ConfirmAndCreateReminderUseCase` (записывает напоминание в репозиторий).
- `EditPendingReminderUseCase`.
- `CancelPendingReminderUseCase`.
- `ListUserRemindersUseCase`.
- `DeleteUserReminderUseCase`.
- `SnoozeReminderUseCase`.
- `CompleteReminderUseCase`.

User / Profile:
- `EnsureUserUseCase`.
- `SetUserTimezoneUseCase`.
- `SetSnoozeButtonsUseCase`.
- `SetAutoSnoozeUseCase`.
- `GetProfileUseCase`.

Subscription / Payments:
- `EnsureSubscriptionUseCase` (создаёт триал при первом обращении).
- `InitYooKassaPaymentUseCase`.
- `CheckPaymentStatusUseCase`.
- `ProcessYooKassaWebhookUseCase`.
- `WarnExpiringSubscriptionsUseCase`.
- `PurgeExpiredSubscriptionsUseCase`.

Channels:
- `SubscribeChannelUseCase`.
- `UnsubscribeChannelUseCase`.
- `ListChannelSubscriptionsUseCase`.
- `CheckTwitchStreamsUseCase`.

Scheduler:
- `DeliverDueRemindersUseCase`.
- `RecoverStuckRemindersUseCase`.

Application-сервисы:
- `DialogCoordinator` — обёртка над `DialogStateStore` для конкретных state-переходов.
- `NotificationCenter` — формирует структурированные `Notification` объекты, не тексты сообщений; в presentation эти `Notification` рендерятся в текст и клавиатуры.

DTO:
- `Notification` enum: `RemindMe`, `ReminderCreated`, `ReminderListView`, `SubscriptionExpiring`, `SubscriptionExpired`, `PaymentLink`, `PaymentSucceeded`, `ReferralReward`, `ChannelSubscribed`, `StreamLive`, etc. Каждый вариант содержит данные, не строки.
- Это позволяет presentation-слою выбирать форматирование, локализацию, клавиатуры.

Тестирование:
- Каждый use case покрывается юнит-тестом с in-memory реализациями портов.
- Для FSM — отдельные тесты переходов.

### 5.3 Infrastructure (`crates/infrastructure`)

Зависимости: `application`, `domain`, `shared`, `mongodb`, `bson`, `redis`, `reqwest`, `yookassa`, `tokio`, `chrono`, `serde`.

Содержимое:

- `mongo/{user,reminder,subscription,channel_subscription,referral,transaction}_repository.rs` с маппингом `domain ↔ BSON`.
- `mongo/migrations.rs` или `mongo/bootstrap.rs` — индексы и счётчик `remID`, вынесенные из `Db::connect`.
- `redis/payment_cache.rs`.
- `yookassa/payment_gateway.rs`.
- `llm/http_reminder_parser.rs` — превращает JSON LLM в доменные `RecurrenceRule` и `Schedule`. Сейчас этот маппинг разбросан между `llm_models.rs` и `time_calculator.rs`.
- `twitch/twitch_gateway.rs`.
- `clock/system_clock.rs`.
- `id/uuid_id_generator.rs`.
- `dialog/dashmap_dialog_store.rs` — реализация `DialogStateStore` (текущий `DialogueStore`).
- `errors.rs` — преобразование инфраструктурных ошибок в `ApplicationError`.

Опционально:
- `outbox/mongo_outbox_repository.rs` для доставки уведомлений в "exactly-once" стиле в будущем.

### 5.4 Transport-core (`crates/transport-core`)

Зависимости: `async_trait`, `serde_json` (для payload), `thiserror`.

API:

- `pub trait BotTransport: Send + Sync + Clone + 'static` — текущий, но дополнен:
  - `fn capabilities(&self) -> TransportCapabilities` (max_buttons, max_rows, max_buttons_per_row, supports_html).
- `pub struct Keyboard { rows: Vec<Vec<Button>> }` (имя короче, перенести из `TransportKeyboard`).
- `pub enum Button { Callback {...}, Url {...} }`.
- `pub struct MessageContent { text: String, keyboard: Option<Keyboard> }` — единая структура отправляемого сообщения.
- `BotTransport::send(peer_id, MessageContent)` (одно семейство методов вместо двух).
- `pub struct CallbackContext { event_id, user_id, peer_id, payload }`.

Это позволяет presentation-слою адаптироваться к ограничениям VK без захардкоженных констант в коде клавиатур (как сейчас исправляли `911 too much buttons`).

### 5.5 Transport-vk (`crates/transport-vk`)

- `VkTransport` реализует `BotTransport` + `capabilities()` с лимитами VK (10 кнопок).
- `VkLongPollAdapter`:
  - получает `Event` из `vk-bot-api`,
  - конвертирует в `IncomingEvent` (доменно-нейтральный):
    - `IncomingMessage { peer_id, user_id, text, is_group }`,
    - `IncomingCallback { event_id, peer_id, user_id, payload: serde_json::Value }`.
  - передаёт в `Router` из presentation.
- VK-конкретные ограничения и `color: None` для URL-кнопок остаются здесь.

### 5.6 Presentation (`crates/presentation`)

Зависимости: `application`, `transport-core`, `shared`.

Содержимое:

- `Router`:
  - принимает `IncomingEvent` и `&AppContext` (контейнер с use case-фасадами и `BotTransport`).
  - Парсит команды (`/start`, `/help`, `/utc`, `/list`, `/pay`, `/setup`, `/profile`, `/subs`, `/ref`, `/yan`, `/remind`).
  - Выбирает следующий шаг по состоянию `DialogStateStore`.
  - Вызывает соответствующий use case.
  - Отдаёт результат `Renderer`.
- `Renderer`:
  - функция `render(notification: Notification, capabilities: TransportCapabilities) -> MessageContent`.
  - Внутри: текстовые шаблоны (русские), клавиатуры с пагинацией, эскейпинг.
  - Содержит `KeyboardBuilder` для каждой страницы (UTC, профиль, оплата и т.д.) и автоматически бьёт длинные клавиатуры на страницы.
- `CommandParser`: VK-нейтральный парсер `/cmd args`.
- `PayloadParser`: парсинг callback payload.

### 5.7 Композиционный корень (`bins/bot`)

- Зависит от: `application`, `infrastructure`, `transport-vk`, `presentation`, `shared`.
- Знает обо всех конкретных реализациях.
- Делает примерно следующее:

```text
fn main():
    Config::from_env()
    Db = MongoClient(...)  // только тут
    repos = MongoRepositories::new(Db)
    cache = RedisPaymentCache::new(...)
    parser = LlmHttpReminderParser::new(...)
    gateway = YooKassaPaymentGateway::new(...)
    twitch = TwitchHttpGateway::new(...)
    clock = SystemClock::new()
    state_store = DashMapDialogStateStore::new()
    transport = VkTransport::new(...)
    notifier = TransportNotifier::new(transport.clone(), Renderer::default())
    use_cases = AppFacade::new(repos, cache, parser, gateway, twitch, clock, state_store, notifier)
    router = Router::new(use_cases.clone())
    vk_adapter = VkLongPollAdapter::new(router)
    spawn_axum_yookassa_webhook(use_cases.clone())
    spawn_reminder_scheduler(use_cases.clone())
    spawn_subscription_scheduler(use_cases.clone())
    spawn_channel_scheduler(use_cases.clone())
    vk_adapter.run().await
```

`AppFacade` — простой struct, агрегирующий ссылки на все use case-фасады; нужен, чтобы handler-ы получали один объект, а не 20 параметров.

## 6. Дорожная карта (фазы)

Каждая фаза должна заканчиваться зелёными `cargo check --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings` и поведенчески идентичным ботом в VK. Никаких "большой взрыв" миграций.

### Фаза 0. Подготовка workspace

1. Превратить корень репозитория в workspace `[workspace]` в `Cargo.toml`.
2. Создать пустые крейты:
   - `crates/shared`,
   - `crates/domain`,
   - `crates/application`,
   - `crates/infrastructure`,
   - `crates/transport-core`,
   - `crates/transport-vk`,
   - `crates/presentation`,
   - `bins/bot`.
3. Текущий код временно остаётся как `crates/legacy/yanapomnyu_bot` (или просто `crates/bot_legacy`) и публикует тот же бинарник, чтобы ничего не сломалось.
4. `bins/bot` пока просто реэкспортирует main legacy, чтобы deploy-флоу не сломался.
5. Обновить Dockerfile / docker-compose под workspace.
6. Проверка: `cargo build`, `cargo run`, `cargo test --workspace`.

### Фаза 1. Извлечение Domain

1. Перенести в `domain` доменные структуры из `legacy`:
   - `User`, `Reminder`, `UserRecord` → переименованные сущности `domain::user::User`, `domain::reminder::Reminder`, `domain::subscription::Subscription`.
   - `Platform`, `ChannelSubscription`, `Referral`, `Tariff`, `PaymentTransaction`.
2. Удалить из них `serde` derive связанный с BSON (rename, with). Эти атрибуты переезжают в Mongo-DTO в `infrastructure`.
3. Вынести из `time_calculator.rs` чистую логику в `domain::scheduling`:
   - `Schedule`, `RecurrenceRule`, `TimePreferences::next_at`, helpers (`add_months`, `days_in_month`, `is_weekday`, `is_weekend`).
4. Вынести в `domain` value objects из `llm_models.rs`:
   - `Weekday`, `TimeOfDay`, `DayPosition`, `RecurrencePattern`, `RecurrenceFilter`, `IntervalUnit`, `OffsetDirection`, `TimeSpecType`.
5. Реализовать state-машину `Reminder`: методы `claim()`, `mark_sent()`, `schedule_retry(policy, now)`, `mark_failed()`, `snooze(now, minutes)`, `next_after_send(now)`.
6. Реализовать `Subscription::is_active(now)`, `extend(months, now)`.
7. Юнит-тесты domain в `crates/domain/tests/...` (повторно использовать существующие тесты `time_calculator` и `llm_models` в части value objects).

Промежуточное состояние: `legacy` крейт продолжает компилироваться и зависит от `domain`.

### Фаза 2. Извлечение портов и use case-ов

1. В `application` определить порты-репозитории и внешние сервисы (см. 5.2).
2. В `application` определить `Notification` enum и `Notifier` порт.
3. Определить `DialogStateStore` порт.
4. Перенести use case-логику из текущих handler-ов в `application` пошагово:
   - Начать с самого простого: `EnsureUserUseCase`, `SetUserTimezoneUseCase`, `GetProfileUseCase`.
   - Потом reminder use cases (CRUD, snooze, list, deletion).
   - Потом subscription/payments use cases.
   - Последним — channels use cases.
5. Каждый use case покрыть юнит-тестами с in-memory реализациями портов.
6. Старый `bot/handlers/*` пока сохраняется и переключается на вызов use case-ов; постепенно превращается в тонкий адаптер.

### Фаза 3. Infrastructure-реализации

1. Перенести MongoDB-клиент в `infrastructure::mongo`.
2. Каждое поведение `Db` разложить на репозитории:
   - `MongoUserRepository` (методы из `users()` и `update_user_state` и т.д.),
   - `MongoReminderRepository` (включая `claim_due_reminders`, `update_reminder_time`, `mark_*`, `recover_stuck_reminders`),
   - `MongoSubscriptionRepository`,
   - `MongoChannelSubscriptionRepository`,
   - `MongoReferralRepository`,
   - `MongoPaymentTransactionRepository`.
3. Mapper-ы `domain ↔ BSON` с тестами на сериализацию.
4. `MongoBootstrap::run` — вынести `ensure_indexes` и `ensure_reminder_counter`.
5. `RedisPaymentCache` (из `api/cache.rs`).
6. `YooKassaPaymentGateway` (из `api/payments.rs`, без axum router и без рассылки сообщений; они переедут в use case + presentation).
7. `LlmHttpReminderParser`:
   - Перенести `LlmClient` и `ParseReminderRequest` сюда.
   - Внутри маппер `LLM JSON → domain::Schedule + ParsedReminder`. То, что сейчас в `time_calculator::calculate_reminder_time` и `llm_models::to_legacy_delay`, переезжает в `domain::scheduling` + adapter.
8. `TwitchHttpGateway` (из `scheduler/channels.rs`).
9. `SystemClock`, `DashMapDialogStateStore`.
10. Unit-тесты infrastructure (минимум: маппинги). Опционально — testcontainers для Mongo/Redis.

### Фаза 4. transport-core + transport-vk

1. Перенести `BotTransport`, `Keyboard`, `Button`, `strip_html` в `transport-core`.
2. Расширить трейт: `capabilities()`, `send(MessageContent)`.
3. `transport-vk` реализует трейт и предоставляет `VkLongPollAdapter`.
4. Адаптер не знает про use cases; принимает callback-роутер из presentation.
5. Презентационные ограничения VK (10 кнопок, color на URL и т.п.) запечатать как `VkCapabilities` и/или `KeyboardSanitizer`.

### Фаза 5. Presentation

1. `Router` принимает `IncomingEvent`, выбирает use case через `AppFacade`, получает `Notification`, рендерит, отправляет через `BotTransport`.
2. `Renderer` владеет всеми текстами и клавиатурами:
   - Сейчас они разбросаны по `bot/handlers/*` и `bot/keyboards/*`. Здесь они объединяются.
   - Пагинация UTC/любых длинных меню реализована централизованно с учётом `capabilities`.
3. `CommandParser`, `PayloadParser`.
4. Юнит-тесты:
   - таблица "command → use case" (matrix test),
   - snapshot-тесты на сообщения.

### Фаза 6. Композиционный корень и удаление legacy

1. `bins/bot/src/main.rs` собирает финальный пайплайн (см. 5.7).
2. Старый `crates/legacy` удаляется. Все legacy Telegram entry points удаляются вместе с ним.
3. `feature = "telegram-legacy"` упраздняется. Если потребуется Telegram, добавится `transport-telegram` крейт по аналогии с `transport-vk`.
4. Очистка docker-compose / README / .env.example: только VK + общие сервисы.

### Фаза 7. Опциональные standalone-сервисы

1. `bins/scheduler` запускает `DeliverDueRemindersUseCase`, `WarnExpiringSubscriptionsUseCase`, `PurgeExpiredSubscriptionsUseCase`, `CheckTwitchStreamsUseCase` без VK transport (notifier — в режиме "fanout через `BotTransport` из конфига" или через обновлённый pull-only режим).
2. `bins/webhook` запускает axum-сервер с YooKassa webhook через `ProcessYooKassaWebhookUseCase`.
3. В docker-compose можно включать/выключать сервисы как сервисы.

### Фаза 8. Финальная сверка и стабилизация

1. Полный smoke-test в VK: `/start`, `/help`, `/utc`, `/setup`, `/profile`, `/list`, `/pay`, `/subs`, `/ref`, создание напоминания, snooze, доставка по времени.
2. `cargo deny check` (если внедрено), `cargo clippy --workspace -- -D warnings`.
3. Описать архитектуру в `ARCHITECTURE.md` (обновить существующий).

## 7. Стратегия миграции каждого use case (шаблон)

Чтобы не остановить разработку, миграция каждого use case проходит так:

1. Описать порты, нужные use case-у.
2. Реализовать use case в `application` + юнит-тесты с in-memory реализациями.
3. Реализовать недостающие infrastructure-адаптеры.
4. В legacy handler-ах заменить прямой вызов `Db`/`PaymentService`/`LlmClient` на вызов use case (через `AppFacade`).
5. Убедиться, что `cargo test --workspace` зелёный.
6. Удалить ставший лишним код в legacy.

## 8. Cross-cutting вопросы

### 8.1 Логирование и наблюдаемость

- `tracing` в каждом крейте; контекст: `user_id`, `peer_id`, `reminder_id`, `payment_id`, `event_kind`.
- Span-ы вокруг use case-ов: `tracing::instrument`.
- В долгосрочной перспективе — экспорт метрик (количество созданных напоминаний, retry, доставленных, ошибок YooKassa).

### 8.2 Ошибки

- `domain::DomainError`, `application::ApplicationError`, `infrastructure::InfraError`.
- Каждый внешний слой отображает ошибки внутреннего наверх.
- В `presentation` ошибки превращаются в `Notification::Error` и сообщение пользователю.
- В composition root — `anyhow` для main, остальное `thiserror`.

### 8.3 Конфигурация

- В `bins/bot` — структура `Config`, парсинг env, валидация (отсутствие `VK_ACCESS_TOKEN` падает на старте, не во время отправки).
- Опциональные блоки: `payments`, `twitch`. Если их нет — соответствующие use case-ы получают "выключенные" реализации (`NoOpPaymentGateway` или `Option<PaymentGateway>` в фасаде).
- Настройки лимитов scheduler (batch size, intervals) — в `Config`, не в `const`.

### 8.4 Время

- `Clock` всегда инжектируется в use case. В тестах — фиксированный `FakeClock`.
- Никаких `Utc::now()` внутри domain/application.

### 8.5 Идемпотентность и атомарность

- `ReminderRepository::claim_due(batch_size)` остаётся атомарным (Mongo `findOneAndUpdate`); это требование документируется в trait.
- `PaymentTransactionRepository::mark_fulfilled` идемпотентен.
- `ProcessYooKassaWebhookUseCase` использует `PaymentCache::notify_once` и `try_acquire_fulfill_lock`.

### 8.6 FSM

- `DialogState` — value object в `domain` или `application` (скорее `application`, чтобы знать о входных данных).
- Очистить дублирование: убрать `User.state` из MongoDB или сделать его аудит-полем, не источником истины.
- Реализация `DialogStateStore` — `DashMap` (in-memory) сейчас и `Redis` в будущем без правок use case-ов.

### 8.7 Тесты

- `crates/domain/tests` — чистые тесты доменной логики (`Schedule`, `Reminder` state machine, `Subscription`).
- `crates/application/tests` — use case-тесты с in-memory реализациями всех портов.
- `crates/infrastructure/tests` — mapping-тесты + опциональные testcontainers.
- `crates/presentation/tests` — snapshot-тесты сообщений и клавиатур.
- `tests/` workspace integration — VK-нейтральные прогоны полных сценариев против in-memory реализаций.
- Существующий `tests/scheduler_load_test.rs` адаптируется к новым репозиториям.

### 8.8 Стиль и контроль

- `cargo fmt`, `cargo clippy -D warnings`, `cargo test --workspace` обязательны на каждом коммите фазы.
- `cargo deny` или эквивалент для контроля графа зависимостей крейтов.
- В `CONTRIBUTING.md` (или `ARCHITECTURE.md`) — карта слоёв и правила импортов.

## 9. Контрольные точки готовности

| Фаза | Критерии готовности |
|------|---------------------|
| 0 | Workspace создан, тесты зелёные, бот собирается и запускается. |
| 1 | `domain` крейт компилируется, имеет тесты, используется legacy для базовых типов. |
| 2 | Все основные use cases описаны в `application`, покрыты unit-тестами с in-memory портами. |
| 3 | Все Mongo/Redis/HTTP-адаптеры в `infrastructure`. legacy `api/db.rs`, `api/cache.rs`, `api/llm_client.rs`, `api/payments.rs` пустеют. |
| 4 | `transport-core` + `transport-vk` собраны, `VkTransport` использует общий тип `MessageContent`. |
| 5 | `presentation::Router` обрабатывает все команды и callback-ы; legacy-router удалён или выродился до тонкой обёртки. |
| 6 | `bins/bot` — единственный entrypoint; `crates/legacy` удалён; `telegram-legacy` феaure удалён. |
| 7 | `bins/scheduler` и `bins/webhook` — опциональные сервисы, переиспользуют те же крейты. |
| 8 | Smoke-test в VK пройден, документация обновлена. |

## 10. Риски и стратегии

- Большой объём изменений, риск регрессии. Митигация: пошаговые фазы, каждый use case мигрируется отдельно, существующая поведенческая семантика сохраняется тестами.
- Сохранение совместимости с MongoDB-схемой. Митигация: BSON-маппинг изолируется в `infrastructure`, существующие имена полей сохраняются в DTO; domain структуры свободны.
- Атомарность `claim_due_reminders`. Митигация: трейт документирует атомарные требования; реализация на Mongo `findOneAndUpdate` не меняется.
- VK ограничения клавиатур (911 ошибки). Митигация: `TransportCapabilities` + централизованный `KeyboardSanitizer` в presentation, плюс существующие тесты на лимиты VK сохраняются и расширяются.
- Telegram-legacy: на время фаз 0-5 он остаётся за feature, но не развивается. На фазе 6 удаляется.
- Двойной источник истины state. Митигация: на фазе 2 фиксируем, что источник — `DialogStateStore`. Поле `User.state` в Mongo либо удаляется, либо переводится в "только пишем для аудита, не читаем при логике".
- Соблазн добавить event-bus / outbox / CQRS на этом этапе. Не делать. План остаётся в рамках чистой архитектуры + DDD без избыточных паттернов.

## 11. Что точно не входит в этот рефакторинг

- Полноценный CQRS с разделением read/write моделей.
- Event sourcing.
- Микросервисы с разделением баз данных.
- Замена MongoDB.
- Полная локализация (i18n) — пока остаётся русский.
- Сторонние DI-фреймворки. Композиция вручную в `bins/bot/main.rs`.
- Изменение поведения LLM провайдера и Go-сервиса `llm_api`.
