# 🚀 Руководство по развертыванию YaPomnyu Bot (Production Ready)

Это руководство позволит развернуть проект на сервере с минимальным потреблением трафика и диска. Мы будем копировать **только** необходимые исходники.

## ⚠️ Важно: О .env файлах

В проекте используются **ДВА** `.env` файла:

| Файл | Путь | Обязательный? | Назначение |
|------|------|---------------|------------|
| **Основной** | `/opt/yanapomnyu/.env` | ⭐ **ДА** | Docker Compose, MongoDB, Bot Token, YooKassa |
| **LLM API** | `/opt/yanapomnyu/llm_api/.env` | 🔧 Нет | Переопределение настроек LLM сервиса (опционально) |

**Для начала создайте ТОЛЬКО основной `.env`** - этого достаточно! Подробнее см. [docs/env_files_explained.md](docs/env_files_explained.md)

## 📋 Содержание

- [Предварительные требования](#предварительные-требования)
- [Шаг 1. Подготовка сервера](#шаг-1-подготовка-сервера)
- [Шаг 2. Умное копирование файлов (rsync)](#шаг-2-умное-копирование-файлов-rsync)
- [Шаг 3. Настройка переменных окружения](#шаг-3-настройка-переменных-окружения)
- [Шаг 4. Запуск](#шаг-4-запуск)
- [Шаг 5. Восстановление базы (если есть дамп)](#шаг-5-восстановление-базы-если-есть-дамп)
- [Обслуживание и мониторинг](#обслуживание-и-мониторинг)
- [Решение проблем](#решение-проблем)

---

## Предварительные требования

- **Чистый сервер Ubuntu** (22.04+) или Debian (11+)
- **SSH доступ** (желательно по ключу)
- **Минимум 2GB RAM** (рекомендуется 4GB)
- **Минимум 10GB свободного места** на диске
- **Утилита `rsync`** на вашем локальном компьютере (есть на Linux/macOS, для Windows используйте WSL или Git Bash)

---

## Шаг 1. Подготовка сервера

### 1.1. Установка Docker

Зайдите на сервер и установите Docker одной командой:

```bash
curl -fsSL https://get.docker.com -o get-docker.sh
sh get-docker.sh
```

### 1.2. Проверка установки

```bash
docker compose version
```

Должна вывестись версия Docker Compose (например: `Docker Compose version v2.20.3`).

### 1.3. Создание рабочей директории

```bash
mkdir -p /opt/yanapomnyu
```

---

## Шаг 2. Умное копирование файлов (rsync)

Мы будем копировать файлы с вашего компьютера на сервер, автоматически исключая всё лишнее (тесты, легаси код, локальные данные, скомпилированные бинарники).

### 2.1. Копирование Rust проекта (Telegram Bot)

**Важно:** Выполняйте команды из **локального компьютера**, находясь в родительской директории проекта.

```bash
# Замените user@your-server-ip на ваши данные SSH
rsync -avz --delete \
  --include='src/***' \
  --include='Cargo.toml' \
  --include='Cargo.lock' \
  --include='Dockerfile' \
  --include='docker-compose.prod.yml' \
  --exclude='target' \
  --exclude='target_user' \
  --exclude='legacy_go' \
  --exclude='docs' \
  --exclude='tests' \
  --exclude='data' \
  --exclude='data_production' \
  --exclude='*.zip' \
  --exclude='.git' \
  --exclude='.env*' \
  yanapomnyu_bot/ user@your-server-ip:/opt/yanapomnyu/yanapomnyu_bot/
```

**Что делает команда:**
- `--include='src/***'` — копирует всю папку `src/` с исходным кодом
- `--exclude='target'` — исключает скомпилированные бинарники (экономит гигабайты)
- `--exclude='legacy_go'` — исключает старый код на Go
- `--exclude='data'` — исключает локальные базы данных
- `--delete` — удаляет файлы на сервере, которых нет локально (синхронизация)

### 2.2. Копирование Go проекта (LLM API)

```bash
rsync -avz --delete \
  --include='cmd/***' \
  --include='internal/***' \
  --include='pkg/***' \
  --include='config/***' \
  --include='go.mod' \
  --include='go.sum' \
  --include='Dockerfile' \
  --exclude='tmp' \
  --exclude='*_TESTS.md' \
  --exclude='*_test.go' \
  --exclude='.git' \
  llm_api/ user@your-server-ip:/opt/yanapomnyu/llm_api/
```

### 2.3. Копирование docker-compose конфигурации

Используем production версию конфигурации:

```bash
scp yanapomnyu_bot/docker-compose.prod.yml user@your-server-ip:/opt/yanapomnyu/docker-compose.yml
```

---

## Шаг 3. Настройка переменных окружения

⚠️ **ВАЖНО**: В проекте используются **ДВА** файла `.env`:
- `/opt/yanapomnyu/.env` - основной файл для Docker Compose и бота
- `/opt/yanapomnyu/llm_api/.env` - для Go LLM API сервиса (опционально)

### 3.1. Подключение к серверу

```bash
ssh user@your-server-ip
cd /opt/yanapomnyu
```

### 3.2. Создание основного файла .env

Этот файл используется Docker Compose и ботом:

```bash
nano .env
```

### 3.3. Заполнение основной конфигурации

Вставьте следующее содержимое, заменив значения на ваши:

```env
# ============================================
# База данных MongoDB
# ============================================
MONGO_USER=admin
MONGO_PASS=ВАШ_СЛОЖНЫЙ_ПАРОЛЬ_ЗДЕСЬ

# ============================================
# Telegram Bot
# ============================================
# Токен бота от @BotFather
BOT_TOKEN=1234567890:ABCdefGHIjklMNOpqrsTUVwxyz

# ID администраторов (через запятую, получите у @userinfobot)
ADMINS_ID=123456789,987654321

# ============================================
# YooKassa (платежи)
# ============================================
# Получите на https://yookassa.ru/
YK_SHOP_ID=123456
YK_SECRET_KEY=live_ваш_секретный_ключ

# ============================================
# OpenRouter API (LLM для парсинга)
# ============================================
# НЕОБЯЗАТЕЛЬНО - по умолчанию используется бесплатный API key
# Получите свой на https://openrouter.ai/ если нужен больший лимит
# Эта переменная передается в LLM API контейнер через docker-compose.yml
# OPENAI_API_KEY=sk-or-v1-ваш_ключ_здесь
```

**Примечание о структуре .env файлов:**
- Основной `.env` в корне `/opt/yanapomnyu/` используется Docker Compose для подстановки переменных
- LLM API может иметь свой `.env` в `/opt/yanapomnyu/llm_api/.env` (опционально)
- Если не создать `llm_api/.env`, сервис использует переменные из docker-compose.yml

**Сохраните файл:**
- `Ctrl+O` → `Enter` → `Ctrl+X`

### 3.4. Создание .env для LLM API (опционально)

LLM API сервис может использовать свой собственный `.env` файл для переопределения настроек.
Это **опционально** - по умолчанию используются значения из docker-compose.yml.

```bash
cd llm_api
nano .env
```

Вставьте (если хотите переопределить настройки):

```env
# ============================================
# LLM API Service Configuration
# ============================================
HOST=0.0.0.0
PORT=8080

# OpenRouter API (опционально - уже есть в docker-compose)
# OPENAI_BASE_URL=https://openrouter.ai/api/v1/
# OPENAI_API_KEY=sk-or-v1-ваш_ключ_здесь
# OPENAI_MODEL=google/gemma-3-27b-it:free
```

Сохраните и вернитесь в корневую директорию:

```bash
cd ..
```

**Примечание:** Если вы не создадите `.env` в `llm_api/`, сервис будет использовать переменные из `docker-compose.yml` (переменная `OPENAI_API_KEY` из корневого `.env`).

### 3.5. Проверка безопасности

Убедитесь, что файлы `.env` не имеют доступа для чтения другим пользователям:

```bash
chmod 600 .env
chmod 600 llm_api/.env 2>/dev/null || true  # Если создали
```

---

## Шаг 4. Запуск

### 4.1. Проверка структуры файлов

Убедитесь, что файлы на месте:

```bash
cd /opt/yanapomnyu
ls -la .env                          # Основной .env файл
ls -la docker-compose.yml            # Docker Compose конфигурация
ls -la yanapomnyu_bot/src/           # Исходники бота
ls -la llm_api/cmd/                  # Исходники LLM API
```

### 4.2. Запуск всех сервисов

```bash
cd /opt/yanapomnyu
docker compose up -d --build
```

**Что происходит:**
1. Docker загружает базовые образы (MongoDB, Golang, Rust)
2. Компилирует LLM API (Go) — ~1-2 минуты
3. Компилирует Telegram Bot (Rust) — ~5-10 минут (первый раз)
4. Запускает все сервисы в фоновом режиме

### 4.2. Просмотр процесса сборки

Если хотите видеть процесс в реальном времени:

```bash
docker compose logs -f
```

Выход: `Ctrl+C` (это не остановит сервисы, только выйдет из просмотра логов)

### 4.3. Проверка статуса

```bash
docker compose ps
```

**Ожидаемый результат:** Все три контейнера в статусе `Up`:

```
NAME                STATUS
yanapomnyu-bot      Up X minutes
yanapomnyu-llm      Up X minutes
yanapomnyu-mongo    Up X minutes
```

### 4.4. Проверка логов бота

```bash
docker compose logs bot
```

Должны увидеть:
```
INFO Starting yanapomnyu_bot...
INFO Connecting to MongoDB...
INFO MongoDB connected
INFO Starting reminder scheduler...
INFO Starting Telegram bot dispatcher...
```

---

## Шаг 5. Восстановление базы (если есть дамп)

Если вы мигрируете со старой версии бота и у вас есть дамп MongoDB:

### 5.1. Загрузка дампа на сервер

С локального компьютера:

```bash
scp -r ./dump user@your-server-ip:/opt/yanapomnyu/dump
```

### 5.2. Восстановление данных

На сервере:

```bash
cd /opt/yanapomnyu

# Копируем дамп в контейнер MongoDB
docker cp ./dump yanapomnyu-mongo:/dump

# Восстанавливаем базу данных
docker exec -it yanapomnyu-mongo mongorestore \
  --username admin \
  --password ВАШ_ПАРОЛЬ_ИЗ_ENV \
  --authenticationDatabase admin \
  /dump

# Удаляем дамп с сервера (экономим место)
rm -rf ./dump
```

### 5.3. Проверка миграции

```bash
# Подключаемся к MongoDB
docker exec -it yanapomnyu-mongo mongosh -u admin -p ВАШ_ПАРОЛЬ --authenticationDatabase admin

# В mongosh выполните:
use tgBot
db.users.countDocuments()    # Количество пользователей
db.reminds.countDocuments()  # Количество напоминаний
exit
```

---

## Обслуживание и мониторинг

### Просмотр логов

```bash
# Все сервисы
docker compose logs -f

# Только бот
docker compose logs -f bot

# Последние 100 строк
docker compose logs --tail=100 bot
```

### Перезапуск сервисов

```bash
# Перезапуск всех сервисов
docker compose restart

# Перезапуск только бота
docker compose restart bot
```

### Остановка и запуск

```bash
# Остановка всех сервисов
docker compose stop

# Запуск всех сервисов
docker compose start

# Полная остановка и удаление контейнеров
docker compose down
```

### Обновление кода

```bash
# 1. На локальной машине: синхронизируйте новый код через rsync (см. Шаг 2)

# 2. На сервере: пересоберите и перезапустите
cd /opt/yanapomnyu
docker compose up -d --build
```

### Бэкап базы данных

```bash
# Создание дампа
docker exec yanapomnyu-mongo mongodump \
  --username admin \
  --password ВАШ_ПАРОЛЬ \
  --authenticationDatabase admin \
  --out=/dump

# Копирование дампа на локальную машину
docker cp yanapomnyu-mongo:/dump ./backup-$(date +%Y%m%d)

# Очистка дампа из контейнера
docker exec yanapomnyu-mongo rm -rf /dump
```

### Мониторинг ресурсов

```bash
# Использование ресурсов контейнерами
docker stats

# Использование диска
du -sh /opt/yanapomnyu/
docker system df
```

---

## Решение проблем

### Бот не запускается

**Проверка 1:** Посмотрите логи

```bash
docker compose logs bot
```

**Проблема:** `MONGO_URI must be set`
- **Решение:** Проверьте файл `.env`, убедитесь что все переменные заполнены

**Проблема:** `Failed to connect to MongoDB`
- **Решение:** Проверьте, что MongoDB запущен: `docker compose ps`
- Проверьте пароль в `.env`

### MongoDB не запускается

```bash
# Проверка логов
docker compose logs mongodb

# Если нужно пересоздать
docker compose down
docker volume rm yanapomnyu_mongo_data
docker compose up -d
```

### Бот не отвечает на сообщения

1. Проверьте, что токен правильный: `echo $BOT_TOKEN`
2. Убедитесь, что бот не запущен в другом месте (конфликт webhook)
3. Проверьте логи: `docker compose logs bot | grep ERROR`

### LLM API не отвечает

```bash
# Проверка статуса
docker compose logs llm_api

# Тест API напрямую
curl http://localhost:8080/health
```

### Нехватка места на диске

```bash
# Очистка неиспользуемых образов и контейнеров
docker system prune -a

# Очистка логов
truncate -s 0 $(docker inspect --format='{{.LogPath}}' yanapomnyu-bot)
```

### Высокое потребление CPU/RAM

```bash
# Проверка ресурсов
docker stats

# Если проблема в боте - перезапуск
docker compose restart bot
```

---

## Дополнительные настройки

### Настройка автозапуска

Docker Compose с `restart: always` уже настроен на автозапуск. Но убедитесь, что Docker сам запускается при загрузке системы:

```bash
sudo systemctl enable docker
```

### Настройка firewall (UFW)

```bash
# Разрешаем SSH
sudo ufw allow 22/tcp

# Разрешаем HTTP/HTTPS (если планируется webhook)
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp

# Включаем firewall
sudo ufw enable
```

### Настройка webhook для YooKassa

Если у вас есть домен и SSL:

1. Настройте nginx reverse proxy на `/yookassa/webhook` → `http://localhost:3001/yookassa/webhook`
2. В личном кабинете YooKassa укажите URL: `https://yourdomain.com/yookassa/webhook`

---

## Архитектура проекта

```
/opt/yanapomnyu/
├── docker-compose.yml          # Конфигурация Docker
├── .env                        # Основной файл с секретами (для Docker Compose)
├── yanapomnyu_bot/             # Rust Telegram Bot
│   ├── src/                    # Исходный код
│   ├── Cargo.toml              # Манифест Rust
│   └── Dockerfile              # Сборка бота
└── llm_api/                    # Go LLM Service
    ├── cmd/                    # Точка входа
    ├── internal/               # Бизнес-логика
    ├── config/                 # Конфигурация
    ├── .env                    # (Опционально) Переопределение настроек LLM API
    └── Dockerfile              # Сборка API
```

### Пояснение о .env файлах

**Основной .env** (`/opt/yanapomnyu/.env`):
- Используется Docker Compose для подстановки переменных типа `${MONGO_USER}`
- Содержит все основные настройки: MongoDB, Bot Token, YooKassa, OpenRouter API
- **ОБЯЗАТЕЛЬНЫЙ** файл

**LLM API .env** (`/opt/yanapomnyu/llm_api/.env`):
- **ОПЦИОНАЛЬНЫЙ** файл
- Используется Go приложением напрямую для переопределения настроек
- Если не создан, LLM API использует переменные окружения из docker-compose.yml
- Нужен только если хотите отдельно настроить LLM API без изменения docker-compose.yml

---

## Совместимость со старой версией (Go)

Проект полностью совместим со старой MongoDB схемой:

- ✅ Коллекция `users` — сохранены все поля (`utc`, `timezone`, `delay`, `autodelay`, etc.)
- ✅ Коллекция `reminds` — используется `remID` для совместимости
- ✅ Коллекция `records` — для управления подписками
- ✅ Счетчик `remID` — механизм инкремента сохранен

**Миграция:** Просто скопируйте дамп старой базы и восстановите (см. Шаг 5).

---

## Поддержка

- **Документация по архитектуре:** см. `ARCHITECTURE.md` в проекте
- **Исходный код:** `/home/hydro/RustProjects/telegram/yanapomnyu_bot/`
- **LLM API:** `/home/hydro/GoProjects/llm_api/`

---

**Разработано с ❤️ на Rust + Go**

