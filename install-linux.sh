#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_HOST="${LAUNDRYKU_APP_HOST:-127.0.0.1}"
APP_PORT="${LAUNDRYKU_APP_PORT:-8000}"
ADMIN_EMAIL="${LAUNDRYKU_ADMIN_EMAIL:-test@example.com}"
ADMIN_PASSWORD="${LAUNDRYKU_ADMIN_PASSWORD:-password}"
DB_DIR="$ROOT_DIR/.mariadb-data"
DB_SOCKET="$ROOT_DIR/.mariadb.sock"
DB_PID="$ROOT_DIR/.mariadb.pid"
DB_PORT="${LAUNDRYKU_DB_PORT:-3308}"

cd "$ROOT_DIR"

need_cmd() {
  command -v "$1" >/dev/null 2>&1
}

sudo_cmd() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  else
    sudo "$@"
  fi
}

install_linux_dependencies() {
  local has_database=false

  if need_cmd php; then
    if php_module_enabled pdo_sqlite || { php_module_enabled pdo_mysql && need_cmd mariadbd && need_cmd mariadb-install-db && need_cmd mariadb-admin && need_cmd mariadb; }; then
      has_database=true
    fi
  fi

  if need_cmd php && need_cmd composer && need_cmd node && need_cmd npm && [[ "$has_database" == true ]]; then
    return
  fi

  echo "Installing required packages..."

  if need_cmd apt-get; then
    sudo_cmd apt-get update
    sudo_cmd apt-get install -y php-cli php-sqlite3 php-mysql php-mbstring php-xml php-curl php-zip php-bcmath php-intl mariadb-server unzip curl git composer nodejs npm
  elif need_cmd pacman; then
    sudo_cmd pacman -Sy --needed --noconfirm php php-sqlite mariadb composer nodejs npm unzip curl git
  elif need_cmd dnf; then
    sudo_cmd dnf install -y php-cli php-pdo php-mysqlnd php-sqlite3 php-mbstring php-xml php-curl php-zip php-bcmath php-intl mariadb-server unzip curl git composer nodejs npm
  elif need_cmd zypper; then
    sudo_cmd zypper --non-interactive install php php-sqlite3 php-mysql php-mbstring php-xmlreader php-curl php-zip php-bcmath php-intl mariadb unzip curl git composer nodejs npm
  else
    echo "Unsupported Linux package manager. Install PHP 8.3+, Composer, Node.js, and npm, then rerun this script."
    exit 1
  fi
}

php_module_enabled() {
  php -m | grep -qi "^${1}$"
}

start_mariadb() {
  if [[ ! -d "$DB_DIR/mysql" ]]; then
    mariadb-install-db --no-defaults --datadir="$DB_DIR" --auth-root-authentication-method=normal
  fi

  if ! mariadb-admin --protocol=tcp -h127.0.0.1 -P"$DB_PORT" -uroot ping --silent >/dev/null 2>&1; then
    mariadbd --no-defaults \
      --datadir="$DB_DIR" \
      --socket="$DB_SOCKET" \
      --pid-file="$DB_PID" \
      --port="$DB_PORT" \
      --bind-address=127.0.0.1 &

    for _ in {1..40}; do
      if mariadb-admin --protocol=tcp -h127.0.0.1 -P"$DB_PORT" -uroot ping --silent >/dev/null 2>&1; then
        break
      fi
      sleep 0.25
    done
  fi

  mariadb --protocol=tcp -h127.0.0.1 -P"$DB_PORT" -uroot -e \
    'CREATE DATABASE IF NOT EXISTS laundryku CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;'
}

env_set() {
  local key="$1"
  local value="$2"

  if grep -q "^${key}=" .env; then
    sed -i "s|^${key}=.*|${key}=${value}|" .env
  else
    printf '%s=%s\n' "$key" "$value" >> .env
  fi
}

install_linux_dependencies

if [[ ! -f .env ]]; then
  cp .env.example .env
fi

env_set APP_ENV local
env_set APP_DEBUG true
env_set APP_URL "http://${APP_HOST}:${APP_PORT}"
env_set SESSION_DRIVER file
env_set QUEUE_CONNECTION sync
env_set CACHE_STORE file
env_set LAUNDRYKU_ADMIN_EMAIL "$ADMIN_EMAIL"
env_set LAUNDRYKU_ADMIN_PASSWORD "$ADMIN_PASSWORD"

if php_module_enabled pdo_sqlite; then
  mkdir -p database
  touch database/database.sqlite
  env_set DB_CONNECTION sqlite
  env_set DB_DATABASE "$ROOT_DIR/database/database.sqlite"
elif php_module_enabled pdo_mysql && need_cmd mariadbd && need_cmd mariadb-install-db && need_cmd mariadb-admin && need_cmd mariadb; then
  start_mariadb
  env_set DB_CONNECTION mysql
  env_set DB_HOST 127.0.0.1
  env_set DB_PORT "$DB_PORT"
  env_set DB_DATABASE laundryku
  env_set DB_USERNAME root
  env_set DB_PASSWORD ""
else
  echo "No usable local database driver found."
  echo "Enable PHP pdo_sqlite, or install MariaDB plus PHP pdo_mysql, then rerun this script."
  exit 1
fi

composer install --no-interaction --prefer-dist --optimize-autoloader
npm ci

if ! grep -q '^APP_KEY=base64:' .env; then
  php artisan key:generate --force
fi

php artisan config:clear
php artisan migrate --force
php artisan db:seed --force
npm run build

echo
echo "Laundryku is ready."
echo "URL: http://${APP_HOST}:${APP_PORT}/admin"
echo "Email: ${ADMIN_EMAIL}"
echo "Password: ${ADMIN_PASSWORD}"
echo
php artisan serve --host="$APP_HOST" --port="$APP_PORT"
