# 📦 Руководство по rsync для развертывания

## Базовая команда rsync

```bash
rsync [OPTIONS] SOURCE DESTINATION
```

## Опции используемые в проекте

| Опция | Описание |
|-------|----------|
| `-a` | Archive mode (сохраняет права, время, символические ссылки) |
| `-v` | Verbose (показывает процесс копирования) |
| `-z` | Compress (сжимает данные при передаче, экономит трафик) |
| `--delete` | Удаляет файлы на сервере, которых нет локально (полная синхронизация) |
| `--include='pattern'` | Включить файлы/папки по маске |
| `--exclude='pattern'` | Исключить файлы/папки по маске |

## Правила работы include/exclude

⚠️ **ВАЖНО**: Порядок `--include` и `--exclude` имеет значение!

1. `--include='src/***'` — включить всю папку `src/` рекурсивно
2. `--exclude='target'` — исключить папку `target`

Паттерн `***` означает "все файлы и подпапки рекурсивно".

## Команды для проекта

### Rust Bot (yanapomnyu_bot)

```bash
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
  yanapomnyu_bot/ user@server:/opt/yanapomnyu/yanapomnyu_bot/
```

**Что копируется:**
- ✅ `src/` — весь исходный код
- ✅ `Cargo.toml`, `Cargo.lock` — манифест проекта
- ✅ `Dockerfile` — для сборки
- ✅ `docker-compose.prod.yml` — production конфигурация

**Что НЕ копируется:**
- ❌ `target/` — скомпилированные бинарники (экономия ~1-2GB)
- ❌ `legacy_go/` — старый код
- ❌ `docs/` — документация
- ❌ `data/` — локальные базы данных (экономия ~500MB+)
- ❌ `.env` — секреты (безопасность!)

### Go LLM API

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
  llm_api/ user@server:/opt/yanapomnyu/llm_api/
```

**Что копируется:**
- ✅ `cmd/`, `internal/`, `pkg/`, `config/` — исходный код
- ✅ `go.mod`, `go.sum` — зависимости
- ✅ `Dockerfile` — для сборки

**Что НЕ копируется:**
- ❌ `tmp/` — временные файлы
- ❌ `*_test.go` — тесты
- ❌ `*_TESTS.md` — документация тестов

## Оценка экономии трафика

| Проект | Полная папка | С rsync фильтрацией | Экономия |
|--------|--------------|---------------------|----------|
| Rust Bot | ~2.5GB | ~50MB | **98%** |
| Go API | ~100MB | ~5MB | **95%** |

## Dry-run режим (проверка без копирования)

Чтобы увидеть, что будет скопировано, без реального копирования:

```bash
rsync -avzn --delete \
  --include='src/***' \
  --exclude='target' \
  yanapomnyu_bot/ user@server:/opt/yanapomnyu/yanapomnyu_bot/
```

Опция `-n` (или `--dry-run`) показывает список файлов без копирования.

## Примеры использования

### 1. Первое развертывание

```bash
# Создать папку на сервере
ssh user@server "mkdir -p /opt/yanapomnyu"

# Копировать проекты
./DEPLOY_QUICK.sh user@server
```

### 2. Обновление кода

После внесения изменений локально:

```bash
# Синхронизировать только измененные файлы
rsync -avz --delete \
  --include='src/***' \
  --include='Cargo.toml' \
  --include='Cargo.lock' \
  --exclude='target' \
  --exclude='legacy_go' \
  yanapomnyu_bot/ user@server:/opt/yanapomnyu/yanapomnyu_bot/

# Пересобрать на сервере
ssh user@server "cd /opt/yanapomnyu && docker compose up -d --build bot"
```

### 3. Копирование только одного файла

```bash
scp yanapomnyu_bot/docker-compose.prod.yml user@server:/opt/yanapomnyu/docker-compose.yml
```

## Устранение проблем

### Ошибка: Permission denied

```bash
# Решение 1: Проверьте SSH ключ
ssh user@server

# Решение 2: Создайте папку с правами
ssh user@server "sudo mkdir -p /opt/yanapomnyu && sudo chown $USER /opt/yanapomnyu"
```

### Ошибка: Connection refused

```bash
# Проверьте SSH подключение
ssh -v user@server

# Проверьте порт (если нестандартный)
rsync -avz -e "ssh -p 2222" ...
```

### Копирование слишком медленное

```bash
# Используйте больше сжатия (9 = максимальное)
rsync -avz --compress-level=9 ...

# Или отключите сжатие если у вас быстрый канал
rsync -av ...
```

## Альтернативы rsync

### Git (для открытых проектов)

```bash
# На сервере
git clone https://github.com/user/repo.git
git pull  # обновление
```

**Плюсы:** Простота, версионирование  
**Минусы:** Копирует всю историю, сложнее настроить фильтрацию

### scp (для разовых файлов)

```bash
scp file.txt user@server:/path/
scp -r folder/ user@server:/path/
```

**Плюсы:** Встроен везде, простой  
**Минусы:** Нет инкрементальной синхронизации, копирует всё каждый раз

### Docker context (experimental)

```bash
docker context create remote --docker "host=ssh://user@server"
docker --context remote compose up -d
```

**Плюсы:** Нативная интеграция с Docker  
**Минусы:** Экспериментальная функция, требует настройки

## Рекомендации

1. **Всегда используйте `--dry-run`** перед первым запуском на новый сервер
2. **Проверяйте `.gitignore`** — он должен исключать `.env` файлы
3. **Используйте SSH ключи** вместо паролей для автоматизации
4. **Делайте бэкапы** перед использованием `--delete`
5. **Проверяйте размер передачи** через `rsync -avzn` перед реальным копированием

## Полезные ссылки

- [rsync man page](https://linux.die.net/man/1/rsync)
- [rsync examples](https://www.tecmint.com/rsync-local-remote-file-synchronization-commands/)
- [SSH key setup](https://www.digitalocean.com/community/tutorials/how-to-set-up-ssh-keys-on-ubuntu-20-04)


