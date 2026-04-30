# 📝 Пояснение о .env файлах в проекте

В проекте используются **ДВА** `.env` файла с разным назначением. Это может вызвать путаницу, поэтому вот подробное объяснение.

## Структура .env файлов

```
/opt/yanapomnyu/
├── .env                          # ⭐ ОСНОВНОЙ - обязательный
└── llm_api/
    └── .env                      # 🔧 ДОПОЛНИТЕЛЬНЫЙ - опциональный
```

---

## 1. Основной .env (корневой)

**Путь:** `/opt/yanapomnyu/.env`

**Назначение:**
- Используется **Docker Compose** для подстановки переменных в `docker-compose.yml`
- Содержит все основные настройки проекта

**Статус:** ⭐ **ОБЯЗАТЕЛЬНЫЙ**

**Что внутри:**

```env
# MongoDB
MONGO_USER=admin
MONGO_PASS=your_password

# Telegram Bot
BOT_TOKEN=1234567890:ABC...

# Admins
ADMINS_ID=123456789

# YooKassa (платежи)
YK_SHOP_ID=123456
YK_SECRET_KEY=live_xxx

# OpenRouter API (LLM)
OPENAI_API_KEY=sk-or-v1-xxx
```

**Как используется:**

В `docker-compose.yml` есть подстановки:

```yaml
mongodb:
  environment:
    MONGO_INITDB_ROOT_USERNAME: ${MONGO_USER}    # ← Читается из .env
    MONGO_INITDB_ROOT_PASSWORD: ${MONGO_PASS}    # ← Читается из .env

bot:
  environment:
    TELOXIDE_TOKEN: ${BOT_TOKEN}                 # ← Читается из .env
    MONGO_URI: "mongodb://${MONGO_USER}:${MONGO_PASS}@..."  # ← Подстановка

llm_api:
  environment:
    OPENAI_API_KEY: ${OPENAI_API_KEY}            # ← Читается из .env
```

**Создание:**

```bash
cd /opt/yanapomnyu
nano .env
# Скопировать содержимое из yanapomnyu_bot/.env.example
```

---

## 2. LLM API .env (в подпапке)

**Путь:** `/opt/yanapomnyu/llm_api/.env`

**Назначение:**
- Используется **Go приложением** (LLM API) напрямую через библиотеку `cleanenv`
- Позволяет переопределить настройки LLM API без изменения docker-compose.yml

**Статус:** 🔧 **ОПЦИОНАЛЬНЫЙ**

**Что внутри:**

```env
# Переопределение настроек LLM API
HOST=0.0.0.0
PORT=8080
OPENAI_BASE_URL=https://openrouter.ai/api/v1/
OPENAI_API_KEY=sk-or-v1-xxx
OPENAI_MODEL=google/gemma-3-27b-it:free
```

**Как используется:**

В Go коде (`llm_api/config/config.go`):

```go
func Init() error {
    err := cleanenv.ReadEnv(&AppConfig)  // Читает из .env если он есть
    if err != nil {
        return err
    }
    return nil
}
```

**Когда нужен:**
- Если хотите использовать **свой** OpenRouter API key для LLM API
- Если хотите изменить модель (например, на `meta-llama/llama-3.1-8b-instruct:free`)
- Если нужно изменить порт или другие настройки LLM сервиса

**Когда НЕ нужен:**
- Если устраивают настройки по умолчанию из `docker-compose.yml`
- Если `OPENAI_API_KEY` уже указан в корневом `.env` и его достаточно

**Создание (если нужно):**

```bash
cd /opt/yanapomnyu/llm_api
nano .env
# Скопировать содержимое из .env.example
```

---

## Приоритет переменных

### Для LLM API сервиса:

1. **Переменные из `llm_api/.env`** (если файл существует)
2. **Переменные окружения из docker-compose.yml** (если нет локального .env)
3. **Значения по умолчанию в коде Go**

### Пример:

```yaml
# docker-compose.yml
llm_api:
  environment:
    OPENAI_API_KEY: ${OPENAI_API_KEY}  # Из корневого .env
    OPENAI_MODEL: "google/gemma-3-27b-it:free"
```

Если создать `llm_api/.env`:

```env
OPENAI_MODEL=meta-llama/llama-3.1-8b-instruct:free
```

То LLM API будет использовать **модель из локального .env**, но **API key из docker-compose.yml**.

---

## Рекомендуемая конфигурация

### Минимальная (рекомендуется для начала)

Создайте **только корневой .env**:

```bash
cd /opt/yanapomnyu
nano .env
```

Содержимое:

```env
MONGO_USER=admin
MONGO_PASS=сложный_пароль
BOT_TOKEN=ваш_токен
ADMINS_ID=123456789
YK_SHOP_ID=123456
YK_SECRET_KEY=live_xxx
# OPENAI_API_KEY оставьте закомментированным - будет использоваться default
```

**Не создавайте** `llm_api/.env` - LLM API будет работать с настройками по умолчанию.

### Расширенная (если нужен свой OpenRouter ключ)

1. Создайте корневой `.env` (как выше)
2. Добавьте в него:
   ```env
   OPENAI_API_KEY=sk-or-v1-ваш_ключ
   ```

**Не создавайте** `llm_api/.env` - ключ из корневого `.env` будет передан через docker-compose.

### Максимальная (полный контроль над LLM API)

1. Создайте корневой `.env` (основные настройки)
2. Создайте `llm_api/.env`:
   ```bash
   cd llm_api
   nano .env
   ```
   
   Содержимое:
   ```env
   HOST=0.0.0.0
   PORT=8080
   OPENAI_BASE_URL=https://openrouter.ai/api/v1/
   OPENAI_API_KEY=sk-or-v1-отдельный_ключ_для_llm
   OPENAI_MODEL=meta-llama/llama-3.1-8b-instruct:free
   ```

---

## Частые вопросы

### Q: Нужно ли создавать оба .env файла?

**A:** Нет! Достаточно создать **только корневой** `/opt/yanapomnyu/.env`. Второй файл опциональный.

### Q: Что будет, если не создать `llm_api/.env`?

**A:** Ничего страшного! LLM API будет использовать переменные из `docker-compose.yml`, которые берутся из корневого `.env`.

### Q: Можно ли использовать разные OpenRouter ключи для бота и LLM API?

**A:** Да, но бот не использует OpenRouter напрямую. Только LLM API использует. Так что одного ключа достаточно.

### Q: Какой .env имеет приоритет?

**A:** 
- Для **Docker Compose** - только корневой `.env`
- Для **LLM API** - локальный `llm_api/.env` имеет приоритет над переменными из docker-compose

### Q: Где хранить .env файлы в Git?

**A:** **НИГДЕ!** Оба `.env` файла должны быть в `.gitignore`. Храните только `.env.example` файлы.

---

## Безопасность

### Права доступа

```bash
chmod 600 /opt/yanapomnyu/.env
chmod 600 /opt/yanapomnyu/llm_api/.env  # Если создали
```

### Проверка что .env не в Git

```bash
cd /home/hydro/RustProjects/telegram/yanapomnyu_bot
git status  # .env не должен быть в списке
cat .gitignore | grep .env  # Должно показать исключение
```

---

## Быстрый старт (TL;DR)

```bash
# На сервере
cd /opt/yanapomnyu

# 1. Создайте ОДИН файл .env в корне
nano .env

# 2. Вставьте настройки из yanapomnyu_bot/.env.example

# 3. НЕ создавайте llm_api/.env (не нужен для начала)

# 4. Запустите
docker compose up -d --build
```

**Всё!** Оба сервиса (бот и LLM API) будут работать с настройками из одного корневого `.env`.

---

## Итого

| Файл | Обязательный? | Для чего |
|------|---------------|----------|
| `/opt/yanapomnyu/.env` | ⭐ **ДА** | Docker Compose, основные настройки |
| `/opt/yanapomnyu/llm_api/.env` | 🔧 Нет | Тонкая настройка LLM API (опционально) |

**Рекомендация:** Создайте только корневой `.env` и не усложняйте!


