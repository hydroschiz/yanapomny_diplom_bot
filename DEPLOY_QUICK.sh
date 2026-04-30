#!/bin/bash
# Быстрый скрипт развертывания YaPomnyu Bot
# Использование: ./DEPLOY_QUICK.sh user@your-server-ip

set -e

if [ -z "$1" ]; then
    echo "❌ Ошибка: не указан SSH хост"
    echo "Использование: ./DEPLOY_QUICK.sh user@your-server-ip"
    exit 1
fi

SSH_HOST="$1"
REMOTE_DIR="/opt/yanapomnyu"

echo "🚀 Начинаем развертывание на $SSH_HOST"
echo ""

# Создаем директорию на сервере
echo "📁 Создание директории на сервере..."
ssh "$SSH_HOST" "mkdir -p $REMOTE_DIR"

# Копируем Rust бот
echo "📦 Копирование Rust бота..."
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
  ./ "$SSH_HOST:$REMOTE_DIR/yanapomnyu_bot/"

# Копируем Go LLM API (относительный путь)
echo "📦 Копирование LLM API..."
if [ -d "../../../GoProjects/llm_api" ]; then
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
      ../../../GoProjects/llm_api/ "$SSH_HOST:$REMOTE_DIR/llm_api/"
else
    echo "⚠️  Директория llm_api не найдена, пропускаем..."
fi

# Копируем docker-compose конфигурацию
echo "📋 Копирование docker-compose.yml..."
scp docker-compose.prod.yml "$SSH_HOST:$REMOTE_DIR/docker-compose.yml"

echo ""
echo "✅ Файлы скопированы успешно!"
echo ""
echo "📝 Следующие шаги:"
echo "1. Подключитесь к серверу: ssh $SSH_HOST"
echo "2. Перейдите в директорию: cd $REMOTE_DIR"
echo "3. Создайте основной файл .env:"
echo "   nano .env"
echo "   (используйте yanapomnyu_bot/.env.example как шаблон)"
echo ""
echo "4. (Опционально) Создайте llm_api/.env если нужно переопределить настройки LLM API"
echo ""
echo "5. Запустите сервисы: docker compose up -d --build"
echo ""
echo "Для просмотра логов: docker compose logs -f"

