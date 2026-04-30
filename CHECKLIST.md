# ✅ Чек-лист перед развертыванием

Используйте этот чек-лист для проверки готовности проекта к production развертыванию.

## 🔒 Безопасность

- [ ] Создан файл `.env` на сервере на основе `.env.example`
- [ ] Все токены и пароли заменены на реальные production значения
- [ ] Файл `.env` имеет права доступа `600` (`chmod 600 .env`)
- [ ] `.env` файл добавлен в `.gitignore` (уже сделано ✅)
- [ ] Проверено, что `.env` НЕ закоммичен в Git (`git status`)
- [ ] MongoDB пароль сложный (минимум 16 символов, буквы+цифры+спецсимволы)
- [ ] YooKassa credentials реальные (не `test`)
- [ ] SSH ключи настроены для беспарольного доступа (опционально)

## 🖥️ Сервер

- [ ] Ubuntu 22.04+ или Debian 11+ установлен
- [ ] Docker и Docker Compose установлены
- [ ] Минимум 2GB RAM (рекомендуется 4GB)
- [ ] Минимум 10GB свободного места
- [ ] Порты 27017 (MongoDB), 6379 (Redis), 3001 (HTTP) доступны
- [ ] Firewall настроен (UFW): разрешены SSH, HTTP/HTTPS
- [ ] Создана директория `/opt/yanapomnyu`

## 📦 Копирование файлов

- [ ] rsync установлен на локальной машине
- [ ] SSH доступ к серверу работает (`ssh user@server`)
- [ ] Выполнен `./DEPLOY_QUICK.sh user@server` ИЛИ
- [ ] Выполнены команды rsync вручную для:
  - [ ] Rust бота (`yanapomnyu_bot/`)
  - [ ] Go LLM API (`llm_api/`)
  - [ ] docker-compose.yml (из `docker-compose.prod.yml`)
- [ ] На сервере проверено наличие файлов:
  ```bash
  ls -la /opt/yanapomnyu/yanapomnyu_bot/src/
  ls -la /opt/yanapomnyu/llm_api/cmd/
  ```

## ⚙️ Конфигурация

### Основной .env файл

- [ ] Файл `.env` создан в корне `/opt/yanapomnyu/`
- [ ] Переменная `BOT_TOKEN` заполнена (от @BotFather)
- [ ] Переменные `MONGO_USER` и `MONGO_PASS` заполнены
- [ ] Переменная `ADMINS_ID` содержит ваши Telegram ID
- [ ] YooKassa настроена (`YK_SHOP_ID`, `YK_SECRET_KEY`) если используется
- [ ] `OPENAI_API_KEY` настроен если нужен свой ключ (опционально)
- [ ] Проверен файл `docker-compose.yml`:
  ```bash
  cat /opt/yanapomnyu/docker-compose.yml
  ```

### LLM API .env (опционально)

- [ ] Если нужно переопределить настройки LLM API - создан `/opt/yanapomnyu/llm_api/.env`
- [ ] Или пропущен этот шаг (LLM API будет использовать переменные из docker-compose.yml)

## 🚀 Запуск

- [ ] Выполнен первый запуск: `docker compose up -d --build`
- [ ] Дождались завершения сборки (~10-15 минут для первого раза)
- [ ] Проверен статус контейнеров: `docker compose ps`
- [ ] Все три контейнера в статусе `Up`:
  - [ ] `yanapomnyu-mongo`
  - [ ] `yanapomnyu-llm`
  - [ ] `yanapomnyu-bot`
- [ ] Проверены логи бота: `docker compose logs bot | grep "INFO"`
- [ ] В логах нет ERROR сообщений
- [ ] Бот отвечает в Telegram на команду `/start`

## 🗄️ База данных

Если мигрируете со старой версии:

- [ ] Дамп старой БД скопирован на сервер
- [ ] Выполнен `mongorestore`
- [ ] Проверено количество пользователей: `db.users.countDocuments()`
- [ ] Проверено количество напоминаний: `db.reminds.countDocuments()`
- [ ] Дамп удален с сервера для экономии места

Если новая установка:

- [ ] MongoDB запустился без ошибок
- [ ] Индексы созданы автоматически при первом запуске
- [ ] Создан тестовый пользователь через бота `/start`

## 🧪 Тестирование

- [ ] Отправлено `/start` боту — получен welcome message
- [ ] Создано тестовое напоминание
- [ ] Проверена команда `/list` — напоминание отображается
- [ ] Удалено тестовое напоминание
- [ ] Проверены настройки через `/setup`
- [ ] Проверен профиль через `/profile`
- [ ] Проверены логи: `docker compose logs -f bot`

## 💳 Платежи (если используется)

- [ ] YooKassa аккаунт создан и настроен
- [ ] Shop ID и Secret Key внесены в `.env`
- [ ] Webhook URL настроен в ЮKassa: `https://yourdomain.com/yookassa/webhook`
- [ ] Nginx/Caddy reverse proxy настроен (если используется домен)
- [ ] SSL сертификат установлен (Let's Encrypt)
- [ ] Тестовый платеж проведен успешно
- [ ] Подписка активировалась после тестового платежа

## 📊 Мониторинг

- [ ] Настроен автозапуск Docker: `sudo systemctl enable docker`
- [ ] Настроен мониторинг ресурсов: `docker stats`
- [ ] Настроены ротация логов (встроена в docker-compose)
- [ ] Настроен cron для регулярных бэкапов БД (опционально)
- [ ] Настроены алерты при падении контейнеров (опционально)

## 📚 Документация

- [ ] Команда знает, где найти документацию:
  - README.md — общее описание
  - DEPLOYMENT.md — развертывание
  - ARCHITECTURE.md — архитектура
- [ ] Известно, как смотреть логи: `docker compose logs -f`
- [ ] Известно, как перезапускать: `docker compose restart`
- [ ] Известно, как обновлять код: см. DEPLOYMENT.md
- [ ] Известно, как делать бэкапы: см. DEPLOYMENT.md

## 🔧 Обслуживание

Настроить регулярные задачи:

- [ ] **Еженедельно**: Проверка логов на ошибки
- [ ] **Еженедельно**: Проверка свободного места: `df -h`
- [ ] **Еженедельно**: Проверка статуса контейнеров: `docker compose ps`
- [ ] **Ежемесячно**: Бэкап базы данных MongoDB
- [ ] **Ежемесячно**: Очистка старых Docker образов: `docker system prune -a`
- [ ] **По необходимости**: Обновление зависимостей (Rust, Go)
- [ ] **По необходимости**: Обновление кода через rsync

## 🐛 Решение проблем

Знаю, как решать типичные проблемы:

- [ ] Бот не запускается → проверить логи
- [ ] MongoDB не подключается → проверить пароль в .env
- [ ] Нехватка места → очистить `docker system prune`
- [ ] Высокое CPU/RAM → перезапустить контейнеры
- [ ] Бот не отвечает → проверить TELOXIDE_TOKEN

См. раздел "Решение проблем" в DEPLOYMENT.md

## ✅ Финальная проверка

Убедитесь, что всё работает:

```bash
# На сервере
cd /opt/yanapomnyu

# Все контейнеры запущены
docker compose ps

# Логи без ошибок
docker compose logs bot | tail -50

# Бот отвечает в Telegram
# Отправьте: /start, /help, /profile
```

---

## 🎉 Готово к production!

Если все пункты отмечены ✅ — проект готов к работе!

**Полезные команды для работы:**

```bash
# Просмотр логов
docker compose logs -f bot

# Перезапуск
docker compose restart

# Обновление кода
./DEPLOY_QUICK.sh user@server
docker compose up -d --build

# Бэкап БД
docker exec yanapomnyu-mongo mongodump \
  --username admin --password PASSWORD \
  --authenticationDatabase admin --out=/dump
docker cp yanapomnyu-mongo:/dump ./backup-$(date +%Y%m%d)

# Мониторинг ресурсов
docker stats
```

---

**Поддержка:**
- 📖 Документация: см. README.md, DEPLOYMENT.md
- 🐛 Issues: создайте issue в репозитории
- 💬 Вопросы: свяжитесь с администраторами

