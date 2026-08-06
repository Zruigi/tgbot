#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
runner="$script_directory/beta-test-bot-polling.sh"
pid_file="/tmp/tgbot-beta-test-bot.pid"
log_path="/tmp/tgbot-beta-test-bot.log"

for tool in awk bash cat grep kill nohup ps rm seq sleep; do
  command -v "$tool" >/dev/null || {
    printf 'Beta test Bot control requires command: %s\n' "$tool" >&2
    exit 1
  }
done

read_pid() {
  [[ -f "$pid_file" ]] || return 1
  local process_id
  process_id=$(cat "$pid_file")
  [[ "$process_id" =~ ^[0-9]+$ ]] || return 1
  printf '%s\n' "$process_id"
}

running_pid() {
  local process_id
  process_id=$(read_pid) || return 1
  kill -0 "$process_id" >/dev/null 2>&1 || return 1
  printf '%s\n' "$process_id"
}

external_polling_pid() {
  local process_id
  process_id=$(
    ps -eo pid=,comm=,args= | awk -v root="$rust_root" '
      $2 != "awk" && (index($0, root "/target/debug/bot-polling") || index($0, root "/target/release/bot-polling")) {
        print $1
        exit
      }
    '
  )
  [[ "$process_id" =~ ^[0-9]+$ ]] || return 1
  [[ "$process_id" != "$$" ]] || return 1
  printf '%s\n' "$process_id"
}

start_bot() {
  [[ "${BETA_TEST_BOT_ENABLED:-}" == "true" ]] || {
    printf 'Refusing test Bot start; set BETA_TEST_BOT_ENABLED=true explicitly\n' >&2
    exit 1
  }
  [[ -n "${BOT_TOKEN:-}" ]] || {
    printf 'Refusing test Bot start; inject BOT_TOKEN into this process environment\n' >&2
    exit 1
  }
  if process_id=$(running_pid); then
    printf 'Beta test Bot already running; pid=%s\n' "$process_id"
    return
  fi
  if process_id=$(external_polling_pid); then
    printf 'Beta test Bot already running outside control; pid=%s; use its terminal to stop it\n' "$process_id"
    return
  fi
  : >"$log_path"
  BETA_TEST_BOT_ENABLED=true nohup bash "$runner" >"$log_path" 2>&1 < /dev/null &
  process_id=$!
  printf '%s\n' "$process_id" >"$pid_file"
  for attempt in $(seq 1 15); do
    if grep -Fq 'Polling started' "$log_path"; then
      printf 'Beta test Bot is polling; pid=%s log=%s\n' "$process_id" "$log_path"
      return
    fi
    if ! kill -0 "$process_id" >/dev/null 2>&1; then
      printf 'Beta test Bot exited before polling started; inspect %s\n' "$log_path" >&2
      rm -f "$pid_file"
      exit 1
    fi
    sleep 1
  done
  printf 'Beta test Bot is still starting; pid=%s log=%s\n' "$process_id" "$log_path"
}

stop_bot() {
  local process_id
  if ! process_id=$(running_pid); then
    if process_id=$(external_polling_pid); then
      printf 'Beta test Bot is managed outside control; stop it from its terminal; pid=%s\n' "$process_id"
      return
    fi
    rm -f "$pid_file"
    printf 'Beta test Bot is stopped\n'
    return
  fi
  kill "$process_id" >/dev/null 2>&1 || true
  for attempt in $(seq 1 10); do
    if ! kill -0 "$process_id" >/dev/null 2>&1; then
      rm -f "$pid_file"
      printf 'Beta test Bot stopped\n'
      return
    fi
    sleep 1
  done
  kill -KILL "$process_id" >/dev/null 2>&1 || true
  rm -f "$pid_file"
  printf 'Beta test Bot stopped\n'
}

status_bot() {
  local process_id
  if process_id=$(running_pid); then
    if grep -Fq 'Polling started' "$log_path" 2>/dev/null; then
      printf 'state=ready pid=%s log=%s\n' "$process_id" "$log_path"
    else
      printf 'state=starting pid=%s log=%s\n' "$process_id" "$log_path"
    fi
  elif process_id=$(external_polling_pid); then
    printf 'state=external pid=%s\n' "$process_id"
  else
    printf 'state=stopped\n'
  fi
}

case "${1:-status}" in
  start) start_bot ;;
  stop) stop_bot ;;
  status) status_bot ;;
  *)
    printf 'Usage: %s {start|stop|status}\n' "$0" >&2
    exit 2
    ;;
esac
