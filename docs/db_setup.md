## Быстрый запуск MongoDB для yanapomnyu_bot

### Что храним
- База `tgBot`.
- Коллекции:
  - `users`: `{ id: i64, utc: string, timezone: string, delay: [string], autodelay: string, morning: string, afternoon: string, evening: string, state: string, paymentInfo?: string }` (`paymentInfo` встречается редко).
  - `reminds`: `{ id: i64, text: string, delay: string (может быть "" для разовых), time: datetime, status: string, remID: int, messageID?: int, snoozeTime?: datetime }` + служебный документ-счетчик `{ number: 1, num: int }` для автоинкремента `remID`.
  - `records`: `{ id: i64, balance: int, isGroup: bool, groupName: string, nextPaymentDate: datetime, active: bool, freestate?: int }`
  - `transactions`: зеркалирует поля из legacy Go (transactionID, chat_id, суммы/статусы/charge_id и даты) — в текущем дампе отсутствует, но тип сохранён для совместимости.

### Поднять MongoDB через Docker
```bash
# 1) старт контейнера
docker run -d --name yanapomnyu-mongo \
  -p 27017:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=admin \
  -e MONGO_INITDB_ROOT_PASSWORD=admin \
  mongo:7.0

# 2) создать пользователя для бота (однократно)
docker exec -it yanapomnyu-mongo mongosh -u admin -p admin --authenticationDatabase admin <<'EOF'
use tgBot
db.createUser({user: "tgBotUser", pwd: "tgBotPassword", roles: [{role: "readWrite", db: "tgBot"}]})
EOF
```

### Строка подключения
- Локально: `mongodb://tgBotUser:tgBotPassword@localhost:27017/tgBot?authSource=tgBot`
- Добавь в `.env` (для будущего использования модулем БД):
  ```
  MONGO_URI=mongodb://tgBotUser:tgBotPassword@localhost:27017/tgBot?authSource=tgBot
  ```

### Проверка подключения
```bash
docker exec -it yanapomnyu-mongo mongosh "mongodb://tgBotUser:tgBotPassword@localhost:27017/tgBot?authSource=tgBot" --eval "db.users.countDocuments()"
```

### Что делает модуль `src/api/db.rs`
- Подключается к MongoDB и выставляет уникальные индексы на `users.id`, `records.id`, `reminds.remID`.
- Автоматически создаёт документ-счетчик `{number:1,num:1}` в `reminds`, если его нет.
- Даёт типизированные коллекции и базовые операции: поиск/создание пользователя, смена `timezone`/`utc`, автоинкремент `remID`, вставка напоминаний.

### Структура данных в репозитории
- Продовые данные/конфиг для локального подъёма Mongo лежат в корневой папке `data/` (`db_data/`, `log/`, `mongod.conf`). `legacy_go/` теперь используется только как референс кода и не нужен для запуска.
