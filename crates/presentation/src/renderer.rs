use application::Notification as ApplicationNotification;
use transport_core::{
    strip_html, MessageContent, TextFormat, TransportCapabilities, TransportKeyboard,
};

use crate::{
    keyboard::{
        back_keyboard, pay_menu_keyboard, profile_back_keyboard, profile_keyboard, setup_keyboard,
        utc_keyboard, utc_keyboard_page, utc_keyboard_page_count, KeyboardBuilder,
    },
    OutgoingMessage, RenderedResponse,
};

pub const UTC_PROMPT_MESSAGE: &str = r#"Укажите разницу во времени относительно UTC или просто назовите город, где вы находитесь — ИИ-помощник <b>Ян</b> определит всё сам.

Например:
<b>Москва находится в UTC +3.</b>

Напишите UTC или укажите город, где вы находитесь (данные можно в любое время изменить).
Если нужного смещения нет на кнопках, отправьте его текстом, например UTC+5:45:"#;

pub const UTC_SUCCESS_MESSAGE: &str = r#"Часовой пояс <b>+3:00</b> успешно установлен.

Теперь можно создавать напоминания! <b>Отправь текстовое или голосовое сообщение — и бот всё запомнит</b>.

Примеры запросов: <blockquote>
• через 20 минут позвонить руководителю
• в понедельник в 18 — в поликлинику
• в 13:30 — обед
• завтра в 14 — в налоговую
• 16 сентября в 10:20 — на почту
• 17.04.2025 в 9:15 — поздравить коллегу с днём рождения
• в среду утром — оформить документы
• 9 мая в 19:00 — купить билеты
• каждый день в 18 — домой
• каждую среду в 17:30 — на тренировку
• по будням в 10 — планёрка
• каждое 28 число в 20 — оплатить интернет
• каждое 30 мая — купить подарок на годовщину </blockquote>

⚙️ Чтобы изменить часовой пояс, используйте команду <b>/utc</b>
ℹ️ Дополнительная информация — команда <b>/help</b>"#;

pub const SETUP_PROMPT: &str = r#"<b>Выберите раздел для настройки</b>:

• <b>Время откладывания</b> — время, на которое можно перенести напоминание вручную.
• <b>Авто откладывание</b> — настройка автоматического переноса напоминаний.
• <b>Время суток</b> — укажите часовой пояс (UTC) или отправьте свой город."#;

pub const SNOOZE_PROMPT: &str = r#"<b>Выберите кнопки для откладывания напоминаний</b>
Эти варианты будут показываться при получении напоминания.

По умолчанию: <b>15 мин, 1 час, 3 часа</b>

Введите своё время, которое хотите видеть для откладывания:"#;

pub const AUTO_SNOOZE_PROMPT: &str = r#"Настройте время автоматического откладывания напоминаний

По умолчанию: 15 мин

Введите своё время для автоматического откладывания напоминаний:"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimezoneDisplay {
    NotSet,
    Utc(String),
    Named { name: String, offset: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    Start,
    Help,
    Yan,
    SetupMenu,
    UtcPrompt {
        current: TimezoneDisplay,
    },
    UtcPage {
        page: usize,
    },
    UtcSuccess {
        offset: String,
    },
    UtcCancelled,
    SnoozePrompt {
        current: String,
    },
    AutoSnoozePrompt {
        current: String,
    },
    PayMenu {
        is_active: bool,
        expiry: Option<String>,
    },
    PaymentDisabled,
    Profile {
        user_id: i64,
        utc_offset: String,
        snooze_buttons: String,
        auto_snooze: String,
        subscription: String,
    },
    PlainText {
        text: String,
        keyboard: Option<TransportKeyboard>,
    },
    Error(String),
}

impl From<ApplicationNotification> for Notification {
    fn from(notification: ApplicationNotification) -> Self {
        match notification {
            ApplicationNotification::Text { text, .. } => Self::PlainText {
                text,
                keyboard: None,
            },
            ApplicationNotification::ReminderDue { text, .. } => Self::PlainText {
                text,
                keyboard: None,
            },
            ApplicationNotification::Profile(profile) => Self::Profile {
                user_id: profile.user_id.value(),
                utc_offset: "не загружен".to_string(),
                snooze_buttons: "не загружены".to_string(),
                auto_snooze: "не загружено".to_string(),
                subscription: "не загружена".to_string(),
            },
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Renderer;

impl Renderer {
    pub fn render(
        &self,
        notification: Notification,
        capabilities: TransportCapabilities,
    ) -> MessageContent {
        match notification {
            Notification::Start => self.message(start_text(), None, capabilities),
            Notification::Help => self.message(
                help_text(),
                Some(profile_back_keyboard(capabilities)),
                capabilities,
            ),
            Notification::Yan => self.message(
                yan_text(),
                Some(profile_back_keyboard(capabilities)),
                capabilities,
            ),
            Notification::SetupMenu => {
                self.message(SETUP_PROMPT, Some(setup_keyboard(capabilities)), capabilities)
            }
            Notification::UtcPrompt { current } => {
                let page_count = utc_keyboard_page_count(capabilities);
                let text = format!(
                    "{}\n\n{}\n\nСтраница 1/{}",
                    current_timezone_text(&current),
                    UTC_PROMPT_MESSAGE,
                    page_count
                );
                self.message(text, Some(utc_keyboard(capabilities)), capabilities)
            }
            Notification::UtcPage { page } => {
                let page_count = utc_keyboard_page_count(capabilities);
                let text = format!(
                    "Выберите UTC смещение кнопкой или отправьте город/смещение текстом.\n\nСтраница {}/{}",
                    page % page_count + 1,
                    page_count
                );
                self.message(text, Some(utc_keyboard_page(capabilities, page)), capabilities)
            }
            Notification::UtcSuccess { offset } => {
                self.message(UTC_SUCCESS_MESSAGE.replace("+3:00", &offset), None, capabilities)
            }
            Notification::UtcCancelled => {
                self.message("Настройка часового пояса отменена.", None, capabilities)
            }
            Notification::SnoozePrompt { current } => self.message(
                format!("{}\n\nТекущие: <b>{}</b>", SNOOZE_PROMPT, current),
                Some(back_keyboard(capabilities)),
                capabilities,
            ),
            Notification::AutoSnoozePrompt { current } => self.message(
                format!("{}\n\nТекущее: <b>{}</b>", AUTO_SNOOZE_PROMPT, current),
                Some(back_keyboard(capabilities)),
                capabilities,
            ),
            Notification::PayMenu { is_active, expiry } => self.message(
                format_subscription_status(is_active, expiry.as_deref()),
                Some(pay_menu_keyboard(capabilities)),
                capabilities,
            ),
            Notification::PaymentDisabled => self.message(
                "⚠️ Платёжный контур сейчас отключён. Базовые сценарии напоминаний работают в reminder-only режиме.",
                None,
                capabilities,
            ),
            Notification::Profile {
                user_id,
                utc_offset,
                snooze_buttons,
                auto_snooze,
                subscription,
            } => self.message(
                format_profile(
                    user_id,
                    &utc_offset,
                    &snooze_buttons,
                    &auto_snooze,
                    &subscription,
                ),
                Some(profile_keyboard(capabilities)),
                capabilities,
            ),
            Notification::PlainText { text, keyboard } => self.message(text, keyboard, capabilities),
            Notification::Error(text) => self.message(format!("⚠️ {}", text), None, capabilities),
        }
    }

    pub fn render_message(
        &self,
        peer_id: i64,
        notification: Notification,
        capabilities: TransportCapabilities,
    ) -> OutgoingMessage {
        OutgoingMessage::new(peer_id, self.render(notification, capabilities))
    }

    pub fn render_response(
        &self,
        peer_id: i64,
        notification: Notification,
        capabilities: TransportCapabilities,
    ) -> RenderedResponse {
        RenderedResponse::message(peer_id, self.render(notification, capabilities))
    }

    fn message(
        &self,
        text: impl Into<String>,
        keyboard: Option<TransportKeyboard>,
        capabilities: TransportCapabilities,
    ) -> MessageContent {
        let text = text.into();
        let (text, format) = if capabilities.supports_html {
            (text, TextFormat::Html)
        } else {
            (strip_html(&text), TextFormat::Plain)
        };
        let keyboard = keyboard.map(|keyboard| KeyboardBuilder::new(capabilities).fit(keyboard));

        MessageContent {
            text,
            format,
            keyboard,
        }
    }
}

pub fn format_subscription_status(is_active: bool, expiry: Option<&str>) -> String {
    let status = if is_active {
        "активна ✅"
    } else {
        "неактивна ❌"
    };
    let expiry_line = if is_active {
        expiry
            .map(|expiry| format!("\n📅 <b>Действует до:</b> {}", expiry))
            .unwrap_or_default()
    } else {
        String::new()
    };

    format!(
        "👛 <b>Выберите срок, на который хотите оформить подписку</b>\n\n\
         📧 <b>Статус:</b> {}{}\n\n\
         <i>Совет:</i> <b>выбирайте более длительную подписку</b>, чтобы снизить стоимость одного месяца.",
        status, expiry_line
    )
}

fn format_profile(
    user_id: i64,
    utc_offset: &str,
    snooze_buttons: &str,
    auto_snooze: &str,
    subscription: &str,
) -> String {
    format!(
        "👤 <b>Профиль #{}</b>\n\n\
         💎 <b>Подписка:</b> {}\n\
         🌍 <b>Часовой пояс:</b> UTC {}\n\
         ⏰ <b>Кнопки откладывания:</b> {}\n\
         🔁 <b>Автооткладывание:</b> {}\n\n\
         Используйте кнопки ниже, чтобы открыть нужный раздел.",
        user_id, subscription, utc_offset, snooze_buttons, auto_snooze
    )
}

fn current_timezone_text(current: &TimezoneDisplay) -> String {
    match current {
        TimezoneDisplay::NotSet => "Текущий часовой пояс: <b>не установлен</b>".to_string(),
        TimezoneDisplay::Utc(offset) => format!("Текущий часовой пояс: <b>UTC {}</b>", offset),
        TimezoneDisplay::Named { name, offset } => {
            format!("Текущий часовой пояс: <b>{} ({})</b>", name, offset)
        }
    }
}

fn start_text() -> &'static str {
    r#"<b>YANAPOMNYU</b> — твой личный помощник для организации дел!

Создавай напоминания, планируй задачи и получай уведомления без лишних приложений. Внутри тебя ждёт ИИ-помощник <b>Ян</b> — он мгновенно создаст напоминания, подскажет, как улучшить тайм-менеджмент, и поможет быть продуктивнее.

Узнай больше о Яне через команду /yan.

✨ Возможности бота:<blockquote>
• Создание напоминаний на любую дату и время
• Отслеживание всех задач в одном месте
• Автоматические уведомления о важных делах</blockquote>

📺 <b>Получай уведомления о новых видео и трансляциях бесплатно!</b>
Подписка не нужна — просто отправь ссылку через <b>/subs</b> (YouTube или Twitch), и я буду напоминать о новом контенте.

📢 Новости и обновления — в канале @yanapomnyu"#
}

fn help_text() -> &'static str {
    r#"💬 <b>Создавайте напоминания своими словами!</b>

Вы можете отправлять <b>текстовые</b> или <b>голосовые сообщения</b>, а ИИ-помощник <b>Ян</b> сам распознает, что и когда нужно запланировать.

Примеры:<blockquote>
• через 20 минут позвонить руководителю
• в понедельник в 18 — в поликлинику
• в 13:30 — обед
• завтра в 14 — в налоговую
• 16 сентября в 10:20 — на почту
• 17.04.2017 в 9:15 — поздравить коллегу с днём рождения
• в среду утром — оформить документы
• 9 мая в 19:00 — купить билеты
• каждый день в 18 — домой
• каждую среду в 17:30 — на тренировку
• по будням в 10 — планёрка
• каждое 28 число в 20 — оплатить интернет
• каждое 30 мая — подарок на годовщину</blockquote>

👥 <b>Использование в группах и каналах</b>:
Добавьте бота в групповой чат или канал и настройте часовой пояс с помощью команды /start.

Для напоминаний в группах указывайте имя бота в тексте.

📞 <b>Вопросы или предложения</b>:

Пишите в чат технической поддержки: @yanapomnyu_support"#
}

fn yan_text() -> &'static str {
    r#"Привет! Я — <b>Ян</b>, твой персональный ИИ-помощник 🧠 Я помогу тебе управлять временем, делами и напоминаниями.

<b>Вот что я умею</b>:<blockquote>
• Автоматически создавать напоминания по любому тексту — просто напиши, что и когда сделать.
• Подсказывать, как лучше распределить задачи и не перегружать день.
• Давать советы по тайм-менеджменту и концентрации.
• Анализировать твои напоминания и помогать выстроить привычки.</blockquote>

<b>Попробуй прямо сейчас</b>:
💬 "Завтра в 9:30 совещание с командой""#
}
