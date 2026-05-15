# YaPomnyu Bot - Архитектура проекта

VK бот для создания и управления напоминаниями с использованием LLM для парсинга естественного языка.

## Общая схема

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                 main.rs / bin/scheduler.rs / bin/webhook.rs                 │
│                    Точки входа, инициализация логирования                    │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                               app.rs                                         │
│  1. Config::from_env()     ─── Загрузка конфигурации                        │
│  2. Db::connect()          ─── Подключение к MongoDB                        │
│  3. PaymentService         ─── Инициализация YooKassa                       │
│  4. Axum HTTP server       ─── Webhooks (embedded или standalone)           │
│  5. Scheduler              ─── Планировщик (embedded или standalone)        │
│  6. VK long poll           ─── Основной цикл обработки сообщений            │
└─────────────────────────────────────────────────────────────────────────────┘
          │                          │                          │
          ▼                          ▼                          ▼
    ┌──────────┐              ┌──────────┐              ┌──────────────┐
    │   bot/   │              │   api/   │              │  scheduler/  │
    │   VK     │              │ MongoDB  │              │   Фоновая    │
    │ handlers │              │ LLM API  │              │   задача     │
    └──────────┘              └──────────┘              └──────────────┘
```

---

## Структура каталогов

```
src/
├── bin/
│   ├── scheduler.rs       # Standalone scheduler service
│   └── webhook.rs         # Standalone YooKassa webhook service
├── main.rs              # Точка входа
├── app.rs               # Инициализация и запуск компонентов
├── config.rs            # Конфигурация из ENV
│
├── api/                 # Работа с внешними сервисами
│   ├── mod.rs           # Re-exports
│   ├── db.rs            # MongoDB: пользователи, напоминания, платежи
│   ├── cache.rs         # Redis: кэширование pending платежей
│   ├── payments.rs      # YooKassa: создание/обработка платежей
│   ├── llm_client.rs    # HTTP клиент для LLM API
│   ├── llm_models.rs    # Модели данных LLM API (парсинг напоминаний)
│   └── time_calculator.rs # Вычисление времени напоминания из LLM ответа
│
├── bot/                 # VK бот
│   ├── mod.rs           # Re-exports
│   ├── router.rs        # VK long poll роутинг: Commands → Text → Callbacks
│   ├── states/          # Состояния диалогов (FSM)
│   │   └── mod.rs       # AppState enum
│   ├── handlers/        # Обработчики сообщений
│   │   ├── mod.rs       # Re-exports
│   │   ├── commands.rs  # /start, /setup, /list
│   │   ├── text.rs      # Текстовые сообщения (настройки timezone)
│   │   ├── reminder.rs  # Создание/редактирование напоминаний
│   │   └── pay.rs       # Платежи через YooKassa
│   ├── keyboards/       # Inline клавиатуры
│   │   ├── mod.rs       # Re-exports всех клавиатур
│   │   ├── common.rs    # Общие: setup, back, utc
│   │   ├── pay.rs       # Платежи: menu, provider, link
│   │   └── reminder.rs  # Напоминания: confirm, edit, delete
│   └── filters/         # Фильтры для handlers
│       └── mod.rs
│
├── scheduler/           # Планировщик напоминаний
│   └── mod.rs           # Фоновая задача отправки напоминаний
│
└── utils/               # Вспомогательные утилиты
    └── mod.rs
```

---

## Модули

### `api/db.rs` - MongoDB

**Коллекции:**
- `users` - Пользователи с настройками (timezone, snooze, etc.)
- `reminders` - Напоминания
- `records` - Записи о пользователях (legacy)
- `transactions` - Платежи

**Ключевые структуры:**

```rust
/// Пользователь с настройками
pub struct User {
    pub id: i64,              // VK peer_id / chat_id
    pub utc: String,          // UTC offset ("UTC+3")
    pub time_zone: String,    // IANA timezone ("Europe/Moscow")
    pub snooze_buttons: Vec<String>,  // Кнопки отложить
    pub morning: String,      // Время "утро" ("8:00")
    pub afternoon: String,    // Время "день" ("14:00")
    pub evening: String,      // Время "вечер" ("19:00")
}

/// Напоминание
pub struct Reminder {
    pub chat_id: i64,         // VK peer_id / chat_id
    pub text: String,         // Текст напоминания
    pub delay: String,        // Повторение: "", "day", "week", etc.
    pub time: DateTime<Utc>,  // Время срабатывания (UTC)
    pub status: String,       // "active", "processing", "retry", "sent", "failed"
    pub rem_id: Option<i32>,  // Уникальный ID напоминания
    pub retry_count: i32,     // Счётчик retry
    pub retry_at: Option<DateTime<Utc>>,  // Время следующего retry
}
```

**Ключевые методы:**

| Метод | Описание |
|-------|----------|
| `connect()` | Подключение к MongoDB |
| `find_user()` | Найти пользователя по chat_id |
| `upsert_user()` | Создать/обновить пользователя |
| `insert_reminder()` | Создать напоминание |
| `claim_due_reminders()` | Атомарно захватить batch due напоминаний |
| `mark_reminder_sent()` | Пометить как отправленное |
| `schedule_retry()` | Запланировать retry с exponential backoff |

---

### `api/llm_client.rs` - LLM API клиент

HTTP клиент для обращения к Go сервису `llm_api`, который парсит
естественный язык в структурированные напоминания.

```rust
pub struct LlmClient {
    client: reqwest::Client,
    base_url: String,
}

impl LlmClient {
    /// Парсит текст напоминания через LLM
    pub async fn parse_reminder(
        &self,
        text: &str,
        user_timezone: &str,    // "+07:00"
        user_datetime: &str,    // "2025-12-05 00:42"
    ) -> Result<ReminderResponse>;
}
```

---

### `api/llm_models.rs` - Модели LLM API

Модели для парсинга JSON ответов от LLM API.

```rust
/// Ответ от LLM API
pub struct ReminderResponse {
    pub status: String,        // "success" | "error"
    pub reminder: Option<ParsedReminder>,
    pub error: Option<ErrorDetail>,
}

/// Распарсенное напоминание
pub struct ParsedReminder {
    pub description: String,   // Текст напоминания
    pub reminder_type: ReminderType,  // OneTime | Recurring
    pub time_spec: Option<TimeSpec>,  // Спецификация времени
    pub recurrence: Option<RecurrenceInfo>,  // Для повторяющихся
}

/// Спецификация времени
pub struct TimeSpec {
    pub spec_type: TimeSpecType,  // Relative, Weekday, Absolute, etc.
    pub anchor: Option<Anchor>,   // now, today, specific date
    pub offset_minutes: Option<i32>,
    pub offset_hours: Option<i32>,
    pub offset_days: Option<i32>,
    pub weekday: Option<Weekday>,
    pub time: Option<String>,     // "HH:MM"
    pub time_of_day: Option<TimeOfDay>,  // Morning, Afternoon, Evening
}
```

---

### `api/time_calculator.rs` - Вычисление времени

Преобразует `TimeSpec` из LLM в конкретный `DateTime<Utc>`.

```rust
/// Вычисляет время напоминания из спецификации LLM
pub fn calculate_reminder_time(
    parsed: &ParsedReminder,
    now: DateTime<Utc>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<Utc>>;

/// Настройки времени пользователя
pub struct UserTimePrefs {
    pub morning: NaiveTime,    // "Утро" = 08:00
    pub afternoon: NaiveTime,  // "День" = 14:00
    pub evening: NaiveTime,    // "Вечер" = 19:00
    pub timezone_offset_hours: i32,  // Смещение от UTC
}
```

---

### `bot/router.rs` - Роутинг

Определяет обработку VK long poll events:

```rust
impl MessageHandler for AppHandler<VkTransport> {
    async fn handle(&self, event: &Event, api: &VkApi) -> VkResult<()>;
}
```

**Порядок обработки:**
1. Commands (имеют приоритет)
2. Text messages (обрабатываются по состоянию)
3. Callback queries (inline кнопки)

---

### `bot/states/mod.rs` - Состояния диалога

FSM (Finite State Machine) для управления диалогом:

```rust
pub enum AppState {
    /// Ожидание команды или текста напоминания
    Idle,
    
    /// Ожидание подтверждения текста перед LLM
    AwaitingTextConfirmation { pending: PendingText },
    
    /// Ожидание подтверждения распарсенного напоминания
    AwaitingReminderConfirmation { parsed: ParsedReminder },
    
    /// Ожидание ввода часового пояса
    AwaitingUtc,
    
    /// Ожидание выбора кнопок snooze
    AwaitingSnoozeButtons,
    
    /// Ожидание выбора auto-snooze
    AwaitingAutoSnooze,
    
    /// Выбор напоминания для удаления
    AwaitingDeleteSelection { reminders: Vec<(i32, String)> },
}
```

---

### `bot/handlers/reminder.rs` - Создание напоминаний

Основной flow создания напоминания:

```
1. Пользователь отправляет текст
   │
   ▼
2. handle_idle_text()
   ├── Показать: "Создать напоминание? [Да/Нет]"
   └── State → AwaitingTextConfirmation
   │
   ▼
3. handle_text_confirm()  (по нажатию "Да")
   ├── Вызов LLM API: parse_reminder(text, timezone, datetime)
   ├── Вычисление времени: calculate_reminder_time()
   ├── Показать: "Подтвердите: ... [Создать/Изменить/Отменить]"
   └── State → AwaitingReminderConfirmation
   │
   ▼
4. handle_reminder_confirm()  (по нажатию "Создать")
   ├── db.insert_reminder()
   ├── Показать: "✅ Напоминание создано!"
   └── State → Idle
```

---

### `scheduler/mod.rs` - Планировщик

Фоновая задача отправки напоминаний:

```
┌─────────────────────────────────────────────────────────────┐
│                   Scheduler Loop (каждые 10 сек)            │
├─────────────────────────────────────────────────────────────┤
│  1. claim_due_reminders(100)                                │
│     └── MongoDB findOneAndUpdate: status → "processing"     │
│                                                             │
│  2. Параллельная отправка (max 20 concurrent)              │
│     └── VK API: send_message()                              │
│                                                             │
│  3. Обработка результата:                                   │
│     ├── OK → mark_sent() / update_time() (recurring)       │
│     ├── Temp error → schedule_retry() (30s, 60s, 120s)     │
│     └── Permanent error → mark_sent() (user blocked)       │
└─────────────────────────────────────────────────────────────┘
```

**Статусы напоминаний:**

| Статус | Описание |
|--------|----------|
| `active` | Ожидает отправки |
| `processing` | Взято в обработку (атомарный lock) |
| `retry` | Ожидает retry после ошибки |
| `sent` | Успешно отправлено |
| `failed` | Превышено max retries (3) |

---

### `api/payments.rs` - Платежи YooKassa

Интеграция с платёжной системой YooKassa:

```rust
pub struct PaymentService {
    shop_id: String,
    secret_key: String,
    db: Db,
    cache: PaymentCache,  // Redis
}

impl PaymentService {
    /// Создаёт платёж в YooKassa
    pub async fn create_payment(&self, user_id: i64, tariff: &Tariff) 
        -> Result<InitializedPayment>;
    
    /// Axum router для webhook /yookassa/webhook
    pub fn router<T: BotTransport>(self: Arc<Self>, transport: T) -> Router;
}
```

**Flow платежа:**
1. Пользователь нажимает "Оплатить"
2. `create_payment()` → YooKassa API
3. Возвращается URL для оплаты
4. Пользователь оплачивает
5. YooKassa шлёт webhook → `handle_webhook()`
6. Активируется подписка в БД

---

## Переменные окружения

| Переменная | Описание | Обязательна |
|------------|----------|-------------|
| `VK_ACCESS_TOKEN` | Access token сообщества VK | Да |
| `VK_GROUP_ID` | ID сообщества VK | Да |
| `MONGO_URI` | MongoDB connection string | Да |
| `REDIS_URL` | Redis connection string | Нет |
| `LLM_API_URL` | URL LLM API сервиса | Да |
| `YK_SHOP_ID` | YooKassa Shop ID | Для платежей |
| `YK_SECRET_KEY` | YooKassa Secret Key | Для платежей |
| `IP` | IP для HTTP сервера | Нет (0.0.0.0) |
| `PORT` | Порт HTTP сервера | Нет (3001) |
| `ADMINS` | ID админов через запятую | Нет |
| `RUST_LOG` | Уровень логирования | Нет (info) |

---

## Запуск

```bash
# 1. Запустить Ollama (LLM)
ollama serve

# 2. Запустить инфраструктуру
docker compose up -d mongodb1 redis llm_api

# 3. Запустить бот
cargo run
```

---

## Тесты

```bash
# Unit тесты
cargo test

# Тесты нагрузки scheduler
MONGO_URI="mongodb://..." cargo test --test scheduler_load_test -- --nocapture
```
