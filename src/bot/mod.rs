//! Telegram бот модуль.
//!
//! ## Подмодули
//!
//! - [`router`] - Схема роутинга: Commands → Text → Callbacks
//! - [`states`] - FSM состояния диалога (AppState)
//! - [`handlers`] - Обработчики сообщений и callbacks
//! - [`keyboards`] - Inline клавиатуры
//! - [`filters`] - Фильтры для handlers
//!
//! ## Flow обработки сообщений
//!
//! ```text
//! Update
//!   │
//!   ├─► Message
//!   │     ├─► Command (/start, /setup, /list)
//!   │     │     └─► handlers::commands
//!   │     │
//!   │     └─► Text (зависит от AppState)
//!   │           ├─► Idle → handlers::reminder (создание)
//!   │           ├─► AwaitingUtc → handlers::text (timezone)
//!   │           └─► AwaitingSnooze → handlers::text (snooze)
//!   │
//!   └─► CallbackQuery (inline кнопки)
//!         └─► handlers::callbacks → reminder/pay
//! ```

pub mod filters;
pub mod handlers;
pub mod keyboards;
pub mod router;
pub mod states;
