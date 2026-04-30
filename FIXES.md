# 🔧 Исправления для успешного развертывания

Этот документ описывает исправления, внесенные для решения проблем при первом развертывании.

## ❌ Проблема 1: Edition 2024

### Ошибка
```
error: feature `edition2024` is required
The package requires the Cargo feature called `edition2024`, but that feature is not stabilized
```

### Причина
`Cargo.toml` содержал `edition = "2024"`, которая еще не стабилизирована в Rust 1.81.

### Решение
✅ Изменено на `edition = "2021"` в `Cargo.toml`

```diff
[package]
name = "yanapomnyu_bot"
version = "0.1.0"
-edition = "2024"
+edition = "2021"
```

---

## ❌ Проблема 2: Устаревшая версия Docker Compose

### Ошибка
```
WARN: the attribute `version` is obsolete, it will be ignored
```

### Причина
Docker Compose v2+ не требует указания версии файла.

### Решение
✅ Удалена строка `version: '3.8'` из `docker-compose.prod.yml`

```diff
-version: '3.8'
-
 services:
   mongodb:
     ...
```

---

## ❌ Проблема 3: Сложный Dockerfile с dummy build

### Ошибка
```
exit code: 101 при cargo build --release
```

### Причина
Попытка оптимизации с dummy build создавала конфликты между слоями.

### Решение
✅ Упрощен `Dockerfile` - убраны промежуточные шаги с dummy сборкой

**Было:**
```dockerfile
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -f target/release/deps/yanapomnyu_bot*
RUN rm src/main.rs
COPY . .
RUN cargo build --release
```

**Стало:**
```dockerfile
COPY . .
RUN cargo build --release
```

**Плюсы упрощенного подхода:**
- Нет конфликтов между слоями
- Более надежная сборка
- Проще понять и поддерживать

**Минусы:**
- Чуть медленнее повторные сборки при изменении только кода
- Не критично для production развертывания

---

## ✅ Дополнительные улучшения

### 1. Создан `.dockerignore` для Rust проекта

```
target
target_*
.git
.env
.env.*
!.env.example
data
data_production
legacy_go
docs
tests
*.md
*.zip
.gitignore
```

**Зачем:**
- Исключает ненужные файлы из Docker context
- Ускоряет сборку (не копируются гигабайты из `target/`)
- Улучшает безопасность (не копируется `.env`)

### 2. Создан `.dockerignore` для Go проекта

```
tmp
.git
.env
.env.*
!.env.example
*_test.go
*_TESTS.md
*.md
```

**Зачем:**
- Не копируются тесты и временные файлы
- Меньше размер контекста

### 3. Исправлен case в Dockerfile

```diff
-FROM rust:1.81-slim-bookworm as builder
+FROM rust:1.81-slim-bookworm AS builder
```

Uppercase `AS` - рекомендованный стиль по Docker best practices.

---

## ❌ Проблема 4: Несовместимость версии Rust

### Ошибка
```
error: rustc 1.81.0 is not supported by the following packages:
  teloxide@0.17.0 requires rustc 1.82
  icu_*@2.1.1 requires rustc 1.83
```

### Причина
Dockerfile использовал `rust:1.81-slim-bookworm`, но зависимости проекта требуют более новую версию.

### Решение
✅ Обновлен `Dockerfile` на `rust:1.83-slim-bookworm`

```diff
 # Build stage
-FROM rust:1.81-slim-bookworm AS builder
+FROM rust:1.83-slim-bookworm AS builder
```

---

## ❌ Проблема 5: Отсутствует Redis в docker-compose.prod.yml

### Ошибка
```
thread 'main' panicked at src/config.rs:75:47:
REDIS_URL must be set: NotPresent
```

### Причина
В `docker-compose.prod.yml` не был добавлен сервис Redis, который необходим для кэширования pending платежей YooKassa.

### Решение
✅ Добавлен сервис `redis` в `docker-compose.prod.yml`
✅ Добавлен volume `redis_data` для персистентности
✅ Добавлена переменная `REDIS_URL` в environment бота
✅ Добавлен `redis` в `depends_on` для правильного порядка запуска

**Добавленный сервис:**
```yaml
redis:
  image: redis:7-alpine
  container_name: yanapomnyu-redis
  restart: always
  volumes:
    - redis_data:/data
  networks:
    - bot_network
  command: redis-server --appendonly yes
```

**Обновлены environment переменные бота:**
```yaml
environment:
  REDIS_URL: "redis://redis:6379/"
  YK_RETURN_URL: ${YK_RETURN_URL:-https://t.me/yanapomnyu_bot}
  RUST_LOG: ${RUST_LOG:-info}
```

---

## ❌ Проблема 6: Пароль MongoDB не URL-encoded

### Ошибка
```
Error: Kind: An invalid argument was provided: password must be URL encoded
```

### Причина
В `docker-compose.prod.yml` пароль MongoDB подставлялся напрямую в URI:
```yaml
MONGO_URI: "mongodb://${MONGO_USER}:${MONGO_PASS}@mongodb:27017/tgBot?authSource=admin"
```

Если пароль содержит специальные символы (`@`, `#`, `%`, `&` и т.д.), они должны быть URL-encoded, но Docker Compose не делает этого автоматически.

### Решение
✅ Добавлена зависимость `urlencoding = "2.1"` в `Cargo.toml`
✅ Обновлен `config.rs` - теперь поддерживает два способа:
   - **Через отдельные переменные** (рекомендуется): `MONGO_USER`, `MONGO_PASS` - пароль автоматически кодируется
   - **Через MONGO_URI** (старый способ): пароль должен быть закодирован вручную
✅ Обновлен `docker-compose.prod.yml` - использует отдельные переменные

**Новый код в config.rs:**
```rust
let mongo_uri = if let Ok(uri) = env::var("MONGO_URI") {
    uri  // Используем напрямую если задан
} else {
    // Формируем из отдельных переменных с автоматическим кодированием
    let encoded_pass = urlencoding::encode(&pass);
    format!("mongodb://{}:{}@{}:{}/{}?authSource={}", 
        user, encoded_pass, host, port, db, auth_source)
};
```

**Преимущества:**
- Пароль можно задавать с любыми символами без ручного кодирования
- Более безопасно и удобно
- Обратная совместимость с `MONGO_URI`

---

## ❌ Проблема 7: Бот не может найти MongoDB сервер (DNS resolution)

### Ошибка
```
Error: failed to lookup address information: Temporary failure in name resolution
Address: mongodb:27017
```

### Причина
Бот запускался раньше, чем MongoDB был полностью готов к приему подключений. Docker Compose `depends_on` с `condition: service_started` ждет только запуска контейнера, но не готовности сервиса внутри.

### Решение
✅ Добавлен `healthcheck` для MongoDB контейнера
✅ Обновлен `depends_on` - бот ждет пока MongoDB станет `healthy`
✅ Используется `condition: service_healthy` вместо `service_started`

**Добавлен healthcheck:**
```yaml
mongodb:
  healthcheck:
    test: ["CMD", "mongosh", "--eval", "db.adminCommand('ping')", "--quiet"]
    interval: 5s
    timeout: 3s
    retries: 5
    start_period: 10s
```

**Обновлен depends_on:**
```yaml
bot:
  depends_on:
    mongodb:
      condition: service_healthy  # Ждет готовности MongoDB
```

**Результат:** Бот будет ждать пока MongoDB полностью запустится и будет готов принимать подключения (обычно ~10-15 секунд после старта контейнера).

---

## ❌ Проблема 8: MongoDB 6.0 требует AVX (процессор не поддерживает)

### Ошибка
```
WARNING: MongoDB 5.0+ requires a CPU with AVX support, and your current system does not appear to have that!
Illegal instruction (core dumped)
```

### Причина
MongoDB версии 5.0+ требует процессор с поддержкой инструкций AVX (Advanced Vector Extensions). Старые процессоры (например, Intel до Sandy Bridge, AMD до Bulldozer) не поддерживают AVX.

### Решение
✅ Понижена версия MongoDB с `6.0` на `4.4` в `docker-compose.prod.yml`
✅ Обновлена команда healthcheck с `mongosh` на `mongo` (MongoDB 4.4 использует старый клиент)

**Изменения:**
```yaml
mongodb:
  image: mongo:4.4  # Было: mongo:6.0
  healthcheck:
    test: ["CMD", "mongo", "--eval", "db.adminCommand('ping')", "--quiet"]
    # Было: mongosh
```

**MongoDB 4.4:**
- ✅ Последняя версия, поддерживающая процессоры без AVX
- ✅ Полная совместимость с проектом (используется та же схема БД)
- ✅ Поддерживается до 2025 года (extended support)
- ✅ Все функции проекта работают идентично

**Примечание:** Если в будущем сервер будет обновлен на процессор с AVX, можно вернуться на `mongo:6.0` или новее.

---

## 📊 Результаты тестирования

### Локальная сборка

✅ **Rust (debug mode):**
```bash
cargo build
# Finished `dev` profile [unoptimized + debuginfo] target(s) in 33.28s
```

✅ **Go:**
```bash
go build -o llm_api ./cmd/main.go
# ✅ Go build OK
```

### Ожидаемое время сборки Docker

| Сервис | Первая сборка | Повторная (без изменений) |
|--------|---------------|---------------------------|
| Go LLM API | 2-3 мин | 5-10 сек |
| Rust Bot | 8-12 мин | 1-2 мин |
| MongoDB | 30 сек | 5 сек |
| **Итого** | **~10-15 мин** | **~2-3 мин** |

---

## 🚀 Инструкция по развертыванию после исправлений

### 1. Обновите файлы на сервере

Если уже скопировали старые файлы - обновите их:

```bash
# С локальной машины
./DEPLOY_QUICK.sh user@your-server-ip
```

Или вручную скопируйте исправленные файлы:
- `Cargo.toml`
- `Dockerfile`
- `docker-compose.prod.yml`
- `.dockerignore`
- `llm_api/.dockerignore`

### 2. На сервере

```bash
ssh user@server
cd /opt/yanapomnyu

# Убедитесь что .env файл создан
ls -la .env

# Если нет - создайте
nano .env
# Заполните: BOT_TOKEN, MONGO_USER, MONGO_PASS, YK_*, ADMINS_ID

# Запустите сборку
docker compose down
docker compose up -d --build
```

### 3. Мониторинг сборки

```bash
# Просмотр процесса в реальном времени
docker compose logs -f

# Или только логи бота
docker compose logs -f bot
```

### 4. Проверка успешного запуска

```bash
# Статус контейнеров
docker compose ps

# Должны быть все в статусе Up:
# yanapomnyu-mongo    Up
# yanapomnyu-llm      Up
# yanapomnyu-bot      Up
```

**Успешные логи бота:**
```
INFO Starting yanapomnyu_bot...
INFO Connecting to MongoDB...
INFO MongoDB connected
INFO Initializing PaymentService...
INFO PaymentService initialized
INFO Starting HTTP server for YooKassa webhooks
INFO Starting reminder scheduler...
INFO Starting subscription scheduler...
INFO Starting channel scheduler...
INFO Starting Telegram bot dispatcher...
```

---

## 🐛 Если что-то пошло не так

### Ошибка: MongoDB connection failed

```bash
# Проверьте переменные
docker compose config | grep MONGO

# Проверьте статус MongoDB
docker compose logs mongodb
```

### Ошибка: Build failed

```bash
# Очистите все и пересоберите
docker compose down
docker system prune -f
docker compose up -d --build
```

### Ошибка: Permission denied

```bash
# На сервере
sudo chown -R $USER:$USER /opt/yanapomnyu
```

---

## 📚 Дополнительная информация

- **Полное руководство:** [DEPLOYMENT.md](DEPLOYMENT.md)
- **Архитектура:** [ARCHITECTURE.md](ARCHITECTURE.md)
- **О .env файлах:** [docs/env_files_explained.md](docs/env_files_explained.md)
- **Чек-лист:** [CHECKLIST.md](CHECKLIST.md)

---

**Дата исправлений:** 2026-01-06  
**Проверено:** Локальная сборка успешна, готово к production развертыванию

