# Миграция бота yanapomnyu: Telegram → VK

## Принятые решения

| Вопрос | Решение |
|---|---|
| Архитектура | **Abstraction Layer** — трейты `BotTransport` + `DialogueStore` |
| HTML | **Plain text + emoji** (VK не поддерживает HTML) |
| FSM | **DashMap\<i64, AppState\>** (in-memory, без Redis) |
| Реферальные ссылки | **TODO** — закомментировать, пометить, не ломать |
| Платежи | **YooKassa остаётся**, ссылка открывается в браузере |
| БД | **Без изменений** |

---

## Фаза 1: Transport Abstraction Layer

**Цель:** Создать трейты, от которых будут зависеть handlers, вместо прямой зависимости от teloxide/vk-bot-api.

#### [NEW] `src/transport/mod.rs`
```rust
pub mod traits;
pub mod vk;
pub mod dialogue_store;
pub mod text_format;
```

#### [NEW] `src/transport/traits.rs`
Абстрактный интерфейс отправки сообщений:
```rust
#[async_trait]
pub trait BotTransport: Send + Sync + Clone {
    async fn send_text(&self, peer_id: i64, text: &str) -> anyhow::Result<()>;
    async fn send_with_keyboard(&self, peer_id: i64, text: &str, kb: &TransportKeyboard) -> anyhow::Result<()>;
    async fn answer_callback(&self, event_id: &str, user_id: i64, peer_id: i64) -> anyhow::Result<()>;
}
```

Абстрактная клавиатура:
```rust
pub struct TransportKeyboard { /* rows of TransportButton */ }
pub enum TransportButton {
    Callback { label: String, data: String },
    Url { label: String, url: String },
}
```

#### [NEW] `src/transport/vk.rs`
Имплементация `BotTransport` через `vk_bot_api::VkApi`.

#### [NEW] `src/transport/text_format.rs`
Утилита `strip_html()` — удаление `<b>`, `<i>`, `<a href>` тегов, замена на plain text.

**Результат фазы:** Компилируемый модуль `transport` с трейтами и VK-реализацией. Остальной код пока не трогаем.

---

## Фаза 2: FSM State Store (DialogueStore)

**Цель:** Замена `teloxide::Dialogue<AppState>` на собственное хранилище.

#### [NEW] `src/transport/dialogue_store.rs`
```rust
use dashmap::DashMap;
use crate::bot::states::AppState;

#[derive(Clone)]
pub struct DialogueStore {
    states: Arc<DashMap<i64, AppState>>,
}

impl DialogueStore {
    pub fn new() -> Self;
    pub fn get(&self, user_id: i64) -> AppState;        // default = Idle
    pub fn update(&self, user_id: i64, state: AppState);
    pub fn reset(&self, user_id: i64);                   // → Idle
}
```

#### [MODIFY] `Cargo.toml`
Добавить `dashmap = "6"`.

**Результат фазы:** Рабочий DialogueStore, покрытый юнит-тестами. Ещё не подключён к handlers.

---

## Фаза 3: Клавиатуры → TransportKeyboard

**Цель:** Перевести все клавиатуры с `teloxide::InlineKeyboardMarkup` на `TransportKeyboard`.

#### [MODIFY] `src/bot/keyboards/common.rs`
#### [MODIFY] `src/bot/keyboards/reminder.rs`
#### [MODIFY] `src/bot/keyboards/pay.rs`
#### [MODIFY] `src/bot/keyboards/profile.rs`
#### [MODIFY] `src/bot/keyboards/channels.rs`
#### [MODIFY] `src/bot/keyboards/mod.rs`

Каждая функция возвращает `TransportKeyboard` вместо `InlineKeyboardMarkup`.

**Результат фазы:** Все клавиатуры используют абстрактный тип. Код пока не компилируется (handlers ещё ссылаются на старые типы).

---

## Фаза 4: Адаптация Handlers

**Цель:** Перевести все handlers на абстрактные типы. Самая объёмная фаза.

Общая схема изменений в каждом handler:

| Было | Стало |
|---|---|
| `bot: Bot` | `transport: &impl BotTransport` |
| `msg: teloxide::Message` | `peer_id: i64, user_id: i64, text: &str` |
| `cq: CallbackQuery` | `event_id: &str, user_id: i64, peer_id: i64, payload: &str` |
| `dialogue: AppDialogue` | `store: &DialogueStore` |
| `bot.send_message(...).parse_mode(Html)` | `transport.send_text(peer_id, &strip_html(text))` |
| `dialogue.update(state).await?` | `store.update(user_id, state)` |

#### [MODIFY] `src/bot/handlers/reminder.rs` — 🔴 High (776 строк)
#### [MODIFY] `src/bot/handlers/pay.rs` — 🟡 Medium
#### [MODIFY] `src/bot/handlers/profile.rs` — 🟢 Low
#### [MODIFY] `src/bot/handlers/channels.rs` — 🟡 Medium

#### [MODIFY] `src/bot/handlers/referral.rs` — 🟢 Low
- Закомментировать реферальную логику
- Пометить `// TODO(vk-migration): реферальные ссылки VK`
- Команда `/ref` отвечает: «Реферальная программа временно недоступна»

#### [REWRITE] `src/bot/router.rs`
Новый `AppHandler` реализующий `vk_bot_api::MessageHandler`:
- `Event::MessageNew` → роутинг по команде/состоянию → вызов handler
- `Event::MessageEvent` → роутинг по payload → вызов callback handler

#### [DELETE] `src/bot/filters/mod.rs`
Фильтрация `private_chat` / `group_chat` переносится в `AppHandler::handle()` через `peer_id` проверку.

**Результат фазы:** Все handlers работают через абстракции. Код компилируется с VK-реализацией.

---

## Фаза 5: Scheduler + Payments

**Цель:** Заменить `teloxide::Bot` на `impl BotTransport` в фоновых задачах.

#### [MODIFY] `src/scheduler/mod.rs`
```diff
-pub fn start_scheduler(bot: Bot, db: Db)
+pub fn start_scheduler(transport: impl BotTransport + 'static, db: Db)
```
- `send_reminder`: `transport.send_with_keyboard(...)` вместо `bot.send_message(...)`
- Убрать `ParseMode::Html`, использовать `strip_html()`

#### [MODIFY] `src/scheduler/subscription.rs`
#### [MODIFY] `src/scheduler/channels.rs`

#### [MODIFY] `src/api/payments.rs`
```diff
-pub async fn handle_webhook(&self, bot: &Bot, ...)
+pub async fn handle_webhook(&self, transport: &impl BotTransport, ...)
```
- `fulfill_payment`, `manual_check` — заменить `bot.send_message` → `transport.send_text`
- Webhook Axum-роутер — без изменений (платформонезависим)

**Результат фазы:** Весь код использует абстракции. Telegram-зависимости полностью удалены.

---

## Фаза 6: Финальная сборка

#### [MODIFY] `Cargo.toml`
```diff
-teloxide = { version = "0.13", features = ["macros"] }
+vk-bot-api = { version = "1.0", features = ["full"] }
+dashmap = "6"
+async-trait = "0.1"
```

#### [MODIFY] `src/config.rs`
```diff
-// TELOXIDE_TOKEN загружается teloxide автоматически
+pub vk_access_token: String,   // env: VK_ACCESS_TOKEN
+pub vk_group_id: i64,          // env: VK_GROUP_ID
-pub webhook_url: Option<String>,
-pub webhook_port: Option<u16>,
```

#### [MODIFY] `src/app.rs`
```rust
let vk_transport = VkTransport::new(&config.vk_access_token, config.vk_group_id)?;
let dialogue_store = DialogueStore::new();

let mut vk_bot = VkBot::builder()
    .token(&config.vk_access_token)
    .group_id(config.vk_group_id)
    .build()?;

let handler = AppHandler::new(vk_transport.clone(), db.clone(), llm, payment_svc, dialogue_store);
vk_bot.add_handler(handler);

start_scheduler(vk_transport.clone(), db.clone());
start_subscription_scheduler(vk_transport.clone(), db.clone());

tokio::select! {
    _ = vk_bot.run() => {},
    _ = axum_server => {},  // YooKassa webhooks
}
```

#### [MODIFY] `src/bot/mod.rs`
Обновить документацию и экспорты.

**Результат фазы:** Бот запускается и работает на VK.

---

## Сводка файлов по фазам

| Фаза | Файлы | Новые | Изменённые | Удалённые |
|---|---|---|---|---|
| 1. Transport | 4 | 4 | 0 | 0 |
| 2. FSM | 1 | 1 (+Cargo.toml) | 1 | 0 |
| 3. Keyboards | 6 | 0 | 6 | 0 |
| 4. Handlers | 8 | 0 | 7 | 1 |
| 5. Scheduler/Pay | 4 | 0 | 4 | 0 |
| 6. Assembly | 4 | 0 | 4 | 0 |
| **Итого** | **~20** | **5** | **~14** | **1** |

## Verification Plan

```bash
# После каждой фазы:
cargo check          # компиляция
cargo clippy         # линтер
cargo test           # юнит-тесты (time_calculator, parse_channel_url и т.д.)
```

### Manual (после фазы 6):
1. `/start` → приветствие + клавиатура
2. Отправить текст → создание напоминания через LLM → подтверждение
3. `/list` → список напоминаний
4. `/pay` → тарифы → ссылка YooKassa
5. `/profile` → статистика
6. Планировщик → напоминание приходит вовремя
