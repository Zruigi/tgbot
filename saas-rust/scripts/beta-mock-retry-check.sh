#!/usr/bin/env bash
# Read-only acceptance snapshot for a disposable local Telegram mock stack.
#
# This script deliberately does not start or stop any process, call a mutating
# Telegram endpoint, or read a Bot token. The mock bridge/app must already be
# running and configured by the operator.

set -euo pipefail

script_directory=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
report_directory=${BETA_REPORT_DIR:-"$rust_root/target/beta-reports"}
report_path="$report_directory/beta-mock-retry-check.json"
mock_telegram_url=${BETA_MOCK_TELEGRAM_URL:-http://127.0.0.1:18092}
webhook_probe_url=${BETA_MOCK_WEBHOOK_PROBE_URL:-http://127.0.0.1:18090/healthz}
heartbeat_file=${BETA_MOCK_HEARTBEAT_FILE:-/tmp/tgbot-beta-real-bot/bridge.heartbeat}
expected_bot_id=${BETA_MOCK_EXPECTED_BOT_ID:-777000}
expected_failures=${BETA_MOCK_EXPECTED_GET_UPDATES_FAILURES:-1}
heartbeat_max_age=${BETA_MOCK_HEARTBEAT_MAX_AGE_SECONDS:-90}
request_timeout=${BETA_MOCK_REQUEST_TIMEOUT_SECONDS:-3}

for tool in awk curl date grep jq mkdir readlink stat tr; do
  command -v "$tool" >/dev/null || {
    printf 'Beta mock retry check requires command: %s\n' "$tool" >&2
    exit 1
  }
done

enabled() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

# An explicit gate prevents accidentally pointing this read-only check at a
# provider or a production service.
if ! enabled "${BETA_DISPOSABLE_MOCK:-}"; then
  printf 'Refusing mock acceptance check; set BETA_DISPOSABLE_MOCK=true explicitly\n' >&2
  exit 1
fi

is_local_http_url() {
  case "$1" in
    http://127.0.0.1:*|http://localhost:*) return 0 ;;
    *) return 1 ;;
  esac
}

if ! is_local_http_url "$mock_telegram_url"; then
  printf 'Refusing non-local mock Telegram URL: %s\n' "$mock_telegram_url" >&2
  exit 1
fi
if ! is_local_http_url "$webhook_probe_url"; then
  printf 'Refusing non-local webhook probe URL: %s\n' "$webhook_probe_url" >&2
  exit 1
fi

[[ "$expected_bot_id" =~ ^[0-9]+$ ]] || {
  printf 'BETA_MOCK_EXPECTED_BOT_ID must be an integer\n' >&2
  exit 1
}
[[ "$expected_failures" =~ ^[1-9][0-9]*$ ]] || {
  printf 'BETA_MOCK_EXPECTED_GET_UPDATES_FAILURES must be a positive integer\n' >&2
  exit 1
}
[[ "$heartbeat_max_age" =~ ^[0-9]+$ ]] || {
  printf 'BETA_MOCK_HEARTBEAT_MAX_AGE_SECONDS must be a non-negative integer\n' >&2
  exit 1
}
[[ "$request_timeout" =~ ^[1-9][0-9]*$ ]] || {
  printf 'BETA_MOCK_REQUEST_TIMEOUT_SECONDS must be a positive integer\n' >&2
  exit 1
}

failed_checks=()
record_failure() {
  failed_checks+=("$1")
}

health_status=unavailable
health_json=''
if health_json=$(curl -fsS --max-time "$request_timeout" "$mock_telegram_url/health"); then
  if jq -e '.status == "ok"' >/dev/null 2>&1 <<<"$health_json"; then
    health_status=ok
  else
    record_failure mock_telegram_health_payload
  fi
else
  record_failure mock_telegram_health_endpoint
fi

calls_json='[]'
if calls_json=$(curl -fsS --max-time "$request_timeout" "$mock_telegram_url/calls"); then
  if ! jq -e 'type == "array"' >/dev/null 2>&1 <<<"$calls_json"; then
    record_failure mock_telegram_calls_payload
    calls_json='[]'
  fi
else
  record_failure mock_telegram_calls_endpoint
fi

# The mock server records each getUpdates attempt. The first configured
# MOCK_TELEGRAM_GET_UPDATES_FAILURES attempts are transient failures; the next
# attempt is the successful retry. No endpoint is mutated here, so a fresh
# mock process (or an operator-provided expected attempt number) is required
# for a reproducible snapshot.
updates_json=$(jq --argjson bot_id "$expected_bot_id" '
  [ .[] | select(.method == "getUpdates" and .bot_id == $bot_id) ] | sort_by(.attempt)
' <<<"$calls_json")
updates_count=$(jq 'length' <<<"$updates_json")
first_attempt=$(jq -r '.[0].attempt // empty' <<<"$updates_json")
successful_retry_attempt=$((expected_failures + 1))
if (( updates_count < successful_retry_attempt )); then
  record_failure "get_updates_retry_attempts_$successful_retry_attempt"
else
  for ((index = 0; index < successful_retry_attempt; index += 1)); do
    expected_attempt=$((index + 1))
    actual_attempt=$(jq -r --argjson index "$index" '.[ $index ].attempt // empty' <<<"$updates_json")
    if [[ "$actual_attempt" != "$expected_attempt" ]]; then
      record_failure "get_updates_attempt_sequence_$expected_attempt"
    fi
  done
fi

heartbeat_status=missing
heartbeat_age_seconds=''
heartbeat_timestamp=''
if [[ -f "$heartbeat_file" ]]; then
  heartbeat_timestamp=$(tr -d '[:space:]' <"$heartbeat_file" 2>/dev/null || true)
  heartbeat_mtime=$(stat -c '%Y' "$heartbeat_file" 2>/dev/null || true)
  now=$(date +%s)
  if [[ "$heartbeat_timestamp" =~ ^[0-9]+$ && "$heartbeat_mtime" =~ ^[0-9]+$ && "$now" =~ ^[0-9]+$ ]]; then
    heartbeat_age_seconds=$((now - heartbeat_mtime))
    timestamp_age_seconds=$((now - heartbeat_timestamp))
    if (( heartbeat_age_seconds >= 0 && heartbeat_age_seconds <= heartbeat_max_age \
      && timestamp_age_seconds >= 0 && timestamp_age_seconds <= heartbeat_max_age )); then
      heartbeat_status=fresh
    else
      heartbeat_status=stale
      record_failure heartbeat_stale
    fi
  else
    heartbeat_status=invalid
    record_failure heartbeat_payload
  fi
else
  record_failure heartbeat_missing
fi

# A GET-only origin probe is intentional. It verifies local endpoint
# reachability without POSTing an update, deriving a secret, or touching
# Telegram state. The full POST webhook probe remains a separate explicit
# test in beta-real-bot-stack.sh.
webhook_probe_status=000
if ! webhook_probe_status=$(curl -sS --max-time "$request_timeout" -o /dev/null \
  -w '%{http_code}' "$webhook_probe_url"); then
  webhook_probe_status=000
fi
if [[ "$webhook_probe_status" != "200" ]]; then
  record_failure webhook_origin_get_status_${webhook_probe_status}
fi

if ((${#failed_checks[@]} == 0)); then
  audit_status=passed
else
  audit_status=failed
fi

mkdir -p "$report_directory"
failed_checks_json=$(printf '%s\n' "${failed_checks[@]}" | jq -Rsc 'split("\n") | map(select(length > 0))')
jq -n \
  --arg generated_at_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg status "$audit_status" \
  --arg scope "local_disposable_mock_read_only" \
  --arg mock_telegram_url "$mock_telegram_url" \
  --arg webhook_probe_url "$webhook_probe_url" \
  --arg heartbeat_file "$heartbeat_file" \
  --arg heartbeat_status "$heartbeat_status" \
  --arg heartbeat_age_seconds "$heartbeat_age_seconds" \
  --arg heartbeat_timestamp "$heartbeat_timestamp" \
  --arg health_status "$health_status" \
  --arg webhook_probe_status "$webhook_probe_status" \
  --arg expected_bot_id "$expected_bot_id" \
  --arg expected_failures "$expected_failures" \
  --arg observed_get_updates_attempts "$updates_count" \
  --arg first_get_updates_attempt "$first_attempt" \
  --arg successful_retry_attempt "$successful_retry_attempt" \
  --argjson failed_checks "$failed_checks_json" \
  '{
    report_type: "rust_beta_mock_retry_check",
    generated_at_utc: $generated_at_utc,
    status: $status,
    scope: $scope,
    credentials: "none_read_or_injected",
    mutating_requests: false,
    services_started: false,
    mock_telegram: {
      url: $mock_telegram_url,
      health: $health_status,
      expected_bot_id: ($expected_bot_id | tonumber),
      expected_transient_failures: ($expected_failures | tonumber),
      observed_get_updates_attempts: ($observed_get_updates_attempts | tonumber),
      first_get_updates_attempt: ($first_get_updates_attempt | if . == "" then null else tonumber end),
      successful_retry_attempt: ($successful_retry_attempt | tonumber),
      evidence: "GET /calls sequence; failure count comes from the externally configured mock fixture"
    },
    heartbeat: {
      path: $heartbeat_file,
      status: $heartbeat_status,
      age_seconds: ($heartbeat_age_seconds | if . == "" then null else tonumber end),
      payload_timestamp: ($heartbeat_timestamp | if . == "" then null else tonumber end)
    },
    webhook_probe: {
      method: "GET",
      url: $webhook_probe_url,
      status: ($webhook_probe_status | tonumber),
      semantics: "local origin reachability only; does not replace the POST webhook semantic probe"
    },
    failed_checks: $failed_checks
  }' >"$report_path"

if [[ "$audit_status" != "passed" ]]; then
  printf 'Beta mock retry check failed: %s checks; report: %s\n' "${#failed_checks[@]}" "$report_path" >&2
  exit 1
fi
printf 'Beta mock retry check passed; report: %s\n' "$report_path"

