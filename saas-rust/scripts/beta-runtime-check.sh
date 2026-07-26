#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
database_name=tgbot_rust_beta
database_url="postgresql:///$database_name"
bind_address=127.0.0.1:18090
base_url="http://$bind_address"
server_log=$(mktemp)
report_pdf=$(mktemp)
report_xlsx=$(mktemp)
image_fixture=$(mktemp --suffix=.png)
server_pid=
llm_pid=
telegram_pid=

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ -n "$llm_pid" ]]; then
    kill "$llm_pid" 2>/dev/null || true
    wait "$llm_pid" 2>/dev/null || true
  fi
  if [[ -n "$telegram_pid" ]]; then
    kill "$telegram_pid" 2>/dev/null || true
    wait "$telegram_pid" 2>/dev/null || true
  fi
  rm -f "$server_log" "$report_pdf" "$report_xlsx" "$image_fixture"
}
trap cleanup EXIT

report_failure() {
  local exit_status=$?
  echo "Beta runtime gate failed at line ${BASH_LINENO[0]} (exit $exit_status)." >&2
  exit "$exit_status"
}
trap report_failure ERR

for command in base64 cargo curl jq node openssl psql sudo unzip xxd; do
  command -v "$command" >/dev/null || {
    echo "Missing required command: $command" >&2
    exit 1
  }
done

if ss -H -ltn "sport = :18090" | grep -q .; then
  echo "Beta runtime gate requires unused port $bind_address." >&2
  exit 1
fi
if ss -H -ltn "sport = :18091" | grep -q .; then
  echo "Beta runtime gate requires unused mock LLM port 127.0.0.1:18091." >&2
  exit 1
fi
if ss -H -ltn "sport = :18092" | grep -q .; then
  echo "Beta runtime gate requires unused mock Telegram port 127.0.0.1:18092." >&2
  exit 1
fi

sudo -u postgres psql -v ON_ERROR_STOP=1 <<'SQL'
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ubuntu') THEN
    CREATE ROLE ubuntu LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
  END IF;
END $$;
SQL
sudo -u postgres dropdb --if-exists "$database_name"
sudo -u postgres createdb --owner=ubuntu "$database_name"

migrations=(
  0001_saas_core.sql
  0002_merchant_public_fields.sql
  0001_core.sql
  0002_auth_sessions.sql
  0003_saas.sql
  0004_broadcast_push.sql
  0004_monitor_finance.sql
  0005_indexes.sql
  0006_chat_tenant_isolation.sql
  0007_broadcast_tenant_isolation.sql
  0008_group_telegram_tenant.sql
  0009_channel_reminder_cleanup.sql
  0010_potential_user_tenant_policy.sql
  0011_group_message_policy_outbox.sql
  0012_telegram_command_reply_outbox.sql
  0013_telegram_auto_reply_outbox.sql
)

for pass in 1 2; do
  for migration in "${migrations[@]}"; do
    psql "$database_url" -v ON_ERROR_STOP=1 \
      -f "$rust_root/migrations/$migration" >/dev/null
  done
done

psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO auth.users (id, tg_user_id, username)
VALUES ('11111111-1111-4111-8111-111111111111', 10001, 'beta_owner');
INSERT INTO merchants (id, tenant_id, owner_user_id, owner_tg_user_id, merchant_name, status, onboarding_completed)
VALUES ('22222222-2222-4222-8222-222222222222', 'beta-local', '11111111-1111-4111-8111-111111111111', 10001, 'Beta Local', 'active', true);
INSERT INTO tenant_memberships (user_id, tenant_id, merchant_id, role)
VALUES ('11111111-1111-4111-8111-111111111111', 'beta-local', '22222222-2222-4222-8222-222222222222', 'merchant_owner');
INSERT INTO auth.users (id, tg_user_id, username)
VALUES ('55555555-5555-4555-8555-555555555555', 20001, 'other_owner');
INSERT INTO merchants (id, tenant_id, owner_user_id, owner_tg_user_id, merchant_name, status, onboarding_completed, bot_id)
VALUES ('66666666-6666-4666-8666-666666666666', 'other-tenant', '55555555-5555-4555-8555-555555555555', 20001, 'Other Tenant', 'active', true, 888000);
INSERT INTO groups.tracked_groups
  (group_id, group_title, tenant_id, is_monitored, custom_keywords, verify_enabled, verify_type, verify_channel_id)
VALUES
  (-10001, 'Beta Group One', 'beta-local', true, '定制需求', true, 'channel', '-30001'),
  (-10002, 'Beta Group Two', 'beta-local', false, NULL, false, 'captcha', NULL),
  (-20001, 'Other Group', 'other-tenant', true, NULL, true, 'channel', '-30001');
INSERT INTO groups.group_members
  (user_id, username, first_name, group_id, group_title, tenant_id, status, is_verified)
VALUES
  (30001, 'shared_beta_user', 'Shared', -10001, 'Beta Group One', 'beta-local', 'active', false),
  (30002, 'beta_admin', 'Admin', -10001, 'Beta Group One', 'beta-local', 'admin', false),
  (30003, 'beta_creator', 'Creator', -10001, 'Beta Group One', 'beta-local', 'admin', false),
  (30004, 'beta_failure', 'Failure', -10001, 'Beta Group One', 'beta-local', 'active', false),
  (40001, 'other_user', 'Other', -20001, 'Other Group', 'other-tenant', 'active', false);
INSERT INTO groups.group_messages
  (group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, tenant_id, message_ts)
VALUES
  (-10001, 30001, 'shared_beta_user', 601, 'user', 'original beta group message', 'text', 'beta-local', 1699999999),
  (-20001, 40001, 'other_user', 701, 'user', 'other tenant group message', 'text', 'other-tenant', 1699999998);
INSERT INTO job_queue (tenant_id, job_type, payload, status)
VALUES ('beta-local', 'unregistered_beta_probe', '{}'::jsonb, 'pending');
INSERT INTO chat.users (user_id, username, first_name)
VALUES (30001, 'shared_beta_user', 'Shared'), (40001, 'other_user', 'Other');
INSERT INTO chat.messages
  (from_userid, to_userid, content, msg_type, is_read, tg_msg_id, tenant_id)
VALUES
  (30001, 777000, 'beta inbound', 'text', false, 450, 'beta-local'),
  (30001, 888000, 'other shared inbound', 'text', false, 550, 'other-tenant'),
  (40001, 888000, 'other inbound', 'text', false, 551, 'other-tenant');
INSERT INTO chat.admin_inbox (user_id, username, summary, unread_count, tenant_id)
VALUES
  (30001, 'shared_beta_user', 'beta inbound', 1, 'beta-local'),
  (30001, 'shared_beta_user', 'other shared inbound', 1, 'other-tenant'),
  (40001, 'other_user', 'other inbound', 1, 'other-tenant');
SQL

psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
BEGIN;
INSERT INTO chat.messages
  (from_userid, to_userid, content, msg_type, is_read, tg_msg_id, tenant_id)
VALUES
  (30001, 777000, '营业时间', 'text', false, 448, 'beta-local'),
  (30001, 777000, '/help', 'text', false, 449, 'beta-local');
DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM job_queue
    WHERE job_type = 'telegram_command_reply'
  ) THEN
    RAISE EXCEPTION 'telegram command outbox must default to disabled';
  END IF;
  IF EXISTS (
    SELECT 1
    FROM job_queue
    WHERE job_type = 'telegram_auto_reply'
  ) THEN
    RAISE EXCEPTION 'telegram auto-reply outbox must default to disabled';
  END IF;
END
$$;
ROLLBACK;
SQL

printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=' \
  | base64 -d >"$image_fixture"

cargo build -q --manifest-path "$rust_root/Cargo.toml" -p tgbot-app --bin tgbot-app
MOCK_LITELLM_PORT=18091 node "$script_directory/mock-litellm.mjs" &
llm_pid=$!
MOCK_TELEGRAM_PORT=18092 node "$script_directory/mock-telegram.mjs" &
telegram_pid=$!
for _ in $(seq 1 50); do
  if curl -fsS http://127.0.0.1:18091/health >/dev/null; then
    break
  fi
  if ! kill -0 "$llm_pid" 2>/dev/null; then
    echo "Mock LLM failed to start." >&2
    exit 1
  fi
  sleep 0.1
done
curl -fsS http://127.0.0.1:18091/health | jq -e '.status == "ok"' >/dev/null
for _ in $(seq 1 50); do
  if curl -fsS http://127.0.0.1:18092/health >/dev/null; then
    break
  fi
  if ! kill -0 "$telegram_pid" 2>/dev/null; then
    echo "Mock Telegram failed to start." >&2
    exit 1
  fi
  sleep 0.1
done
curl -fsS http://127.0.0.1:18092/health | jq -e '.status == "ok"' >/dev/null
secret_encryption_key=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
env \
  DATABASE_URL="$database_url" \
  INTERNAL_SAAS_SECRET=beta-local-secret \
  LITELLM_BASE_URL=http://127.0.0.1:18091 \
  LITELLM_API_KEY=beta-mock \
  TELEGRAM_API_BASE_URL=http://127.0.0.1:18092 \
  TELEGRAM_WEBHOOK_BASE_URL=http://127.0.0.1:18090 \
  TELEGRAM_AUTO_REPLIES_ENABLED=true \
  TELEGRAM_COMMANDS_ENABLED=true \
  TGBOT_SECRET_ENCRYPTION_KEY="$secret_encryption_key" \
  TGBOT_PROJECT_ROOT="$rust_root" \
  AI_SKILL_LOCAL_DIRS=scripts/fixtures/skills \
  SAAS_BIND_ADDR="$bind_address" \
  RUST_LOG=warn \
  HTTP_RATE_LIMIT_REQUESTS_PER_MINUTE=1000 \
  WORKER_POLL_INTERVAL_SECS=1 \
  PUSH_TASK_CRON_INTERVAL_SECS=1 \
  CHANNEL_REMINDER_CLEANUP_INTERVAL_SECS=1 \
  "$rust_root/target/debug/tgbot-app" >"$server_log" 2>&1 &
server_pid=$!

for _ in $(seq 1 50); do
  if curl -fsS "$base_url/api/health" >/dev/null; then
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    cat "$server_log" >&2
    exit 1
  fi
  sleep 0.2
done
curl -fsS "$base_url/api/health" | jq -e '.status == "ok"' >/dev/null

for _ in $(seq 1 50); do
  completed_seed_policy_jobs=$(psql "$database_url" -Atc "SELECT count(*) FROM job_queue WHERE job_type = 'group_message_policy' AND status = 'completed'")
  [[ "$completed_seed_policy_jobs" == 2 ]] && break
  sleep 0.1
done
[[ "$completed_seed_policy_jobs" == 2 ]]
[[ $(psql "$database_url" -Atc "SELECT status || ':' || attempts FROM job_queue WHERE job_type = 'unregistered_beta_probe'") == pending:0 ]]

headers=(
  -H 'x-saas-internal-secret: beta-local-secret'
  -H 'x-user-role: merchant_owner'
  -H 'x-user-id: 11111111-1111-4111-8111-111111111111'
  -H 'x-tenant-id: beta-local'
  -H 'x-merchant-id: 22222222-2222-4222-8222-222222222222'
)
other_headers=(
  -H 'x-saas-internal-secret: beta-local-secret'
  -H 'x-user-role: merchant_owner'
  -H 'x-user-id: 55555555-5555-4555-8555-555555555555'
  -H 'x-tenant-id: other-tenant'
  -H 'x-merchant-id: 66666666-6666-4666-8666-666666666666'
)
viewer_headers=(
  -H 'x-saas-internal-secret: beta-local-secret'
  -H 'x-user-role: merchant_viewer'
  -H 'x-user-id: 11111111-1111-4111-8111-111111111111'
  -H 'x-tenant-id: beta-local'
  -H 'x-merchant-id: 22222222-2222-4222-8222-222222222222'
)
platform_headers=(
  -H 'x-saas-internal-secret: beta-local-secret'
  -H 'x-user-role: platform_admin'
  -H 'x-user-id: 11111111-1111-4111-8111-111111111111'
)
finance_url="$base_url/api/v1/monitors/finance"

auto_reply_url="$base_url/api/merchant/telegram/auto-reply"
curl -fsS "${headers[@]}" "$auto_reply_url" \
  | jq -e '.status == "ok" and .data == {"enabled":false,"rules":[]}' >/dev/null
curl -fsS "${viewer_headers[@]}" "$auto_reply_url" \
  | jq -e '.status == "ok" and .data == {"enabled":false,"rules":[]}' >/dev/null
viewer_auto_reply_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" \
  -X PUT -H 'content-type: application/json' \
  -d '{"enabled":true,"rules":[]}' "$auto_reply_url")
[[ "$viewer_auto_reply_status" == 403 ]]
beta_auto_reply_config='{"enabled":true,"rules":[{"match_type":"exact","trigger":" 营业时间 ","reply":" Beta 营业时间：周一至周五 ","case_sensitive":false},{"match_type":"contains","trigger":"人工客服","reply":"客服稍后联系您","case_sensitive":false},{"match_type":"exact","trigger":"失败测试","reply":"此回复不应落库","case_sensitive":false}]}'
curl -fsS "${headers[@]}" -X PUT -H 'content-type: application/json' \
  -d "$beta_auto_reply_config" "$auto_reply_url" \
  | jq -e '.status == "ok" and .data.enabled == true and .data.rules[0].trigger == "营业时间" and .data.rules[0].reply == "Beta 营业时间：周一至周五"' >/dev/null
other_auto_reply_config='{"enabled":true,"rules":[{"match_type":"exact","trigger":"营业时间","reply":"Other tenant reply","case_sensitive":false}]}'
curl -fsS "${other_headers[@]}" -X PUT -H 'content-type: application/json' \
  -d "$other_auto_reply_config" "$auto_reply_url" \
  | jq -e '.data.enabled == true and .data.rules[0].reply == "Other tenant reply"' >/dev/null
curl -fsS "${headers[@]}" "$auto_reply_url" \
  | jq -e '.data.rules[0].reply == "Beta 营业时间：周一至周五"' >/dev/null
duplicate_auto_reply_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -X PUT -H 'content-type: application/json' \
  -d '{"enabled":true,"rules":[{"match_type":"exact","trigger":"PRICE","reply":"one"},{"match_type":"exact","trigger":"price","reply":"two"}]}' \
  "$auto_reply_url")
[[ "$duplicate_auto_reply_status" == 400 ]]
curl -fsS "${headers[@]}" "$auto_reply_url" \
  | jq -e '.data.rules[0].reply == "Beta 营业时间：周一至周五"' >/dev/null

config_merchant_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" "$base_url/api/config")
[[ "$config_merchant_status" == 403 ]]
curl -fsS "${platform_headers[@]}" "$base_url/api/config" \
  | jq -e '.status == "ok" and .data.llm_api_key == "" and .data.llm_api_mode == "rented"' >/dev/null
curl -fsS "${platform_headers[@]}" -H 'content-type: application/json' \
  -d '{"llm_api_mode":"own","llm_model_type":"gemini","llm_base_url":"https://llm.beta.example/v1","llm_api_key":"beta-config-secret","llm_model":"ignored-by-lock","llm_model_lock_enabled":"true","llm_model_lock_target":"beta-locked-model","global_auto_translate":"true","translation_engine":"llm"}' \
  "$base_url/api/config" | jq -e '.status == "ok" and .data.updated >= 8' >/dev/null
curl -fsS "${platform_headers[@]}" "$base_url/api/config" \
  | jq -e '.data.llm_api_mode == "own" and .data.llm_model_type == "gemini" and .data.llm_model == "beta-locked-model" and .data.llm_model_lock_enabled == true and .data.global_auto_translate == true and .data.llm_api_key == "" and .data.llm_api_key_configured == true' >/dev/null
invalid_config_status=$(curl -sS -o /dev/null -w '%{http_code}' "${platform_headers[@]}" \
  -H 'content-type: application/json' -d '{"llm_api_mode":"rented","llm_base_url":"file:///tmp/not-allowed"}' \
  "$base_url/api/config")
[[ "$invalid_config_status" == 400 ]]
curl -fsS "${platform_headers[@]}" "$base_url/api/config" \
  | jq -e '.data.llm_api_mode == "own"' >/dev/null
curl -fsS "${platform_headers[@]}" -H 'content-type: application/json' \
  -d '{"llm_api_key":"","global_auto_translate":"false"}' "$base_url/api/config" \
  | jq -e '.data.updated == 1' >/dev/null
curl -fsS "${platform_headers[@]}" "$base_url/api/config/gemini-models" \
  | jq -e '.data | index("gemini-3.1-pro-preview") != null' >/dev/null
gemini_merchant_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  "$base_url/api/config/gemini-models")
[[ "$gemini_merchant_status" == 403 ]]
curl -fsS "${platform_headers[@]}" -H 'content-type: application/json' \
  -d '{"vertex_api_key":"beta-vertex-key","oneapi_base_url":"","relay_api_key":"","model":"must-be-locked"}' \
  "$base_url/api/config/vertex/oneclick" \
  | jq -e '.data.oneapi_base_url == "https://generativelanguage.googleapis.com/v1beta" and .data.model == "beta-locked-model" and (.data.relay_api_key | startswith("sk-"))' >/dev/null
curl -fsS "${platform_headers[@]}" "$base_url/api/config" \
  | jq -e '.data.llm_model == "beta-locked-model" and .data.global_auto_translate == false and .data.llm_api_key == "" and .data.llm_api_key_configured == true' >/dev/null

viewer_bot_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" \
  -H 'content-type: application/json' -d '{"bot_token":"777000:beta_test_token_0123456789"}' \
  "$base_url/api/merchant/bot_token")
[[ "$viewer_bot_status" == 403 ]]
rejected_bot_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' -d '{"bot_token":"777000:rejected_token_0123456789"}' \
  "$base_url/api/merchant/bot_token")
[[ "$rejected_bot_status" == 400 ]]
beta_bot_token=777000:beta_test_token_0123456789
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d "{\"bot_token\":\"$beta_bot_token\"}" "$base_url/api/merchant/bot_token" \
  | jq -e '.data.id == 777000 and .data.username == "beta_bot_777000" and .data.webhook_configured == true' >/dev/null
stored_bot_secret=$(psql "$database_url" -Atc \
  "SELECT secret_ref FROM tenant_secret_refs WHERE tenant_id = 'beta-local' AND secret_key = 'TELEGRAM_BOT_TOKEN'")
[[ "$stored_bot_secret" == enc:v1:* ]]
[[ "$stored_bot_secret" != *"$beta_bot_token"* ]]
curl -fsS "${headers[@]}" "$base_url/api/merchant/profile" \
  | jq -e '.data.bot_id == 777000 and .data.bot_username == "beta_bot_777000" and .data.bot_token == "exists"' >/dev/null
bot_list_merchant_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  "$base_url/telegram/webhook/bots")
[[ "$bot_list_merchant_status" == 403 ]]
curl -fsS "${platform_headers[@]}" "$base_url/telegram/webhook/bots" \
  | jq -e '.data | length == 1 and .[0].bot_id == 777000 and .[0].tenant_id == "beta-local"' >/dev/null
webhook_without_secret_status=$(curl -sS -o /dev/null -w '%{http_code}' \
  -H 'content-type: application/json' -d '{"update_id":1,"message":{"chat":{"id":123},"text":"beta"}}' \
  "$base_url/telegram/webhook/777000")
[[ "$webhook_without_secret_status" == 401 ]]
webhook_hash=$(printf 'telegram-webhook\0%s\0%s' 777000 "$beta_bot_token" \
  | openssl dgst -sha256 -hmac "$secret_encryption_key" -binary | xxd -p -c 256)
send_beta_webhook() {
  curl -fsS -H "x-telegram-bot-api-secret-token: wh_$webhook_hash" \
    -H 'content-type: application/json' -d "$1" \
    "$base_url/telegram/webhook/777000"
}
curl -fsS -H "x-telegram-bot-api-secret-token: wh_$webhook_hash" \
  -H 'content-type: application/json' -d '{"update_id":2,"message":{"chat":{"id":123},"text":"beta"}}' \
  "$base_url/telegram/webhook/777000" | jq -e '. == {}' >/dev/null
curl -fsS "${platform_headers[@]}" -H 'content-type: application/json' \
  -d '{"bot_id":777000,"update":{"update_id":3,"message":{"chat":{"id":123},"text":"dispatch beta"}}}' \
  "$base_url/telegram/webhook/dispatch-test" \
  | jq -e '.status == "ok" and .dispatched == true and .kind == "message"' >/dev/null

curl -fsS "${headers[@]}" "$base_url/api/users" \
  | jq -e '.total == 1 and .data[0].user_id == 30001 and .data[0].tenant_id == "beta-local"' >/dev/null
curl -fsS "${other_headers[@]}" "$base_url/api/users" \
  | jq -e '.total == 2 and ([.data[].tenant_id] | unique) == ["other-tenant"]' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/messages/30001?page=1" \
  | jq -e '.total == 1 and .data[0].content == "beta inbound" and .data[0].tg_msg_id == 450' >/dev/null
cross_chat_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  "$base_url/api/messages/40001")
[[ "$cross_chat_status" == 403 ]]
viewer_send_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" \
  -H 'content-type: application/json' -d '{"content":"viewer denied"}' \
  "$base_url/api/messages/30001")
[[ "$viewer_send_status" == 403 ]]
cross_send_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' -d '{"content":"cross denied"}' \
  "$base_url/api/messages/40001")
[[ "$cross_send_status" == 403 ]]
sent_text=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"content":"beta outgoing","reply_to_msg_id":450}' "$base_url/api/messages/30001")
sent_text_id=$(jq -er '.msg_id' <<<"$sent_text")
jq -e '.status == "ok" and .data.tg_msg_id == 501 and .msg_id == .data.msg_id' <<<"$sent_text" >/dev/null
psql "$database_url" -Atc \
  "SELECT from_userid = 777000 AND to_userid = 30001 AND tenant_id = 'beta-local' AND tg_msg_id = 501 AND quoted_msg_id = 450 AND quoted_text = 'beta inbound' FROM chat.messages WHERE id = $sent_text_id" \
  | grep -qx t
sent_photo=$(curl -fsS "${headers[@]}" \
  -F "image=@$image_fixture;type=image/png" -F 'content=beta image' -F 'reply_to_msg_id=450' \
  "$base_url/api/messages/30001")
jq -e '.status == "ok" and .data.tg_msg_id == 502 and .data.file_id == "beta-photo-file" and .msg_id == .data.msg_id' \
  <<<"$sent_photo" >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/file/beta-photo-file" \
  | jq -e '.status == "ok" and (.url | startswith("data:image/png;base64,")) and .url == .data.url' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/search?q=inbound" \
  | jq -e '.total == 1 and .data[0].content == "beta inbound"' >/dev/null
viewer_mark_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" \
  -X POST "$base_url/api/mark_read/30001")
[[ "$viewer_mark_status" == 403 ]]
curl -fsS "${headers[@]}" -X POST "$base_url/api/mark_read/30001" \
  | jq -e '.status == "ok"' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT unread_count FROM chat.admin_inbox WHERE user_id = 30001 AND tenant_id = 'beta-local'") == 0 ]]
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d "{\"msg_id\":$sent_text_id}" "$base_url/api/messages/30001/delete" \
  | jq -e '.status == "ok"' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT is_deleted FROM chat.messages WHERE id = $sent_text_id") == t ]]
curl -fsS "${headers[@]}" -X POST "$base_url/api/delete_chat/30001" \
  | jq -e '.status == "ok"' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.admin_inbox WHERE user_id = 30001 AND tenant_id = 'beta-local'") == 0 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.admin_inbox WHERE user_id = 30001 AND tenant_id = 'other-tenant'") == 1 ]]

curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"content":"alias beta","model":"beta-mock"}' "$base_url/api/messages/-99999/ai" \
  | jq -e '.reply == "mock:alias beta"' >/dev/null

curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"content":"hello beta","user_id":-99999,"model":"beta-mock"}' \
  "$base_url/api/ai/chat" | jq -e '.reply == "mock:hello beta"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/merchant/ai/usage?days=7" \
  | jq -e '.data.summary.total_requests == 2 and .data.summary.total_tokens == 14' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"model":"beta-mock","messages":[{"role":"user","content":"openai beta"}]}' \
  "$base_url/api/openai/chat/completions" \
  | jq -e '.choices[0].message.content == "mock:openai beta" and .usage.total_tokens == 7' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"model":"beta-mock","input":"exec beta"}' "$base_url/api/openai/exec" \
  | jq -e '.output == "mock:exec beta" and .output_text == "mock:exec beta" and .model == "beta-mock" and .sandbox_id == "default" and .usage.total_tokens == 7' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"text":"加我微信免费领取红包返利佣金","context_type":"group_message"}' \
  "$base_url/api/ai/detect_ad" \
  | jq -e '.is_ad == true and .ad_score >= 40' >/dev/null

curl -fsS "${headers[@]}" "$base_url/api/ai/skills" \
  | jq -e '.status == "ok" and .total == 0' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"name":"tenant-summary","description":"Tenant summary","prompt_template":"Summarize {{input}}","source":"custom","is_enabled":true}' \
  "$base_url/api/ai/skills" \
  | jq -e '.data.name == "tenant-summary" and .data.is_enabled == true' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"name":"tenant-summary","is_enabled":false}' "$base_url/api/ai/skills/toggle" \
  | jq -e '.data.name == "tenant-summary" and .data.is_enabled == false' >/dev/null
curl -fsS "${other_headers[@]}" "$base_url/api/ai/skills" \
  | jq -e '.total == 0' >/dev/null
other_skill_status=$(curl -sS -o /dev/null -w '%{http_code}' "${other_headers[@]}" \
  -H 'content-type: application/json' -d '{"name":"tenant-summary","is_enabled":true}' \
  "$base_url/api/ai/skills/toggle")
[[ "$other_skill_status" == 404 ]]
curl -fsS "${headers[@]}" "$base_url/api/ai/skills/discovered" \
  | jq -e '.total == 1 and .data[0].name == "beta-contract"' >/dev/null
curl -fsS "${headers[@]}" -X POST "$base_url/api/ai/skills/import_builtin" \
  | jq -e '.data.imported == 1 and .data.detected == 1' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/ai/skills" \
  | jq -e '.total == 1 and .data[0].name == "tenant-summary"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/ai/skills?include_builtin=1" \
  | jq -e '.total == 2 and ([.data[].name] | sort) == ["beta-contract","tenant-summary"]' >/dev/null

pipeline_write_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' -d '{"tenantId":"beta-local","mode":"legacy"}' \
  "$base_url/api/ai/pipeline_mode")
[[ "$pipeline_write_status" == 403 ]]
curl -fsS "${platform_headers[@]}" -H 'content-type: application/json' \
  -d '{"tenantId":"beta-local","mode":"legacy"}' "$base_url/api/ai/pipeline_mode" \
  | jq -e '.data.tenantId == "beta-local" and .data.mode == "legacy"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/ai/pipeline_mode" \
  | jq -e '.data.tenantId == "beta-local" and .data.mode == "legacy" and .data.modes == ["agent","legacy"]' >/dev/null

curl -fsS "${headers[@]}" "$base_url/api/group_listen/stats" \
  | jq -e '.total_groups == 2 and .monitored_groups == 1' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"chat_id":"-10999"}' "$base_url/api/group_listen/create_sync_chat" \
  | jq -e '.data.chat_id == -10999' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"chat_id":-10999,"group_ids":[-10001,-10002]}' \
  "$base_url/api/group_listen/sync_chat_assignments" \
  | jq -e '.data.assigned_group_ids == [-10002,-10001]' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_listen/sync_chat_info" \
  | jq -e '.data.chat_id == -10999 and (.data.assigned_groups | length) == 2' >/dev/null
assignment_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' -d '{"chat_id":-10999,"group_ids":[-20001]}' \
  "$base_url/api/group_listen/sync_chat_assignments")
[[ "$assignment_status" == 400 ]]

viewer_group_toggle_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" \
  -H 'content-type: application/json' -d '{"group_id":-10001,"enabled":false}' \
  "$base_url/api/group_listen/toggle")
[[ "$viewer_group_toggle_status" == 403 ]]
cross_tenant_toggle_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' -d '{"group_id":-20001,"enabled":false}' \
  "$base_url/api/group_listen/toggle")
[[ "$cross_tenant_toggle_status" == 404 ]]
[[ $(psql "$database_url" -Atc "SELECT is_monitored FROM groups.tracked_groups WHERE group_id = -20001 AND tenant_id = 'other-tenant'") == t ]]

curl -fsS -X DELETE http://127.0.0.1:18092/calls >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_photo?group_id=-10001" \
  | jq -e '.file_id == "beta-group-photo-small"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_member_count?group_id=-10001" \
  | jq -e '.count == 42' >/dev/null
cross_tenant_photo_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  "$base_url/api/group_photo?group_id=-20001")
[[ "$cross_tenant_photo_status" == 404 ]]

group_send=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"content":"beta group outbound","msg_type":"text"}' \
  "$base_url/api/group_messages/send")
jq -e '.data.tg_msg_id == 501' <<<"$group_send" >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE group_id = -10001 AND tenant_id = 'beta-local' AND tg_msg_id = 501 AND content = 'beta group outbound' AND sender_type = 'admin'") == 1 ]]
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"tg_msg_id":501,"user_id":777000}' \
  "$base_url/api/group_messages/delete" | jq -e '.data.tg_msg_id == 501' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE group_id = -10001 AND tenant_id = 'beta-local' AND tg_msg_id = 501") == 0 ]]

group_reply=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"tg_msg_id":601,"content":"beta group reply"}' \
  "$base_url/api/group_messages/reply")
jq -e '.data.tg_msg_id == 501' <<<"$group_reply" >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE group_id = -10001 AND tenant_id = 'beta-local' AND tg_msg_id = 501 AND quoted_tg_msg_id = 601 AND quoted_text = 'original beta group message' AND message_ts = 1700000000") == 1 ]]
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"tg_msg_id":501,"user_id":777000}' \
  "$base_url/api/group_messages/delete" >/dev/null
failed_group_send_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' -d '{"group_id":-10002,"content":"expected group failure"}' \
  "$base_url/api/group_messages/send")
[[ "$failed_group_send_status" != 200 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE group_id = -10002 AND tenant_id = 'beta-local' AND content = 'expected group failure'") == 0 ]]

viewer_reminder_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" \
  -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_ids":[30001],"channel_link":"https://t.me/beta_channel"}' \
  "$base_url/api/group_listen/send_channel_reminder")
[[ "$viewer_reminder_status" == 403 ]]
invalid_reminder_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_ids":[30001],"channel_link":"https://example.com/not-telegram"}' \
  "$base_url/api/group_listen/send_channel_reminder")
[[ "$invalid_reminder_status" == 400 ]]
reminder_result=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_ids":[30001,99999],"channel_link":"@beta_channel"}' \
  "$base_url/api/group_listen/send_channel_reminder")
jq -e '.sent_count == 1 and (.results | length) == 2 and any(.results[]; .user_id == 99999 and .error == "member_not_found")' \
  <<<"$reminder_result" >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.channel_reminders WHERE group_id = -10001 AND user_id = 30001 AND tenant_id = 'beta-local' AND expires_at > now() + interval '11 hours'") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.channel_reminders WHERE tenant_id = 'other-tenant'") == 0 ]]

curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_id":30001,"action":"mute","duration":300}' \
  "$base_url/api/group_members/action" >/dev/null
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE group_id = -10001 AND user_id = 30001 AND tenant_id = 'beta-local'") == muted ]]
curl -fsS -H "x-telegram-bot-api-secret-token: wh_$webhook_hash" \
  -H 'content-type: application/json' \
  -d '{"update_id":4,"callback_query":{"id":"beta-subscribe-ok","from":{"id":30001,"is_bot":false,"first_name":"Shared"},"message":{"message_id":501,"chat":{"id":-10001,"type":"supergroup"},"date":1700000000},"data":"subscribe_-10001_-30001"}}' \
  "$base_url/telegram/webhook/777000" | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT status = 'active' AND is_verified FROM groups.group_members WHERE group_id = -10001 AND user_id = 30001 AND tenant_id = 'beta-local'") == t ]]
[[ $(psql "$database_url" -Atc "SELECT is_deleted FROM groups.channel_reminders WHERE group_id = -10001 AND user_id = 30001 AND message_id = 501 AND tenant_id = 'beta-local'") == t ]]
curl -fsS -H "x-telegram-bot-api-secret-token: wh_$webhook_hash" \
  -H 'content-type: application/json' \
  -d '{"update_id":5,"callback_query":{"id":"beta-subscribe-cross-tenant","from":{"id":40001,"is_bot":false,"first_name":"Other"},"message":{"message_id":701,"chat":{"id":-20001,"type":"supergroup"},"date":1700000000},"data":"subscribe_-20001_-30001"}}' \
  "$base_url/telegram/webhook/777000" | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT is_verified FROM groups.group_members WHERE group_id = -20001 AND user_id = 40001 AND tenant_id = 'other-tenant'") == f ]]
admin_ban_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_id":30002,"action":"ban"}' \
  "$base_url/api/group_members/action")
[[ "$admin_ban_status" == 400 ]]
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE group_id = -10001 AND user_id = 30002 AND tenant_id = 'beta-local'") == admin ]]
creator_promote_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' -d '{"group_id":-10001,"user_id":30003}' \
  "$base_url/api/group_members/promote_admin")
[[ "$creator_promote_status" == 400 ]]
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_id":30001}' \
  "$base_url/api/group_members/promote_admin" >/dev/null
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE group_id = -10001 AND user_id = 30001 AND tenant_id = 'beta-local'") == admin ]]
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_id":30001}' \
  "$base_url/api/group_members/demote_admin" >/dev/null
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE group_id = -10001 AND user_id = 30001 AND tenant_id = 'beta-local'") == active ]]
batch_kick=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_ids":[30001,30004,99999],"action":"kick"}' \
  "$base_url/api/group_members/action_batch")
jq -e '.summary.total == 3 and .summary.success == 1 and .summary.failed == 2 and .success_ids == [30001]' \
  <<<"$batch_kick" >/dev/null
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE group_id = -10001 AND user_id = 30001 AND tenant_id = 'beta-local'") == kicked ]]
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE group_id = -10001 AND user_id = 30004 AND tenant_id = 'beta-local'") == active ]]
cross_tenant_member_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' \
  -d '{"group_id":-20001,"user_id":40001,"action":"kick"}' \
  "$base_url/api/group_members/action")
[[ "$cross_tenant_member_status" == 404 ]]

curl -fsS http://127.0.0.1:18092/calls | jq -e '
  any(.[]; .method == "getChat" and .chat_id == -10001) and
  any(.[]; .method == "getChatMemberCount" and .chat_id == -10001) and
  any(.[]; .method == "sendMessage" and .chat_id == -10001 and .reply_to_message_id == 601) and
  any(.[]; .method == "sendMessage" and .chat_id == -10001 and .reply_markup.inline_keyboard[0][0].url == "https://t.me/beta_channel") and
  any(.[]; .method == "sendMessage" and .chat_id == -10001 and .reply_markup.inline_keyboard[1][0].callback_data == "subscribe_-10001_-30001") and
  any(.[]; .method == "deleteMessage" and .chat_id == -10001 and .message_id == 501) and
  any(.[]; .method == "restrictChatMember" and .user_id == 30001 and .permissions.can_send_messages == false and (.until_date | type) == "number") and
  any(.[]; .method == "restrictChatMember" and .user_id == 30001 and .permissions.can_send_messages == true and .until_date == null) and
  any(.[]; .method == "getChatMember" and .chat_id == -30001 and .user_id == 30001) and
  any(.[]; .method == "answerCallbackQuery" and .callback_query_id == "beta-subscribe-ok" and .show_alert == true) and
  any(.[]; .method == "answerCallbackQuery" and .callback_query_id == "beta-subscribe-cross-tenant" and .show_alert == true) and
  ([.[] | select(.method == "restrictChatMember" and .chat_id == -20001)] | length == 0) and
  any(.[]; .method == "promoteChatMember" and .user_id == 30001 and .can_manage_chat == true) and
  any(.[]; .method == "promoteChatMember" and .user_id == 30001 and .can_manage_chat == false) and
  any(.[]; .method == "banChatMember" and .user_id == 30001) and
  any(.[]; .method == "unbanChatMember" and .user_id == 30001 and .only_if_banned == true)
' >/dev/null

viewer_broadcast_sync_status=$(curl -sS -o /dev/null -w '%{http_code}' \
  "${viewer_headers[@]}" -X POST "$base_url/api/broadcast/targets/sync")
[[ "$viewer_broadcast_sync_status" == 403 ]]
curl -fsS "${headers[@]}" -X POST "$base_url/api/broadcast/targets/sync" \
  | jq -e '.summary.total == 2 and .summary.synced == 2' >/dev/null
curl -fsS "${other_headers[@]}" -X POST "$base_url/api/broadcast/targets/sync" \
  | jq -e '.summary.total == 1 and .summary.synced == 1' >/dev/null
psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO broadcast.broadcast_targets
  (chat_id, chat_title, chat_type, can_broadcast, tenant_id)
VALUES (-10001, 'Shared ID Other Tenant', 'supergroup', true, 'other-tenant')
ON CONFLICT (chat_id, tenant_id) DO UPDATE SET chat_title = EXCLUDED.chat_title;
INSERT INTO chat.messages
  (from_userid, to_userid, content, msg_type, is_read, tg_msg_id, tenant_id)
VALUES (30001, 777000, 'broadcast eligibility', 'text', false, 650, 'beta-local');
SQL
curl -fsS "${headers[@]}" "$base_url/api/broadcast/targets" \
  | jq -e '.total == 2 and ([.data[].chat_id] | sort) == [-10002,-10001] and ([.data[].tenant_id] | unique) == ["beta-local"]' >/dev/null
curl -fsS "${other_headers[@]}" "$base_url/api/broadcast/targets" \
  | jq -e '.total == 2 and ([.data[].tenant_id] | unique) == ["other-tenant"]' >/dev/null

curl -fsS -X DELETE http://127.0.0.1:18092/calls >/dev/null
broadcast_result=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"content":"beta broadcast","user_ids":[30001],"chat_ids":[-10001,-20001],"buttons":[{"text":"Beta","url":"https://beta.example"}],"retry_count":0}' \
  "$base_url/api/broadcast/unified")
jq -e '.status == "ok" and .summary.total == 3 and .summary.success == 2 and .summary.failed == 1 and .summary.success_user_ids == [30001] and .summary.success_chat_ids == [-10001] and .summary.failed_items[0].chat_id == -20001' \
  <<<"$broadcast_result" >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 777000 AND to_userid = 30001 AND content = 'beta broadcast' AND status = 'sent' AND is_deleted = false") == 1 ]]
curl -fsS http://127.0.0.1:18092/calls \
  | jq -e 'length == 2 and all(.[]; .bot_id == 777000 and .method == "sendMessage" and .reply_markup.inline_keyboard[0][0].url == "https://beta.example")' >/dev/null

curl -fsS -X DELETE http://127.0.0.1:18092/calls >/dev/null
curl -fsS "${headers[@]}" \
  -F "image=@$image_fixture;type=image/png" \
  -F 'content=beta photo broadcast' \
  -F 'user_ids=[30001]' \
  -F 'chat_ids=[-10001]' \
  -F 'buttons=[{"text":"Open","url":"https://beta.example/open"}]' \
  -F 'retry_count=0' \
  "$base_url/api/broadcast/send" \
  | jq -e '.summary.total == 2 and .summary.success == 2 and .summary.failed == 0' >/dev/null
curl -fsS http://127.0.0.1:18092/calls \
  | jq -e 'length == 2 and all(.[]; .bot_id == 777000 and .method == "sendPhoto" and .multipart == true and .reply_markup.inline_keyboard[0][0].url == "https://beta.example/open")' >/dev/null

failed_broadcast=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"content":"expected Telegram failure","chat_ids":[-10002],"retry_count":0}' \
  "$base_url/api/broadcast/chats")
jq -e '.summary.total == 1 and .summary.success == 0 and .summary.failed == 1 and .summary.failed_items[0].chat_id == -10002' \
  <<<"$failed_broadcast" >/dev/null
viewer_broadcast_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" \
  -H 'content-type: application/json' -d '{"content":"denied","chat_ids":[-10001]}' \
  "$base_url/api/broadcast")
[[ "$viewer_broadcast_status" == 403 ]]

future_start=$(date -u -d '+1 hour' +%Y-%m-%dT%H:%M:%SZ)
push_task=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d "{\"task_name\":\"Manual Beta Push\",\"content\":\"manual push\",\"target_chat_ids\":[-10001],\"start_at\":\"$future_start\",\"interval_hours\":6}" \
  "$base_url/api/push_tasks")
push_task_id=$(jq -er '.task_id' <<<"$push_task")
curl -fsS "${headers[@]}" "$base_url/api/push_tasks" \
  | jq -e --argjson id "$push_task_id" '.total == 1 and .data[0].id == $id and .data[0].tenant_id == "beta-local"' >/dev/null
curl -fsS "${other_headers[@]}" "$base_url/api/push_tasks" | jq -e '.total == 0' >/dev/null
other_push_status=$(curl -sS -o /dev/null -w '%{http_code}' "${other_headers[@]}" -X PUT \
  -H 'content-type: application/json' -d "{\"task_id\":$push_task_id,\"status\":\"paused\"}" \
  "$base_url/api/push_tasks/status")
[[ "$other_push_status" == 404 ]]
viewer_push_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" -X PUT \
  -H 'content-type: application/json' -d "{\"task_id\":$push_task_id,\"status\":\"paused\"}" \
  "$base_url/api/push_tasks/status")
[[ "$viewer_push_status" == 403 ]]
curl -fsS "${headers[@]}" -X PUT -H 'content-type: application/json' \
  -d "{\"task_id\":$push_task_id,\"status\":\"paused\"}" "$base_url/api/push_tasks/status" \
  | jq -e '.status == "ok"' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d "{\"task_id\":$push_task_id}" "$base_url/api/push_tasks/run_now" \
  | jq -e '.summary.total == 1 and .summary.success == 1 and .summary.failed == 0' >/dev/null
invalid_push_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" \
  -H 'content-type: application/json' \
  -d "{\"task_name\":\"Cross Tenant\",\"content\":\"denied\",\"target_chat_ids\":[-20001],\"start_at\":\"$future_start\",\"interval_hours\":6}" \
  "$base_url/api/push_tasks")
[[ "$invalid_push_status" == 400 ]]

past_start=$(date -u -d '-1 second' +%Y-%m-%dT%H:%M:%SZ)
auto_push=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d "{\"task_name\":\"Scheduled Beta Push\",\"content\":\"scheduled push\",\"target_chat_ids\":[-10001],\"start_at\":\"$past_start\",\"interval_hours\":6}" \
  "$base_url/api/push_tasks")
auto_push_id=$(jq -er '.task_id' <<<"$auto_push")
scheduled_success=
for _ in $(seq 1 50); do
  scheduled_success=$(psql "$database_url" -Atc "SELECT last_run_summary ->> 'success' FROM broadcast.push_tasks WHERE tenant_id = 'beta-local' AND id = $auto_push_id")
  [[ "$scheduled_success" == 1 ]] && break
  sleep 0.2
done
[[ "$scheduled_success" == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM job_queue WHERE tenant_id = 'beta-local' AND job_type = 'push_task' AND payload ->> 'push_task_id' = '$auto_push_id' AND status = 'completed'") == 1 ]]
curl -fsS http://127.0.0.1:18092/calls \
  | jq -e 'any(.[]; .bot_id == 777000 and .method == "sendMessage" and .chat_id == -10001 and .text == "scheduled push")' >/dev/null
curl -fsS "${headers[@]}" -X PUT -H 'content-type: application/json' \
  -d "{\"task_id\":$auto_push_id,\"status\":\"paused\"}" "$base_url/api/push_tasks/status" >/dev/null

monitor=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"address":"TBetaAddress001","network":"TRC20","remark":"before"}' \
  "$finance_url/monitors")
monitor_id=$(jq -er '.data.monitor_id' <<<"$monitor")

curl -fsS "${headers[@]}" -X PATCH -H 'content-type: application/json' \
  -d '{"remark":"after"}' "$finance_url/monitors/$monitor_id" \
  | jq -e '.data.remark == "after"' >/dev/null
curl -fsS "${headers[@]}" "$finance_url/monitors" \
  | jq -e '.data[0].inflow_20 == 0' >/dev/null

now_ms=$(date +%s%3N)
psql "$database_url" -v ON_ERROR_STOP=1 -v monitor_id="$monitor_id" -v now_ms="$now_ms" <<'SQL'
INSERT INTO monitor_transactions
(tx_id, monitor_id, user_id, tenant_id, address, network, direction, amount, token_symbol, tx_timestamp, counterparty)
VALUES
('beta-tx-in', :monitor_id, 10001, 'beta-local', 'TBetaAddress001', 'TRC20', 'in', 125.50, 'USDT', :now_ms, 'TCounterparty001'),
('beta-tx-out', :monitor_id, 10001, 'beta-local', 'TBetaAddress001', 'TRC20', 'out', 25.25, 'USDT', :now_ms - 1000, 'TCounterparty002'),
('other-tx', :monitor_id, 20001, 'other-tenant', 'TOtherAddress', 'TRC20', 'in', 999, 'USDT', :now_ms, 'TOtherCounterparty');
INSERT INTO finance_ledgers
(ledger_id, monitor_id, user_id, tenant_id, tx_id, direction, amount, token_symbol, network, base_fiat_value, fiat_currency, counterparty, tx_timestamp)
VALUES
('33333333-3333-4333-8333-333333333333', :monitor_id, 10001, 'beta-local', 'beta-tx-in', 'in', 125.50, 'USDT', 'TRC20', 125.50, 'USD', 'TCounterparty001', :now_ms),
('44444444-4444-4444-8444-444444444444', :monitor_id, 10001, 'beta-local', 'beta-tx-out', 'out', 25.25, 'USDT', 'TRC20', 25.25, 'USD', 'TCounterparty002', :now_ms - 1000);
SQL

curl -fsS "${headers[@]}" "$finance_url/transactions?period=month" \
  | jq -e '.count == 2 and ([.data[].amount] | sort == [25.25, 125.5])' >/dev/null
curl -fsS "${headers[@]}" -X PATCH -H 'content-type: application/json' \
  -d '{"income_reason":"beta income","counterparty_label":"Beta Customer"}' \
  "$finance_url/transactions/beta-tx-in/meta" \
  | jq -e '.updated == 1' >/dev/null

other_status=$(curl -sS -o /dev/null -w '%{http_code}' "${headers[@]}" -X PATCH \
  -H 'content-type: application/json' -d '{"income_reason":"forbidden"}' \
  "$finance_url/transactions/other-tx/meta")
[[ "$other_status" == 404 ]]

curl -fsS "${headers[@]}" -X PUT -H 'content-type: application/json' \
  -d '{"transactions":[{"tx_id":"beta-tx-out","expense_reason":"beta expense","payee":"Beta Vendor"},{"tx_id":"other-tx","income_reason":"forbidden"}]}' \
  "$finance_url/transactions/meta/batch" | jq -e '.updated == 1' >/dev/null
curl -fsS "${headers[@]}" -X PUT -H 'content-type: application/json' \
  -d '{"counterparty":"TCounterparty002","network":"TRC20","label":"Beta Vendor"}' \
  "$finance_url/counterparty_labels" | jq -e '.status == "ok"' >/dev/null

curl -fsS "${headers[@]}" "$finance_url/summary?period=month" \
  | jq -e '.data.total_inflow == 125.5 and .data.total_outflow == 25.25 and .data.total_unreconciled == 2' >/dev/null
curl -fsS "${headers[@]}" "$finance_url/unreconciled" \
  | jq -e '.count == 2' >/dev/null
curl -fsS "${headers[@]}" "$finance_url/tokens" \
  | jq -e '.data == ["USDT"]' >/dev/null
curl -fsS "${headers[@]}" "$finance_url/account_categories" \
  | jq -e '[.. | objects | select(.code? == "4100")] | length == 1' >/dev/null

curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"ledger_id":"33333333-3333-4333-8333-333333333333","account_code":"4100"}' \
  "$finance_url/reconcile" | jq -e '.status == "ok"' >/dev/null
curl -fsS "${headers[@]}" "$finance_url/audit_logs" \
  | jq -e '.data[0].action == "RECONCILE"' >/dev/null

curl -fsS "${headers[@]}" "$finance_url/reports/generate?month=$(date +%Y-%m)&format=pdf" -o "$report_pdf"
curl -fsS "${headers[@]}" "$finance_url/reports/generate?month=$(date +%Y-%m)&format=excel" -o "$report_xlsx"
[[ $(head -c 8 "$report_pdf") == '%PDF-1.4' ]]
[[ $(od -An -tx1 -N4 "$report_xlsx" | tr -d ' \n') == 504b0304 ]]
unzip -t "$report_xlsx" >/dev/null

curl -fsS "${headers[@]}" -X DELETE "$finance_url/monitors/$monitor_id" \
  | jq -e '.status == "ok"' >/dev/null

legacy_delete=$(curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"address":"TBetaLegacyDelete","network":"TRC20","remark":"delete me"}' \
  "$base_url/api/monitors")
legacy_delete_id=$(jq -er '.data.monitor_id' <<<"$legacy_delete")
curl -fsS "${headers[@]}" -X DELETE -H 'content-type: application/json' \
  -d "{\"monitor_id\":$legacy_delete_id}" "$base_url/api/monitors" \
  | jq -e '.status == "ok"' >/dev/null

send_beta_webhook '{"update_id":100,"message":{"message_id":801,"from":{"id":30005,"is_bot":false,"first_name":"新用户","last_name":"测试","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000100,"text":"这是一条超过二十个中文字符并用于验证安全摘要截断的消息内容"}}' \
  | jq -e '. == {}' >/dev/null
send_beta_webhook '{"update_id":101,"message":{"message_id":801,"from":{"id":30005,"is_bot":false,"first_name":"新用户","last_name":"测试","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000100,"text":"这是一条超过二十个中文字符并用于验证安全摘要截断的消息内容"}}' \
  | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 30005 AND to_userid = 777000 AND tg_msg_id = 801") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT username = 'beta_inbound' AND unread_count = 1 FROM chat.admin_inbox WHERE tenant_id = 'beta-local' AND user_id = 30005") == t ]]
send_beta_webhook '{"update_id":102,"edited_message":{"message_id":801,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000110,"text":"已编辑的私聊消息"}}' \
  | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT content = '已编辑的私聊消息' AND status = 'edited' AND original_text IS NOT NULL FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 30005 AND tg_msg_id = 801") == t ]]

curl -fsS -X DELETE http://127.0.0.1:18092/calls >/dev/null
send_beta_webhook '{"update_id":112,"message":{"message_id":805,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000180,"text":"/start@beta_bot_777000 ref_xyz"}}' \
  | jq -e '. == {}' >/dev/null
send_beta_webhook '{"update_id":113,"message":{"message_id":805,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000180,"text":"/start@beta_bot_777000 ref_xyz"}}' \
  | jq -e '. == {}' >/dev/null
command_job_status=
for _ in $(seq 1 50); do
  command_job_status=$(psql "$database_url" -Atc "SELECT queued.status FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_command_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 805")
  [[ "$command_job_status" == completed ]] && break
  sleep 0.1
done
[[ "$command_job_status" == completed ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_command_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 805") == 1 ]]
curl -fsS http://127.0.0.1:18092/calls | jq -e '
  length == 1 and
  .[0].bot_id == 777000 and
  .[0].method == "sendMessage" and
  .[0].chat_id == 30005 and
  .[0].reply_to_message_id == 805 and
  (.[0].text | contains("新用户") and contains("/help")) and
  .[0].reply_markup == null
' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 30005 AND to_userid = 777000 AND tg_msg_id = 805") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 777000 AND to_userid = 30005 AND quoted_msg_id = 805 AND is_read = true AND content LIKE '%/help%'") == 1 ]]
send_beta_webhook '{"update_id":114,"message":{"message_id":806,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000190,"text":"/help@beta_bot_777000"}}' \
  | jq -e '. == {}' >/dev/null
send_beta_webhook '{"update_id":115,"message":{"message_id":807,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000200,"text":"/start@another_bot"}}' \
  | jq -e '. == {}' >/dev/null
send_beta_webhook '{"update_id":116,"message":{"message_id":808,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"date":1700000210,"text":"/start"}}' \
  | jq -e '. == {}' >/dev/null
completed_command_jobs=0
for _ in $(seq 1 50); do
  completed_command_jobs=$(psql "$database_url" -Atc "SELECT count(*) FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_command_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id IN (806, 807) AND queued.status = 'completed'")
  [[ "$completed_command_jobs" == 2 ]] && break
  sleep 0.1
done
[[ "$completed_command_jobs" == 2 ]]
curl -fsS http://127.0.0.1:18092/calls | jq -e '
  length == 2 and
  .[1].bot_id == 777000 and
  .[1].method == "sendMessage" and
  .[1].chat_id == 30005 and
  .[1].reply_to_message_id == 806 and
  (.[1].text | contains("/start") and contains("/help"))
' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 777000 AND to_userid = 30005 AND quoted_msg_id = 806 AND is_read = true") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.job_type = 'telegram_command_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 808") == 0 ]]

psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO job_queue (tenant_id, job_type, payload, priority, max_attempts, status)
SELECT
  'other-tenant',
  'telegram_command_reply',
  jsonb_build_object('tenant_id', 'beta-local', 'inbound_message_id', id),
  10,
  2,
  'pending'
FROM chat.messages
WHERE tenant_id = 'beta-local' AND from_userid = 30005 AND tg_msg_id = 805;
SQL
tenant_mismatch_status=
for _ in $(seq 1 50); do
  tenant_mismatch_status=$(psql "$database_url" -Atc "SELECT status || ':' || attempts || ':' || COALESCE(last_error, '') FROM job_queue WHERE tenant_id = 'other-tenant' AND job_type = 'telegram_command_reply'")
  [[ "$tenant_mismatch_status" == "pending:1:telegram command job tenant mismatch" ]] && break
  sleep 0.1
done
[[ "$tenant_mismatch_status" == "pending:1:telegram command job tenant mismatch" ]]
curl -fsS http://127.0.0.1:18092/calls | jq -e 'length == 2' >/dev/null

send_beta_webhook '{"update_id":117,"message":{"message_id":809,"from":{"id":30008,"is_bot":false,"first_name":"失败用户","username":"beta_command_failure"},"chat":{"id":30008,"type":"private"},"date":1700000220,"text":"/help"}}' \
  | jq -e '. == {}' >/dev/null
failed_command_status=
for _ in $(seq 1 50); do
  failed_command_status=$(psql "$database_url" -Atc "SELECT queued.status || ':' || queued.attempts || ':' || COALESCE(queued.last_error, '') FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_command_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 809")
  [[ "$failed_command_status" == "pending:1:failed to deliver Telegram command reply" ]] && break
  sleep 0.1
done
[[ "$failed_command_status" == "pending:1:failed to deliver Telegram command reply" ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 777000 AND to_userid = 30008 AND quoted_msg_id = 809") == 0 ]]
[[ $(psql "$database_url" -Atc "SELECT bool_and(payload ? 'tenant_id' AND payload ? 'inbound_message_id' AND payload - 'tenant_id' - 'inbound_message_id' = '{}'::jsonb) FROM job_queue WHERE tenant_id = 'beta-local' AND job_type = 'telegram_command_reply'") == t ]]
curl -fsS http://127.0.0.1:18092/calls | jq -e '
  length == 3 and
  .[2].bot_id == 777000 and
  .[2].method == "sendMessage" and
  .[2].chat_id == 30008 and
  .[2].reply_to_message_id == 809
' >/dev/null

send_beta_webhook '{"update_id":118,"message":{"message_id":810,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000230,"text":"营业时间"}}' \
  | jq -e '. == {}' >/dev/null
send_beta_webhook '{"update_id":119,"message":{"message_id":810,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":30005,"type":"private"},"date":1700000230,"text":"营业时间"}}' \
  | jq -e '. == {}' >/dev/null
auto_reply_job_status=
for _ in $(seq 1 50); do
  auto_reply_job_status=$(psql "$database_url" -Atc "SELECT queued.status FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_auto_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 810")
  [[ "$auto_reply_job_status" == completed ]] && break
  sleep 0.1
done
[[ "$auto_reply_job_status" == completed ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_auto_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 810") == 1 ]]
curl -fsS http://127.0.0.1:18092/calls | jq -e '
  [.[] | select(.method == "sendMessage" and .reply_to_message_id == 810)] as $calls |
  ($calls | length) == 1 and
  $calls[0].bot_id == 777000 and
  $calls[0].chat_id == 30005 and
  $calls[0].text == "Beta 营业时间：周一至周五" and
  $calls[0].reply_markup == null
' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 777000 AND to_userid = 30005 AND quoted_msg_id = 810 AND is_read = true AND content = 'Beta 营业时间：周一至周五'") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND content = 'Other tenant reply'") == 0 ]]

send_beta_webhook '{"update_id":120,"message":{"message_id":811,"from":{"id":30005,"is_bot":false,"first_name":"新用户"},"chat":{"id":30005,"type":"private"},"date":1700000240,"text":"没有匹配规则"}}' \
  | jq -e '. == {}' >/dev/null
send_beta_webhook '{"update_id":121,"message":{"message_id":812,"from":{"id":30005,"is_bot":false,"first_name":"新用户"},"chat":{"id":30005,"type":"private"},"date":1700000250,"text":"/营业时间"}}' \
  | jq -e '. == {}' >/dev/null
unmatched_auto_reply_status=
for _ in $(seq 1 50); do
  unmatched_auto_reply_status=$(psql "$database_url" -Atc "SELECT queued.status FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_auto_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 811")
  [[ "$unmatched_auto_reply_status" == completed ]] && break
  sleep 0.1
done
[[ "$unmatched_auto_reply_status" == completed ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.job_type = 'telegram_auto_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 812") == 0 ]]
curl -fsS http://127.0.0.1:18092/calls | jq -e '[.[] | select(.reply_to_message_id == 811 or .reply_to_message_id == 812)] | length == 0' >/dev/null

curl -fsS "${headers[@]}" -X PUT -H 'content-type: application/json' \
  -d '{"enabled":false,"rules":[]}' "$auto_reply_url" \
  | jq -e '.data == {"enabled":false,"rules":[]}' >/dev/null
send_beta_webhook '{"update_id":122,"message":{"message_id":813,"from":{"id":30005,"is_bot":false,"first_name":"新用户"},"chat":{"id":30005,"type":"private"},"date":1700000260,"text":"营业时间"}}' \
  | jq -e '. == {}' >/dev/null
disabled_auto_reply_status=
for _ in $(seq 1 50); do
  disabled_auto_reply_status=$(psql "$database_url" -Atc "SELECT queued.status FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_auto_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 813")
  [[ "$disabled_auto_reply_status" == completed ]] && break
  sleep 0.1
done
[[ "$disabled_auto_reply_status" == completed ]]
curl -fsS http://127.0.0.1:18092/calls | jq -e '[.[] | select(.reply_to_message_id == 813)] | length == 0' >/dev/null
curl -fsS "${headers[@]}" -X PUT -H 'content-type: application/json' \
  -d "$beta_auto_reply_config" "$auto_reply_url" \
  | jq -e '.data.enabled == true' >/dev/null

psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO job_queue (tenant_id, job_type, payload, priority, max_attempts, status)
SELECT
  'other-tenant',
  'telegram_auto_reply',
  jsonb_build_object('tenant_id', 'beta-local', 'inbound_message_id', id),
  10,
  2,
  'pending'
FROM chat.messages
WHERE tenant_id = 'beta-local' AND from_userid = 30005 AND tg_msg_id = 810;
SQL
auto_reply_tenant_mismatch_status=
for _ in $(seq 1 50); do
  auto_reply_tenant_mismatch_status=$(psql "$database_url" -Atc "SELECT status || ':' || attempts || ':' || COALESCE(last_error, '') FROM job_queue WHERE tenant_id = 'other-tenant' AND job_type = 'telegram_auto_reply'")
  [[ "$auto_reply_tenant_mismatch_status" == "pending:1:telegram auto-reply job tenant mismatch" ]] && break
  sleep 0.1
done
[[ "$auto_reply_tenant_mismatch_status" == "pending:1:telegram auto-reply job tenant mismatch" ]]

send_beta_webhook '{"update_id":123,"message":{"message_id":814,"from":{"id":30009,"is_bot":false,"first_name":"失败用户","username":"beta_auto_reply_failure"},"chat":{"id":30009,"type":"private"},"date":1700000270,"text":"失败测试"}}' \
  | jq -e '. == {}' >/dev/null
failed_auto_reply_status=
for _ in $(seq 1 50); do
  failed_auto_reply_status=$(psql "$database_url" -Atc "SELECT queued.status || ':' || queued.attempts || ':' || COALESCE(queued.last_error, '') FROM job_queue queued JOIN chat.messages inbound ON inbound.id = (queued.payload ->> 'inbound_message_id')::bigint WHERE queued.tenant_id = 'beta-local' AND queued.job_type = 'telegram_auto_reply' AND inbound.tenant_id = 'beta-local' AND inbound.tg_msg_id = 814")
  [[ "$failed_auto_reply_status" == "pending:1:failed to deliver Telegram auto-reply" ]] && break
  sleep 0.1
done
[[ "$failed_auto_reply_status" == "pending:1:failed to deliver Telegram auto-reply" ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 777000 AND to_userid = 30009 AND quoted_msg_id = 814") == 0 ]]
[[ $(psql "$database_url" -Atc "SELECT bool_and(payload ? 'tenant_id' AND payload ? 'inbound_message_id' AND payload - 'tenant_id' - 'inbound_message_id' = '{}'::jsonb) FROM job_queue WHERE tenant_id = 'beta-local' AND job_type = 'telegram_auto_reply'") == t ]]
curl -fsS http://127.0.0.1:18092/calls | jq -e '[.[] | select(.method == "sendMessage" and .reply_to_message_id == 814 and .chat_id == 30009)] | length >= 1' >/dev/null

send_beta_webhook '{"update_id":103,"message":{"message_id":802,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"date":1700000120,"caption":"群图片消息","photo":[{"file_id":"small-photo","width":10,"height":10},{"file_id":"large-photo","width":100,"height":100}]}}' \
  | jq -e '. == {}' >/dev/null
send_beta_webhook '{"update_id":104,"message":{"message_id":802,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"date":1700000120,"caption":"群图片消息","photo":[{"file_id":"small-photo"},{"file_id":"large-photo"}]}}' \
  | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE tenant_id = 'beta-local' AND group_id = -10001 AND tg_msg_id = 802 AND msg_type = 'photo' AND file_id = 'large-photo'") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_members WHERE tenant_id = 'beta-local' AND group_id = -10001 AND user_id = 30005 AND username = 'beta_inbound'") == 1 ]]
send_beta_webhook '{"update_id":105,"edited_message":{"message_id":802,"from":{"id":30005,"is_bot":false,"first_name":"新用户","username":"beta_inbound"},"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"date":1700000130,"text":"已编辑的群消息"}}' \
  | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT content = '已编辑的群消息' AND msg_type = 'text' FROM groups.group_messages WHERE tenant_id = 'beta-local' AND group_id = -10001 AND tg_msg_id = 802") == t ]]

send_beta_webhook '{"update_id":106,"chat_join_request":{"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"from":{"id":30006,"is_bot":false,"first_name":"Pending","username":"beta_pending"},"date":1700000140}}' \
  | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE tenant_id = 'beta-local' AND group_id = -10001 AND user_id = 30006") == pending ]]
send_beta_webhook '{"update_id":107,"chat_member":{"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"from":{"id":30003,"is_bot":false,"first_name":"Creator"},"date":1700000150,"old_chat_member":{"status":"left","user":{"id":30006,"is_bot":false,"first_name":"Pending"}},"new_chat_member":{"status":"administrator","user":{"id":30006,"is_bot":false,"first_name":"Pending","username":"beta_pending"}}}}' \
  | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE tenant_id = 'beta-local' AND group_id = -10001 AND user_id = 30006") == admin ]]
send_beta_webhook '{"update_id":108,"chat_member":{"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"from":{"id":30003,"is_bot":false,"first_name":"Creator"},"date":1700000155,"old_chat_member":{"status":"administrator","user":{"id":30006,"is_bot":false,"first_name":"Pending"}},"new_chat_member":{"status":"restricted","is_member":false,"user":{"id":30006,"is_bot":false,"first_name":"Pending","username":"beta_pending"}}}}' \
  | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT status FROM groups.group_members WHERE tenant_id = 'beta-local' AND group_id = -10001 AND user_id = 30006") == left ]]

send_beta_webhook '{"update_id":109,"message":{"message_id":803,"from":{"id":40001,"is_bot":false,"first_name":"Other"},"chat":{"id":-20001,"type":"supergroup","title":"Other Group"},"date":1700000160,"text":"cross tenant webhook denied"}}' \
  | jq -e '. == {}' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE group_id = -20001 AND tenant_id = 'other-tenant' AND tg_msg_id = 803") == 0 ]]

send_beta_webhook '{"update_id":110,"message":{"message_id":804,"from":{"id":30005,"is_bot":false,"first_name":"新用户","last_name":"测试","username":"beta_inbound"},"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"date":1700000170,"text":"我有定制需求，想谈合作，长期稳定货源，联系 @beta_sales"}}' \
  | jq -e '. == {}' >/dev/null
send_beta_webhook '{"update_id":111,"message":{"message_id":804,"from":{"id":30005,"is_bot":false,"first_name":"新用户","last_name":"测试","username":"beta_inbound"},"chat":{"id":-10001,"type":"supergroup","title":"Beta Group One"},"date":1700000170,"text":"我有定制需求，想谈合作，长期稳定货源，联系 @beta_sales"}}' \
  | jq -e '. == {}' >/dev/null
for _ in $(seq 1 50); do
  policy_job_status=$(psql "$database_url" -Atc "SELECT status FROM job_queue WHERE tenant_id = 'beta-local' AND job_type = 'group_message_policy' AND payload ->> 'group_id' = '-10001' AND payload ->> 'telegram_message_id' = '804' ORDER BY id DESC LIMIT 1")
  [[ "$policy_job_status" == completed ]] && break
  sleep 0.1
done
[[ "$policy_job_status" == completed ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE tenant_id = 'beta-local' AND group_id = -10001 AND tg_msg_id = 804 AND is_advertising = true AND ad_score >= 40 AND jsonb_array_length(ad_findings::jsonb) > 0") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) = 1 AND bool_and(keywords LIKE '%合作%' AND keywords LIKE '%定制需求%' AND profile_tags LIKE '%合作意向%') FROM tenant_saas.potential_users WHERE tenant_id = 'beta-local' AND user_id = 30005") == t ]]
curl -fsS "${headers[@]}" "$base_url/api/potential_users?tag=%E5%90%88%E4%BD%9C%E6%84%8F%E5%90%91&keyword=beta_inbound" \
  | jq -e '.total == 1 and (.data | length) == 1 and .data[0].group_title == "Beta Group One" and .data[0].last_name == "测试"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/potential_users/tags" \
  | jq -e '.data | index("合作意向") != null' >/dev/null

psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO tenant_saas.potential_users
  (tenant_id, user_id, username, message_text, keywords, profile_tags, group_id, group_title)
VALUES
  ('other-tenant', 30005, 'other_same_user', 'other lead', 'other', 'other-tag', -20001, 'Other Group');
SQL
curl -fsS "${other_headers[@]}" "$base_url/api/potential_users" \
  | jq -e '.total == 1 and .data[0].username == "other_same_user"' >/dev/null
curl -fsS "${headers[@]}" -X POST "$base_url/api/potential_users/30005/promote" \
  | jq -e '.status == "ok"' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM tenant_saas.potential_users WHERE tenant_id = 'beta-local' AND user_id = 30005") == 0 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM tenant_saas.potential_users WHERE tenant_id = 'other-tenant' AND user_id = 30005") == 1 ]]
curl -fsS "${headers[@]}" -X POST "$base_url/api/potential_users/sync" \
  | jq -e '.status == "ok" and .synced >= 1' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM tenant_saas.potential_users WHERE tenant_id = 'beta-local' AND user_id = 30005") == 1 ]]
viewer_potential_sync_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" -X POST "$base_url/api/potential_users/sync")
[[ "$viewer_potential_sync_status" == 403 ]]
viewer_potential_promote_status=$(curl -sS -o /dev/null -w '%{http_code}' "${viewer_headers[@]}" -X POST "$base_url/api/potential_users/30005/promote")
[[ "$viewer_potential_promote_status" == 403 ]]

curl -fsS -X DELETE http://127.0.0.1:18092/calls >/dev/null
psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO groups.channel_reminders
  (group_id, user_id, message_id, expires_at, tenant_id)
VALUES
  (-10001, 30002, 880, now() - interval '1 minute', 'beta-local'),
  (-20001, 40001, 881, now() - interval '1 minute', 'other-tenant');
SQL
expired_reminder_deleted=
for _ in $(seq 1 30); do
  expired_reminder_deleted=$(psql "$database_url" -Atc "SELECT is_deleted FROM groups.channel_reminders WHERE tenant_id = 'beta-local' AND message_id = 880")
  [[ "$expired_reminder_deleted" == t ]] && break
  sleep 0.2
done
[[ "$expired_reminder_deleted" == t ]]
[[ $(psql "$database_url" -Atc "SELECT is_deleted = false AND delete_attempts = 1 AND delete_retry_at > now() FROM groups.channel_reminders WHERE tenant_id = 'other-tenant' AND message_id = 881") == t ]]
curl -fsS http://127.0.0.1:18092/calls \
  | jq -e 'any(.[]; .bot_id == 777000 and .method == "deleteMessage" and .chat_id == -10001 and .message_id == 880) and ([.[] | select(.method == "deleteMessage" and .chat_id == -20001)] | length == 0)' >/dev/null

echo "Beta runtime gates passed: migrations, tenant-isolated webhook ingestion and chat, private commands and tenant auto-replies, group management and broadcast delivery, reminder cleanup, scheduled push tasks, finance, AI, config, Telegram secrets/webhooks, and group assignment APIs."
