#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
data_dir=${BETA_LOCAL_DATA_DIR:-"$rust_root/target/beta-postgres"}
socket_dir=${BETA_LOCAL_SOCKET_DIR:-/tmp/tgbot-beta-socket}
pg_bin_dir=${PG_BIN_DIR:-/usr/lib/postgresql/16/bin}
pg_port=${BETA_LOCAL_PG_PORT:-15432}
database_name=${BETA_LOCAL_DATABASE_NAME:-tgbot_rust_beta}
app_port=${BETA_LOCAL_APP_PORT:-18090}
database_url="postgresql://ubuntu@127.0.0.1:${pg_port}/${database_name}"
admin_url="postgresql://beta_pg_admin@127.0.0.1:${pg_port}/postgres"
manifest_path="$script_directory/beta-migrations.list"
binary_path="$rust_root/target/release/tgbot-app"

pg_ctl_bin="$pg_bin_dir/pg_ctl"
initdb_bin="$pg_bin_dir/initdb"
createdb_bin="$pg_bin_dir/createdb"
psql_bin="$pg_bin_dir/psql"
pg_isready_bin="$pg_bin_dir/pg_isready"

require_tools() {
  for tool in awk chmod curl env grep mkdir openssl sha256sum ss; do
    command -v "$tool" >/dev/null || {
      printf 'Beta local app requires command: %s\n' "$tool" >&2
      exit 1
    }
  done
  for tool in "$pg_ctl_bin" "$initdb_bin" "$createdb_bin" "$psql_bin" "$pg_isready_bin"; do
    [[ -x "$tool" ]] || {
      printf 'Beta local app requires executable: %s\n' "$tool" >&2
      exit 1
    }
  done
}

validate_local_overrides() {
  for setting in pg_port app_port; do
    local value
    value=${!setting}
    if ! [[ "$value" =~ ^[0-9]+$ ]] || ((value < 1024 || value > 65535)); then
      printf 'Invalid Beta local port for %s: %s\n' "$setting" "$value" >&2
      exit 1
    fi
  done
  if ! [[ "$database_name" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
    printf 'Invalid Beta local database name\n' >&2
    exit 1
  fi
}

ensure_cluster() {
  if [[ -e "$data_dir" && ! -f "$data_dir/PG_VERSION" ]]; then
    printf 'Refusing non-PostgreSQL Beta data path: %s\n' "$data_dir" >&2
    exit 1
  fi
  if [[ ! -e "$data_dir" ]]; then
    mkdir -p "$data_dir"
    "$initdb_bin" -D "$data_dir" -U beta_pg_admin \
      --auth-local=trust --auth-host=trust --no-locale >/dev/null
  fi
}

start_database() {
  ensure_cluster
  mkdir -p "$socket_dir"
  chmod 700 "$socket_dir"
  if "$pg_ctl_bin" -D "$data_dir" status >/dev/null 2>&1; then
    return
  fi
  if ss -H -ltn "sport = :$pg_port" | grep -q .; then
    printf 'Refusing occupied Beta port 127.0.0.1:%s\n' "$pg_port" >&2
    exit 1
  fi
  "$pg_ctl_bin" -D "$data_dir" \
    -o "-p $pg_port -h 127.0.0.1 -k $socket_dir" \
    -l "$data_dir/server.log" -w start >/dev/null
}

ensure_roles_and_database() {
  "$psql_bin" "$admin_url" -v ON_ERROR_STOP=1 -q <<'SQL'
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ubuntu') THEN
    CREATE ROLE ubuntu LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticated') THEN
    CREATE ROLE authenticated NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'anon') THEN
    CREATE ROLE anon NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
  END IF;
END
$$;
SQL
  if ! "$psql_bin" "$admin_url" -Atqc \
    "SELECT 1 FROM pg_database WHERE datname = '$database_name'" | grep -qx 1; then
    "$createdb_bin" --host=127.0.0.1 --port="$pg_port" \
      --username=beta_pg_admin --owner=ubuntu "$database_name"
  fi
}

migration_checksum() {
  while IFS= read -r migration; do
    case "$migration" in
      ''|\#*) continue ;;
    esac
    sha256sum "$rust_root/migrations/$migration"
  done < "$manifest_path" | sha256sum | awk '{print $1}'
}

apply_migrations() {
  bash "$script_directory/check-migration-manifest.sh" >/dev/null
  local checksum
  local latest
  local stored_checksum
  checksum=$(migration_checksum)
  latest=$(awk 'NF && $1 !~ /^#/ {value=$1} END {print value}' "$manifest_path")
  "$psql_bin" "$database_url" -v ON_ERROR_STOP=1 -q <<'SQL'
CREATE TABLE IF NOT EXISTS public.beta_migration_state (
  state_id boolean PRIMARY KEY DEFAULT true,
  manifest_sha256 text NOT NULL,
  latest_migration text NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);
SQL
  stored_checksum=$("$psql_bin" "$database_url" -Atqc \
    "SELECT manifest_sha256 FROM public.beta_migration_state WHERE state_id = true" || true)
  if [[ "$stored_checksum" != "$checksum" ]]; then
    while IFS= read -r migration; do
      case "$migration" in
        ''|\#*) continue ;;
      esac
      printf 'Applying %s\n' "$migration"
      "$psql_bin" "$database_url" -v ON_ERROR_STOP=1 -q \
        -f "$rust_root/migrations/$migration" >/dev/null
    done < "$manifest_path"
    "$psql_bin" "$database_url" -v ON_ERROR_STOP=1 -q \
      -v checksum="$checksum" -v latest="$latest" <<'SQL'
INSERT INTO public.beta_migration_state (state_id, manifest_sha256, latest_migration)
VALUES (true, :'checksum', :'latest')
ON CONFLICT (state_id) DO UPDATE
SET manifest_sha256 = EXCLUDED.manifest_sha256,
    latest_migration = EXCLUDED.latest_migration,
    updated_at = now();
SQL
  fi
}

prepare_database() {
  start_database
  ensure_roles_and_database
  apply_migrations
  "$pg_isready_bin" -h 127.0.0.1 -p "$pg_port" >/dev/null
  printf 'Beta database ready: %s\n' "$database_url"
}

run_app() {
  prepare_database
  [[ -x "$binary_path" ]] || {
    printf 'Missing release binary: %s\n' "$binary_path" >&2
    printf 'Build it with: cargo build --release --bin tgbot-app\n' >&2
    exit 1
  }
  local internal_secret
  local session_secret
  local encryption_key
  internal_secret=$(openssl rand -hex 32)
  session_secret=$(openssl rand -hex 32)
  encryption_key=$(openssl rand -hex 32)
  cd /tmp
  exec env -i \
    PATH="$PATH" \
    HOME="${HOME:-/home/ubuntu}" \
    DATABASE_URL="$database_url" \
    SAAS_BIND_ADDR="127.0.0.1:${app_port}" \
    SAAS_DB_MAX_CONNECTIONS="2" \
    SAAS_ALLOW_REST_ONLY="false" \
    INTERNAL_SAAS_SECRET="$internal_secret" \
    SAAS_SESSION_SECRET="$session_secret" \
    TGBOT_SECRET_ENCRYPTION_KEY="$encryption_key" \
    SUPABASE_URL="http://127.0.0.1:18092" \
    SUPABASE_SERVICE_ROLE_KEY="beta-local-placeholder" \
    LITELLM_BASE_URL="http://127.0.0.1:18091" \
    LITELLM_API_KEY="beta-local-placeholder" \
    TELEGRAM_API_BASE_URL="http://127.0.0.1:18092" \
    TELEGRAM_COMMANDS_ENABLED="false" \
    TELEGRAM_AUTO_REPLIES_ENABLED="false" \
    TELEGRAM_WALLET_MONITORS_ENABLED="false" \
    CORS_ALLOWED_ORIGIN="*" \
    RUST_LOG="info" \
    "$binary_path"
}

status() {
  printf 'data_dir=%s\n' "$data_dir"
  printf 'database=%s\n' "$database_url"
  if "$pg_ctl_bin" -D "$data_dir" status >/dev/null 2>&1; then
    printf 'postgres=running\n'
  else
    printf 'postgres=stopped\n'
  fi
  if curl -fsS --max-time 1 "http://127.0.0.1:${app_port}/healthz" >/dev/null 2>&1; then
    printf 'app=healthy\n'
  else
    printf 'app=stopped\n'
  fi
}

stop_database() {
  if "$pg_ctl_bin" -D "$data_dir" status >/dev/null 2>&1; then
    "$pg_ctl_bin" -D "$data_dir" -m fast -w stop >/dev/null
  fi
  printf 'Beta PostgreSQL stopped; data retained at %s\n' "$data_dir"
}

require_tools
validate_local_overrides
case "${1:-run}" in
  migrate) prepare_database ;;
  run) run_app ;;
  status) status ;;
  stop) stop_database ;;
  *)
    printf 'Usage: %s {migrate|run|status|stop}\n' "$0" >&2
    exit 2
    ;;
esac
