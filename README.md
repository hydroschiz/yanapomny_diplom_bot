# YaPomnyu Bot 🤖⏰

VK бот для создания и управления напоминаниями с использованием естественного языка и LLM для парсинга.

<div align="center">

[![Rust](https://img.shields.io/badge/rust-1.81+-orange.svg)](https://www.rust-lang.org/)
[![Go](https://img.shields.io/badge/go-1.23+-00ADD8.svg)](https://golang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MongoDB](https://img.shields.io/badge/MongoDB-6.0-green.svg)](https://www.mongodb.com/)

</div>

## ✨ Основные возможности

- 🗣️ **Естественный язык** — создавайте напоминания обычными фразами: "напомни завтра в 15:00 купить молоко"
- 🤖 **AI парсинг** — использует LLM (OpenRouter) для понимания контекста
- 🔄 **Повторяющиеся напоминания** — ежедневно, еженедельно, ежемесячно
- ⏰ **Отложить (Snooze)** — настраиваемые кнопки отсрочки
- 🌍 **Часовые пояса** — автоматический учет временных зон
- 💳 **Подписка** — интеграция с YooKassa для оплаты
- 📺 **YouTube/Twitch** — уведомления о новых видео и стримах
- 👥 **Реферальная программа** — бонусы за приглашение друзей

## 🏗️ Архитектура

Проект состоит из трех основных компонентов:

```
┌─────────────────────────────────────────────────────────┐
│                       VK Bot (Rust)                      │
│  • vk-bot-api - VK long poll                            │
│  • MongoDB - хранение данных                            │
│  • Redis - кэширование платежей                         │
│  • YooKassa - платежная система                         │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    LLM API (Go)                          │
│  • OpenRouter AI - парсинг естественного языка          │
│  • REST API для преобразования текста в JSON            │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    MongoDB Database                      │
│  • users - настройки пользователей                      │
│  • reminds - напоминания                                │
│  • records - подписки                                   │
│  • transactions - платежи                               │
└─────────────────────────────────────────────────────────┘
```

## 🚀 Быстрый старт

### Локальная разработка

1. **Клонируйте репозиторий**

```bash
git clone <repo-url>
cd yanapomnyu_bot
```

2. **Создайте .env файлы**

```bash
# Основной .env для бота и docker-compose
cp .env.example .env

# Опционально: .env для LLM API (если хотите переопределить настройки)
# cp llm_api/.env.example llm_api/.env

# Отредактируйте .env, добавьте свои токены
nano .env
```

3. **Запустите инфраструктуру**

```bash
docker compose up -d mongodb redis llm_api
```

4. **Запустите бота**

```bash
cargo run
```

### Production развертывание

Для развертывания на сервере используйте готовый скрипт:

```bash
./DEPLOY_QUICK.sh user@your-server-ip
```

Или следуйте подробному руководству: **[DEPLOYMENT.md](DEPLOYMENT.md)**

## 📦 Структура проекта

```
yanapomnyu_bot/
├── src/
│   ├── api/              # Внешние сервисы (MongoDB, LLM, YooKassa)
│   ├── bot/              # VK бот (handlers, keyboards, states)
│   ├── scheduler/        # Планировщик отправки напоминаний
│   ├── config.rs         # Конфигурация из ENV
│   ├── app.rs            # Инициализация приложения
│   └── main.rs           # Точка входа
├── Cargo.toml            # Зависимости Rust
├── Dockerfile            # Сборка Docker образа
├── docker-compose.yml    # Локальная разработка
├── docker-compose.prod.yml  # Production конфигурация
├── .env.example          # Пример переменных окружения
├── DEPLOYMENT.md         # Подробное руководство по развертыванию
├── ARCHITECTURE.md       # Описание архитектуры
└── README.md             # Этот файл
```

## 🔧 Конфигурация

Все настройки задаются через переменные окружения. Создайте файл `.env` на основе `.env.example`:

### Обязательные переменные

| Переменная | Описание |
|------------|----------|
| `VK_ACCESS_TOKEN` | Access token сообщества VK |
| `VK_GROUP_ID` | ID сообщества VK |
| `MONGO_URI` | MongoDB connection string |
| `REDIS_URL` | Redis URL для кэширования |
| `LLM_API_URL` | URL LLM API сервиса |

### Опциональные переменные

| Переменная | Описание | По умолчанию |
|------------|----------|--------------|
| `ADMINS` | ID администраторов (через запятую) | - |
| `PAYMENTS_ENABLED` | Включить YooKassa-контур; для reminder-only ставьте `false` | auto by creds |
| `YK_SHOP_ID` | YooKassa Shop ID | test |
| `YK_SECRET_KEY` | YooKassa Secret Key | test |
| `IP` | IP для HTTP сервера | 0.0.0.0 |
| `PORT` | Порт HTTP сервера | 3001 |
| `RUST_LOG` | Уровень логирования | info |

Подробнее см. [.env.example](.env.example)

## 🗄️ Совместимость с legacy версией

Проект полностью совместим со старой MongoDB схемой (Go версия):

- ✅ Коллекция `users` — все поля сохранены
- ✅ Коллекция `reminds` — используется `remID`
- ✅ Коллекция `records` — управление подписками
- ✅ Счетчик напоминаний — механизм совместим

**Миграция:** Просто восстановите дамп старой базы в новый MongoDB.

## 📚 Документация

- **[ARCHITECTURE.md](ARCHITECTURE.md)** — подробное описание архитектуры, модулей и потоков данных
- **[DEPLOYMENT.md](DEPLOYMENT.md)** — полное руководство по развертыванию на production сервере
- **[.env.example](.env.example)** — описание всех переменных окружения

## 🛠️ Разработка

### Требования

- Rust 1.81+
- Docker & Docker Compose
- MongoDB 6.0+
- Redis 7+
- Go 1.23+ (для LLM API)

### Полезные команды

```bash
# Сборка проекта
cargo build --release

# Запуск тестов
cargo test

# Проверка кода
cargo clippy

# Форматирование
cargo fmt

# Запуск с логами
RUST_LOG=debug cargo run
```

### Структура БД

```javascript
// users
{
  id: 123456789,           // VK peer_id / chat_id
  utc: "UTC+3",           // Часовой пояс
  timezone: "Europe/Moscow",
  delay: ["1hourSnooze", "3hourSnooze"],  // Кнопки отложить
  morning: "8:00",        // Время "утро"
  afternoon: "14:00",     // Время "день"
  evening: "19:00"        // Время "вечер"
}

// reminds
{
  id: 123456789,          // chat_id
  text: "Купить молоко",
  time: ISODate("2025-01-10T12:00:00Z"),
  delay: "day",           // "" | "day" | "week" | "month"
  status: "active",       // active | processing | sent | failed
  remID: 42               // Уникальный ID
}

// records
{
  id: 123456789,
  nextPaymentDate: ISODate("2025-12-31T23:59:59Z"),
  active: true,
  freestate: 1            // 1=trial, 2=paid
}
```

## 📊 Мониторинг

### Логи

```bash
# Все логи
docker compose logs -f

# Только бот
docker compose logs -f bot

# Последние 100 строк
docker compose logs --tail=100
```

### Статус сервисов

```bash
docker compose ps
docker stats
```

### Бэкап БД

```bash
# Создание дампа
docker exec yanapomnyu-mongo mongodump \
  --username admin \
  --password PASSWORD \
  --authenticationDatabase admin \
  --out=/dump

# Копирование на локальную машину
docker cp yanapomnyu-mongo:/dump ./backup-$(date +%Y%m%d)
```

## 🤝 Участие в разработке

1. Fork репозитория
2. Создайте feature branch (`git checkout -b feature/amazing-feature`)
3. Commit изменений (`git commit -m 'Add amazing feature'`)
4. Push в branch (`git push origin feature/amazing-feature`)
5. Откройте Pull Request

## 📝 Лицензия

MIT License - см. [LICENSE](LICENSE)

## 👨‍💻 Автор

Разработано с ❤️ на Rust + Go

---

**Полезные ссылки:**

- [vk-bot-api crate](https://crates.io/crates/vk-bot-api)
- [MongoDB Rust Driver](https://docs.rs/mongodb/)
- [YooKassa API](https://yookassa.ru/developers/api)
- [OpenRouter AI](https://openrouter.ai/)
