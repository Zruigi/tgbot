#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
launcher="$script_directory/beta-local-app.sh"
test_bot_launcher="$script_directory/beta-test-bot-polling.sh"
test_bot_control="$script_directory/beta-test-bot-control.sh"
real_bot_stack="$script_directory/beta-real-bot-stack.sh"
report_directory=${BETA_REPORT_DIR:-"$rust_root/target/beta-reports"}
report_path="$report_directory/isolation-audit.json"
failed_checks=()

for tool in curl date grep jq mkdir ss stat; do
  command -v "$tool" >/dev/null || {
    printf 'Beta isolation audit requires command: %s\n' "$tool" >&2
    exit 1
  }
done

record_failure() {
  failed_checks+=("$1")
}

check_file() {
  local check_name=$1
  local file_path=$2
  [[ -f "$file_path" ]] || record_failure "$check_name"
}

check_contains() {
  local check_name=$1
  local needle=$2
  local file_path=$3
  grep -Fq -- "$needle" "$file_path" || record_failure "$check_name"
}

check_absent() {
  local check_name=$1
  local needle=$2
  local file_path=$3
  grep -Fq -- "$needle" "$file_path" && record_failure "$check_name" || true
}

check_file launcher_present "$launcher"
check_file test_bot_launcher_present "$test_bot_launcher"
check_file test_bot_control_present "$test_bot_control"
check_file real_bot_stack_present "$real_bot_stack"
if [[ -f "$launcher" ]]; then
  check_contains launcher_uses_clean_environment 'exec env -i' "$launcher"
  check_contains launcher_database_host_is_local 'database_url="postgresql://ubuntu@127.0.0.1:${pg_port}/${database_name}"' "$launcher"
  check_contains launcher_database_port_default 'BETA_LOCAL_PG_PORT:-15432' "$launcher"
  check_contains launcher_database_name_default 'BETA_LOCAL_DATABASE_NAME:-tgbot_rust_beta' "$launcher"
  check_contains launcher_app_bind_default 'BETA_LOCAL_APP_PORT:-18090' "$launcher"
  check_contains launcher_supabase_mock_is_local 'SUPABASE_URL="http://127.0.0.1:18092"' "$launcher"
  check_contains launcher_llm_mock_is_local 'LITELLM_BASE_URL="http://127.0.0.1:18091"' "$launcher"
  check_contains launcher_telegram_mock_is_local 'TELEGRAM_API_BASE_URL="http://127.0.0.1:18092"' "$launcher"
  check_absent launcher_does_not_inject_bot_token 'BOT_TOKEN=' "$launcher"
  check_absent launcher_does_not_use_production_database 'SUPABASE_DATABASE_URL' "$launcher"
  check_absent launcher_does_not_reference_production_root '/home/ubuntu/tgbot' "$launcher"
fi
if [[ -f "$test_bot_control" ]]; then
  check_contains test_bot_control_uses_nohup 'nohup bash' "$test_bot_control"
  check_contains test_bot_control_uses_temp_pid '/tmp/tgbot-beta-test-bot.pid' "$test_bot_control"
  check_absent test_bot_control_does_not_set_database_url 'DATABASE_URL=' "$test_bot_control"
fi
if [[ -f "$test_bot_launcher" ]]; then
  check_contains test_bot_uses_clean_environment 'exec env -i' "$test_bot_launcher"
  check_contains test_bot_requires_explicit_gate 'BETA_TEST_BOT_ENABLED' "$test_bot_launcher"
  check_contains test_bot_enables_polling_gate 'TELEGRAM_POLLING_ENABLED=true' "$test_bot_launcher"
  check_absent test_bot_launcher_does_not_set_database_url 'DATABASE_URL=' "$test_bot_launcher"
  check_absent test_bot_launcher_does_not_set_telegram_override 'TELEGRAM_API_BASE_URL=' "$test_bot_launcher"
fi
if [[ -f "$real_bot_stack" ]]; then
  check_contains real_bot_stack_uses_heartbeat 'BETA_BRIDGE_HEARTBEAT_FILE' "$real_bot_stack"
  check_contains real_bot_stack_uses_local_database 'postgresql://ubuntu@127.0.0.1:15432/tgbot_rust_beta' "$real_bot_stack"
  check_absent real_bot_stack_does_not_reference_production_root '/home/ubuntu/tgbot' "$real_bot_stack"
fi

running_mode=false
case "${BETA_ISOLATION_AUDIT_EXPECT_RUNNING:-}" in
  1|true|TRUE|yes|YES|on|ON) running_mode=true ;;
esac
if [[ "$running_mode" == true ]]; then
  if ! ss -H -ltn 'sport = :15432' | grep -q .; then
    record_failure beta_database_port_is_running
  fi
  if ! ss -H -ltn 'sport = :18090' | grep -q .; then
    record_failure beta_app_port_is_running
  fi
  if ! curl -fsS --max-time 2 http://127.0.0.1:18090/healthz >/dev/null 2>&1; then
    record_failure beta_app_health_check
  fi
  real_bot_status=$(BETA_TEST_BOT_ENABLED=true bash "$real_bot_stack" status 2>/dev/null || true)
  if [[ "$real_bot_status" != *"bridge=running"* ]]; then
    record_failure beta_real_bot_bridge_is_running
  fi
else
  for port in 15432 18090; do
    if ss -H -ltn "sport = :$port" | grep -q .; then
      record_failure "beta_port_${port}_is_free"
    fi
  done
fi

if [[ "$running_mode" == true ]]; then
  audit_scope=local_beta_running
else
  audit_scope=local_beta_only
fi

mkdir -p "$report_directory"
failed_checks_json=$(printf '%s\n' "${failed_checks[@]}" | jq -Rsc 'split("\n") | map(select(length > 0))')
if ((${#failed_checks[@]} == 0)); then
  audit_status=passed
else
  audit_status=failed
fi

jq -n \
  --arg generated_at_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg status "$audit_status" \
  --argjson failed_checks "$failed_checks_json" \
  --arg scope "$audit_scope" \
  '{report_type: "rust_beta_isolation_audit", generated_at_utc: $generated_at_utc, status: $status, scope: $scope, credentials: "disposable_local_postgresql_and_mocks_only", failed_checks: $failed_checks}' >"$report_path"

if [[ "$audit_status" != "passed" ]]; then
  printf 'Beta isolation audit failed: %s checks\n' "${#failed_checks[@]}" >&2
  exit 1
fi
printf 'Beta isolation audit passed; report: %s\n' "$report_path"
