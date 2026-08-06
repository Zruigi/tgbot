#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
launcher="$script_directory/beta-local-app.sh"
binary_path="$rust_root/target/release/tgbot-app"
report_directory=${BETA_REPORT_DIR:-"$rust_root/target/beta-reports"}
log_path="$report_directory/local-rollback-rehearsal.log"
report_path="$report_directory/local-rollback-rehearsal.json"
app_pid=""
app_pgid=""
pg_port=${BETA_ROLLBACK_PG_PORT:-15433}
app_port=${BETA_ROLLBACK_APP_PORT:-18093}
database_name=${BETA_ROLLBACK_DATABASE_NAME:-tgbot_rust_beta_rollback}
rollback_data_dir=${BETA_ROLLBACK_DATA_DIR:-}
rollback_socket_dir=${BETA_ROLLBACK_SOCKET_DIR:-}
rollback_data_parent=""
rehearsal_status=running

if [[ -z "$rollback_data_dir" ]]; then
  rollback_data_parent=$(mktemp -d /tmp/tgbot-beta-rollback.XXXXXX)
  rollback_data_dir="$rollback_data_parent/postgres"
fi
if [[ -z "$rollback_socket_dir" ]]; then
  rollback_socket_dir=$(mktemp -d /tmp/tgbot-beta-rollback-socket.XXXXXX)
fi

for tool in awk bash curl date grep mkdir seq setsid sha256sum sleep ss; do
  command -v "$tool" >/dev/null || {
    printf 'Beta rollback rehearsal requires command: %s\n' "$tool" >&2
    exit 1
  }
done

if [[ ! -x "$binary_path" ]]; then
  printf 'Beta rollback rehearsal requires executable: %s\n' "$binary_path" >&2
  exit 1
fi
binary_sha256=$(sha256sum "$binary_path" | awk '{print $1}')

for port in "$pg_port" "$app_port"; do
  if ! [[ "$port" =~ ^[0-9]+$ ]] || ((port < 1024 || port > 65535)); then
    printf 'Invalid rollback port: %s\n' "$port" >&2
    exit 1
  fi
  if ss -H -ltn "sport = :$port" | grep -q .; then
    printf 'Refusing rollback rehearsal with occupied Beta port 127.0.0.1:%s\n' "$port" >&2
    exit 1
  fi
done

mkdir -p "$report_directory"
printf '%s\n' '{"status":"running","scope":"local_beta_only","credentials":"disposable_local_postgresql_and_mocks_only"}' >"$report_path"

if [[ "$rollback_data_dir" == "$rust_root/target/beta-postgres" ]]; then
  printf 'Refusing to reuse the active Beta database directory\n' >&2
  exit 1
fi

run_launcher() {
  env \
    BETA_LOCAL_PG_PORT="$pg_port" \
    BETA_LOCAL_APP_PORT="$app_port" \
    BETA_LOCAL_DATABASE_NAME="$database_name" \
    BETA_LOCAL_DATA_DIR="$rollback_data_dir" \
    BETA_LOCAL_SOCKET_DIR="$rollback_socket_dir" \
    bash "$launcher" "$@"
}

stop_app() {
  if [[ -z "$app_pid" || -z "$app_pgid" ]]; then
    return
  fi
  kill -TERM -- "-$app_pgid" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    if ! kill -0 "$app_pid" >/dev/null 2>&1; then
      break
    fi
    sleep 0.25
  done
  if kill -0 "$app_pid" >/dev/null 2>&1; then
    kill -KILL -- "-$app_pgid" >/dev/null 2>&1 || true
  fi
  wait "$app_pid" >/dev/null 2>&1 || true
  app_pid=""
  app_pgid=""
}

cleanup() {
  stop_app
  run_launcher stop >/dev/null 2>&1 || true
  if [[ -n "$rollback_data_parent" ]]; then
    rm -rf "$rollback_data_parent"
  fi
  if [[ -z "${BETA_ROLLBACK_SOCKET_DIR:-}" ]]; then
    rm -rf "$rollback_socket_dir"
  fi
  if [[ "$rehearsal_status" == running ]]; then
    printf '%s\n' '{"status":"failed","scope":"local_beta_only","credentials":"disposable_local_postgresql_and_mocks_only"}' >"$report_path"
  fi
}
trap cleanup EXIT INT TERM

start_app() {
  (
    export BETA_LOCAL_PG_PORT="$pg_port"
    export BETA_LOCAL_APP_PORT="$app_port"
    export BETA_LOCAL_DATABASE_NAME="$database_name"
    export BETA_LOCAL_DATA_DIR="$rollback_data_dir"
    export BETA_LOCAL_SOCKET_DIR="$rollback_socket_dir"
    exec setsid bash "$launcher" run
  ) >"$log_path" 2>&1 &
  app_pid=$!
  app_pgid=$app_pid
}

wait_for_endpoint() {
  local path=$1
  local attempt
  for attempt in $(seq 1 60); do
    if curl -fsS --max-time 1 "http://127.0.0.1:${app_port}$path" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  printf 'Beta rollback rehearsal timed out waiting for %s\n' "$path" >&2
  return 1
}

assert_stopped() {
  local attempt
  for attempt in $(seq 1 30); do
    if ! curl -fsS --max-time 1 "http://127.0.0.1:${app_port}/healthz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  printf 'Beta rollback rehearsal found a live app after stop\n' >&2
  return 1
}

start_app
wait_for_endpoint /healthz
wait_for_endpoint /readyz
stop_app
assert_stopped

start_app
wait_for_endpoint /healthz
wait_for_endpoint /readyz
printf '{"generated_at_utc":"%s","status":"passed","scope":"local_beta_only","app_port":%s,"database_port":%s,"binary_sha256":"%s","first_start_ready":true,"stopped_between_restarts":true,"second_start_ready":true,"credentials":"disposable_local_postgresql_and_mocks_only"}\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$app_port" "$pg_port" "$binary_sha256" >"$report_path"
rehearsal_status=passed
printf 'Beta local rollback/restart rehearsal passed; log: %s\n' "$log_path"
