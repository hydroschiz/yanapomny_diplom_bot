//! Платформонезависимый модуль бота для VK runtime.
//!
//! ## Подмодули
//!
//! - [`router`] - VK long-poll handler: Commands → Text → Callbacks
//! - [`states`] - FSM состояния диалога (AppState)
//! - [`handlers`] - Обработчики сообщений и callbacks
//! - [`keyboards`] - Inline клавиатуры
//!
//! ## Flow обработки сообщений
//!
//! ```text
//! VK Event
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
//!   └─► MessageEvent (inline кнопки)
//!         └─► router → reminder/pay/profile/subs
//! ```

pub mod handlers;
pub mod keyboards;
pub mod router;
pub mod states;
