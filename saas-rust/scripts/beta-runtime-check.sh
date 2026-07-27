#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
database_name=tgbot_rust_beta
# Keep the runtime gate independent of a host PostgreSQL service and sudo.  The
# CI/container profile runs without a setuid sudo helper, so each invocation
# owns a disposable, trust-authenticated cluster under /tmp.  The application
# still connects as the non-superuser database owner (ubuntu), while a separate
# bootstrap role is used only for database/role setup and RLS assertions.
pg_bin_dir=${PG_BIN_DIR:-/usr/lib/postgresql/16/bin}
pg_port=${BETA_RUNTIME_PG_PORT:-15432}
pg_admin_user=beta_pg_admin
pg_data_dir=$(mktemp -d)
pg_socket_dir=$(mktemp -d)
pg_log=$(mktemp)
pg_ctl_bin="$pg_bin_dir/pg_ctl"
initdb_bin="$pg_bin_dir/initdb"
database_url="postgresql://ubuntu@127.0.0.1:${pg_port}/${database_name}"
pg_admin_url="postgresql://${pg_admin_user}@127.0.0.1:${pg_port}/postgres"
pg_admin_database_url="postgresql://${pg_admin_user}@127.0.0.1:${pg_port}/${database_name}"
bind_address=127.0.0.1:18090
base_url="http://$bind_address"
runtime_report_dir=${BETA_REPORT_DIR:-"$rust_root/target/beta-reports"}
runtime_report_path="$runtime_report_dir/runtime-gate-report.json"
runtime_contract_report_path="$runtime_report_dir/frontend-runtime-contract-report.json"
runtime_request_trace_path="$runtime_report_dir/frontend-runtime-request-trace.tsv"
server_log=$(mktemp)
report_pdf=$(mktemp)
report_xlsx=$(mktemp)
image_fixture=$(mktemp --suffix=.png)
contract_response=$(mktemp)
runtime_request_log=$(mktemp)
server_pid=
llm_pid=
telegram_pid=
pg_started=
runtime_contract_matrix_status=not_run
runtime_contract_static_only_cases=0
runtime_contract_runtime_seen=0

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
  if [[ -n "$pg_started" ]]; then
    "$pg_ctl_bin" -D "$pg_data_dir" -m fast -w stop >/dev/null 2>&1 || true
  fi
  rm -f "$server_log" "$report_pdf" "$report_xlsx" "$image_fixture" "$contract_response" "$pg_log" "$runtime_request_log"
  rm -rf "$pg_data_dir" "$pg_socket_dir"
}
trap cleanup EXIT

write_runtime_report() {
  local status="$1"
  local failure_line="${2:-}"
  local head_commit
  head_commit=$(git -C "$rust_root" rev-parse HEAD 2>/dev/null || printf '%s' unknown)
  mkdir -p "$runtime_report_dir" 2>/dev/null || return 0
  cat >"$runtime_report_path" <<EOF
{
  "report_type": "beta_runtime_gate",
  "status": "$status",
  "generated_at_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "head_commit": "$head_commit",
  "database": {
    "mode": "disposable_local_postgresql",
    "host": "127.0.0.1",
    "port": $pg_port,
    "database": "$database_name",
    "application_role": "ubuntu",
    "bootstrap_role": "$pg_admin_user",
    "production_credentials_used": false
  },
  "migrations": {
    "passes": 2,
    "files": ["0020_group_messages_ad_columns.sql", "0021_ad_fingerprints.sql", "0022_ad_governance.sql", "0023_ad_delete_snapshots.sql", "0024_fix_rls_recursion.sql", "0025_realtime_ad_scan_outbox.sql", "0026_ad_governance_rls.sql"],
    "assertions": ["governance tables and nine tenant RLS policies", "least-privilege helper function grants", "realtime_ad_scan_outbox trigger", "non-owner cross-tenant read/write denial"]
  },
  "runtime_e2e": ["health and worker startup", "tenant-isolated webhook ingestion", "private commands and auto-replies", "chat and file delivery", "group management and reminders", "broadcast delivery", "scheduled push", "finance", "AI and skills", "platform configuration", "Telegram secrets and webhook dispatch", "realtime ad scan enqueue and worker completion", "legacy advertisement event persistence and backfill", "legacy advertisement listing and fail-closed lookup", "potential-user isolation"],
  "frontend_contracts": {
    "method_path_contracts": 102,
    "static_registration_report": "target/beta-reports/api-static-registration.json",
    "runtime_contract_report": "target/beta-reports/frontend-runtime-contract-report.json",
    "runtime_request_trace": "target/beta-reports/frontend-runtime-request-trace.tsv",
    "runtime_request_seen": $runtime_contract_runtime_seen,
    "static_only_cases": $runtime_contract_static_only_cases,
    "runtime_matrix_status": "$runtime_contract_matrix_status",
    "runtime_matrix_assertions": ["success and expected error status codes", "response status/data shape", "owner/viewer/platform authorization", "cross-tenant isolation", "ephemeral payment, monitor, and merchant lifecycle"],
    "production_credentials_used": false
  },
  "failure_line": "$failure_line"
}
EOF
}

report_failure() {
  local exit_status=$?
  local failure_line="${BASH_LINENO[0]}"
  local failure_command="${BASH_COMMAND:-unknown}"
  echo "Beta runtime gate failed at line ${failure_line} (exit $exit_status): ${failure_command}" >&2
  write_runtime_report failed "${failure_line}: ${failure_command}"
  exit "$exit_status"
}
trap report_failure ERR

for command in base64 cargo curl date jq node openssl psql dropdb createdb unzip xxd; do
  command -v "$command" >/dev/null || {
    echo "Missing required command: $command" >&2
    exit 1
  }
done
for command in "$initdb_bin" "$pg_ctl_bin"; do
  [[ -x "$command" ]] || {
    echo "Missing executable PostgreSQL helper: $command" >&2
    exit 1
  }
done

# Record every local HTTP method/path exercised by this gate without recording
# request bodies, headers, tokens, or response data. The persistent contract
# report is derived from this trace and the static frontend registration report.
curl_binary=$(command -v curl)
curl() {
  local method=GET
  local explicit_method=false
  local request_body=false
  local next_method=false
  local next_url=false
  local url=
  local arg
  for arg in "$@"; do
    if [[ "$next_method" == true ]]; then
      method=${arg^^}
      explicit_method=true
      next_method=false
      continue
    fi
    if [[ "$next_url" == true ]]; then
      url=$arg
      next_url=false
      continue
    fi
    case "$arg" in
      -X|--request) next_method=true ;;
      --request=*) method=${arg#*=}; explicit_method=true ;;
      -d|--data|--data-raw|--data-binary|--data-urlencode|-F|--form|--form-string) request_body=true ;;
      --data=*|--data-raw=*|--data-binary=*|--data-urlencode=*|--form=*|--form-string=*) request_body=true ;;
      --url) next_url=true ;;
      http://*|https://*) url=$arg ;;
    esac
  done
  if [[ "$explicit_method" == false && "$request_body" == true ]]; then
    method=POST
  fi
  method=${method^^}
  if [[ -n "$url" ]]; then
    printf '%s\t%s\n' "$method" "$url" >>"$runtime_request_log"
  fi
  "$curl_binary" "$@"
}

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
if ss -H -ltn "sport = :$pg_port" | grep -q .; then
  echo "Beta runtime gate requires unused PostgreSQL port 127.0.0.1:$pg_port." >&2
  exit 1
fi

"$initdb_bin" -D "$pg_data_dir" -U "$pg_admin_user" \
  --auth-local=trust --auth-host=trust --no-locale >/dev/null
"$pg_ctl_bin" -D "$pg_data_dir" \
  -o "-p $pg_port -k $pg_socket_dir -h 127.0.0.1" \
  -l "$pg_log" -w start >/dev/null
pg_started=1

psql "$pg_admin_url" -v ON_ERROR_STOP=1 <<'SQL'
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ubuntu') THEN
    CREATE ROLE ubuntu LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticated') THEN
    CREATE ROLE authenticated NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'anon') THEN
    CREATE ROLE anon NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
  END IF;
END $$;
DO $$ BEGIN
  ASSERT NOT EXISTS (
    SELECT FROM pg_roles
    WHERE rolname IN ('ubuntu', 'authenticated', 'anon')
      AND (rolsuper OR rolcreaterole OR rolbypassrls)
  ), 'beta migration/runtime roles must not bypass RLS or manage roles';
END $$;
SQL
dropdb --if-exists --host=127.0.0.1 --port="$pg_port" --username="$pg_admin_user" "$database_name"
createdb --host=127.0.0.1 --port="$pg_port" --username="$pg_admin_user" --owner=ubuntu "$database_name"

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
  0020_group_messages_ad_columns.sql
  0021_ad_fingerprints.sql
  0022_ad_governance.sql
  0023_ad_delete_snapshots.sql
  0024_fix_rls_recursion.sql
  0025_realtime_ad_scan_outbox.sql
  0026_ad_governance_rls.sql
)

for pass in 1 2; do
  for migration in "${migrations[@]}"; do
    psql "$database_url" -v ON_ERROR_STOP=1 \
      -f "$rust_root/migrations/$migration" >/dev/null
  done
done

# Readonly schema assertions — verify governance tables, least-privilege helpers,
# realtime trigger, and tenant RLS all survived two full migration passes.
psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
DO $$ BEGIN
  -- 0022 governance core tables
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_group_policies');
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_scan_jobs');
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_governance_clusters');
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_governance_cluster_messages');
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_governance_actions');
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_warning_dedup');
  -- 0021 fingerprint tables
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_text_fingerprints');
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_sender_profiles');
  -- 0023 delete snapshots
  ASSERT EXISTS (SELECT FROM pg_tables WHERE schemaname = 'groups' AND tablename = 'ad_delete_snapshots');
  -- 0024 fix: SECURITY DEFINER helper functions
  ASSERT EXISTS (SELECT FROM pg_proc WHERE proname = 'is_platform_admin' AND pronamespace = 'auth'::regnamespace);
  ASSERT EXISTS (SELECT FROM pg_proc WHERE proname = 'current_user_is_platform_admin' AND pronamespace = 'auth'::regnamespace);
  ASSERT NOT has_function_privilege('anon', 'auth.is_platform_admin(uuid)', 'EXECUTE');
  ASSERT NOT has_function_privilege('anon', 'auth.current_user_is_platform_admin()', 'EXECUTE');
  ASSERT NOT has_function_privilege('authenticated', 'auth.is_platform_admin(uuid)', 'EXECUTE');
  ASSERT has_function_privilege('authenticated', 'auth.current_user_is_platform_admin()', 'EXECUTE');
  ASSERT (SELECT prosecdef FROM pg_proc WHERE oid = 'auth.is_platform_admin(uuid)'::regprocedure);
  ASSERT (SELECT prosecdef FROM pg_proc WHERE oid = 'auth.current_user_is_platform_admin()'::regprocedure);
  ASSERT EXISTS (SELECT FROM pg_trigger WHERE tgname = 'realtime_ad_scan_outbox' AND NOT tgisinternal);
  -- 0020 ad columns on group_messages
  ASSERT EXISTS (SELECT FROM pg_attribute a JOIN pg_class c ON a.attrelid = c.oid WHERE c.relname = 'group_messages' AND c.relnamespace = 'groups'::regnamespace AND a.attname = 'ad_text_hash');
  ASSERT EXISTS (SELECT FROM pg_attribute a JOIN pg_class c ON a.attrelid = c.oid WHERE c.relname = 'group_messages' AND c.relnamespace = 'groups'::regnamespace AND a.attname = 'ad_action_status');
  ASSERT 9 = (
    SELECT count(*)
    FROM pg_class table_class
    JOIN pg_namespace schema_namespace ON schema_namespace.oid = table_class.relnamespace
    WHERE schema_namespace.nspname = 'groups'
      AND table_class.relname IN (
        'ad_text_fingerprints', 'ad_sender_profiles', 'ad_group_policies',
        'ad_scan_jobs', 'ad_governance_clusters',
        'ad_governance_cluster_messages', 'ad_governance_actions',
        'ad_warning_dedup', 'ad_delete_snapshots'
      )
      AND table_class.relrowsecurity
      AND NOT table_class.relforcerowsecurity
  ), 'all governance tables must have non-FORCE tenant RLS';
  ASSERT 9 = (
    SELECT count(*)
    FROM pg_policies
    WHERE schemaname = 'groups'
      AND policyname IN (
        'ad_text_fingerprints_tenant', 'ad_sender_profiles_tenant',
        'ad_group_policies_tenant', 'ad_scan_jobs_tenant',
        'ad_governance_clusters_tenant',
        'ad_governance_cluster_messages_tenant',
        'ad_governance_actions_tenant', 'ad_warning_dedup_tenant',
        'ad_delete_snapshots_tenant'
      )
      AND cmd = 'ALL'
      AND qual LIKE '%app.tenant_id%'
      AND with_check LIKE '%app.tenant_id%'
  ), 'all governance tenant policies must scope reads and writes';
END $$;
SQL

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
INSERT INTO groups.ad_text_fingerprints
  (tenant_id, text_hash, normalized_text, representative_text)
VALUES
  ('beta-local', 'beta-rls-probe', 'beta', 'beta'),
  ('other-tenant', 'other-rls-probe', 'other', 'other');
SQL

# Exercise the policy as a non-owner role. The application owner remains able to
# run background jobs because 0026 deliberately does not FORCE RLS.
psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
GRANT USAGE ON SCHEMA auth, groups TO authenticated;
GRANT SELECT, INSERT ON groups.ad_text_fingerprints TO authenticated;
SQL
psql "$pg_admin_database_url" -v ON_ERROR_STOP=1 <<'SQL'
SET ROLE authenticated;
SELECT set_config('app.tenant_id', 'beta-local', false);
DO $$ BEGIN
  ASSERT 1 = (SELECT count(*) FROM groups.ad_text_fingerprints);
  ASSERT EXISTS (
    SELECT FROM groups.ad_text_fingerprints
    WHERE tenant_id = 'beta-local' AND text_hash = 'beta-rls-probe'
  );
  ASSERT NOT EXISTS (
    SELECT FROM groups.ad_text_fingerprints
    WHERE tenant_id = 'other-tenant'
  );
  BEGIN
    INSERT INTO groups.ad_text_fingerprints
      (tenant_id, text_hash, normalized_text)
    VALUES ('other-tenant', 'cross-tenant-write', 'blocked');
    RAISE EXCEPTION 'cross-tenant governance write was not blocked by RLS';
  EXCEPTION
    WHEN insufficient_privilege THEN NULL;
  END;
END $$;
RESET ROLE;
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
env -u AD_AUTO_ENFORCE_ENABLED -u AD_DELETE_ENABLED \
  DATABASE_URL="$database_url" \
  INTERNAL_SAAS_SECRET=beta-local-secret \
  LITELLM_BASE_URL=http://127.0.0.1:18091 \
  LITELLM_API_KEY=beta-mock \
  LITELLM_MODEL_CATALOG=gemini-2.5-flash-lite,gemini-2.5-pro,gpt-4o-mini \
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

assert_http_status() {
  local expected="$1"
  shift
  local actual
  actual=$(curl -sS -o "$contract_response" -w '%{http_code}' "$@")
  [[ "$actual" == "$expected" ]]
}

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

# Frontend method/path contract matrix.  These calls cover the contracts that
# static registration alone cannot prove, using only the disposable database
# and the local mock Telegram/LLM services started by this gate.
runtime_contract_matrix_status=running
psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO subscription_plans
  (plan_code, plan_name, price_usdt, duration_days, features_json, status)
VALUES
  ('beta-runtime-contract-plan', 'Beta Runtime Contract', 9.99, 30, '{"tier":"beta"}'::jsonb, 'active')
ON CONFLICT (plan_code) DO UPDATE
SET plan_name = EXCLUDED.plan_name,
    price_usdt = EXCLUDED.price_usdt,
    duration_days = EXCLUDED.duration_days,
    features_json = EXCLUDED.features_json,
    status = EXCLUDED.status,
    updated_at = now();
SQL

# Authentication and MiniApp lifecycle contracts.
curl -fsS "$base_url/api/csrf-token" \
  | jq -e '.status == "ok" and (.data.csrf_token | type == "string") and (.data.csrf_token | length) >= 32' >/dev/null
assert_http_status 401 -X POST -H 'content-type: application/json' \
  -d '{"token":"invalid-beta-token"}' "$base_url/api/login"
assert_http_status 400 -X POST -H 'content-type: application/json' \
  -d '{}' "$base_url/api/login/otp"
assert_http_status 400 -X POST -H 'content-type: application/json' \
  -d '{}' "$base_url/api/login/tma"
curl -fsS -X POST -H 'content-type: application/json' -d '{}' \
  "$base_url/api/miniapp/bootstrap" \
  | jq -e '.status == "reopen_in_telegram_required" and .reason == "bootstrap_auth_failed" and .renewal.rotated == false' >/dev/null
curl -fsS "$base_url/api/session/context" \
  | jq -e '.status == "ok" and .data.authenticated == false and .data.tenant_id == ""' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/session/context" \
  | jq -e '.status == "ok" and .data.authenticated == true and .data.role == "merchant_owner" and .data.tenant_id == "beta-local"' >/dev/null
assert_http_status 403 "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{}' "$base_url/api/session/context"

# Group listing, verification, membership, messages, and tenant isolation.
curl -fsS "${headers[@]}" "$base_url/api/group_listen/list?page=1&size=20" \
  | jq -e '.status == "ok" and .total == 2 and (.data | length) == 2 and all(.data[]; .tenant_id == "beta-local")' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"enabled":false}' "$base_url/api/group_listen/revoke_sync" \
  | jq -e '.status == "ok" and .delete_on_revoke == false' >/dev/null
assert_http_status 403 "${viewer_headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"group_id":-10001,"enabled":true}' "$base_url/api/group_listen/revoke_sync"
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"enabled":true}' "$base_url/api/group_listen/revoke_sync" \
  | jq -e '.status == "ok" and .delete_on_revoke == true' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_listen/unverified_users?period=week" \
  | jq -e '.status == "ok" and .total == 4 and (.data | all(.[]; .group_id == -10001 or .group_id == -10002))' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_listen/verified_users?period=week" \
  | jq -e '.status == "ok" and .total == 0 and (.data | type == "array")' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_members?group_id=-10001" \
  | jq -e '.status == "ok" and .total == 4 and (.data | all(.[]; .tenant_id == "beta-local" and .group_id == -10001))' >/dev/null
curl -fsS "${other_headers[@]}" "$base_url/api/group_members?group_id=-20001" \
  | jq -e '.status == "ok" and .total == 1 and .data[0].tenant_id == "other-tenant"' >/dev/null
curl -fsS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"group_id":-10001,"user_ids":[99999],"action":"promote"}' \
  "$base_url/api/group_members/admin_batch" \
  | jq -e '.status == "ok" and .summary.total == 1 and .summary.success == 0 and .summary.failed == 1 and .failed[0].reason == "member_not_found"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_messages?group_id=-10001&limit=50&offset=0" \
  | jq -e '.status == "ok" and .total == 1 and .data[0].tenant_id == "beta-local" and .data[0].content == "original beta group message"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_messages?group_id=-20001&limit=50&offset=0" \
  | jq -e '.status == "ok" and .total == 0 and (.data | length) == 0' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_verify_config?group_id=-10001" \
  | jq -e '.status == "ok" and .total == 1 and .data[0].tenant_id == "beta-local" and .data[0].verify_enabled == true' >/dev/null
assert_http_status 400 "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"group_id":-10001,"verify_type":"invalid"}' "$base_url/api/group_verify_config"
curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"group_id":-10001,"verify_enabled":true}' "$base_url/api/group_verify_config" \
  | jq -e '.status == "ok" and .data.group_id == -10001 and .data.tenant_id == "beta-local"' >/dev/null
verify_stats_contract_response=$(curl -fsS "${headers[@]}" "$base_url/api/group_stats/verify_stats?group_id=-10001")
jq -e '.status == "ok" and (.data.total_verified | type == "number") and (.data.total_unverified | type == "number") and .data.total_unverified <= 4' \
  <<<"$verify_stats_contract_response" >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/group_stats/verify_growth?group_id=-10001&days=7" \
  | jq -e '.status == "ok" and .data.days == 7 and (.data.items | type == "array")' >/dev/null

# Merchant contracts, including a real disposable payment order lifecycle.
curl -fsS "${headers[@]}" "$base_url/api/merchant/ai/quota" \
  | jq -e '.status == "ok" and (.data == null or (.data | type == "object"))' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/merchant/audit_logs" \
  | jq -e '.status == "ok" and (.data | type == "array") and (.total | type == "number")' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/merchant/features" \
  | jq -e '.status == "ok" and (.data | type == "array") and (.total | type == "number")' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/merchant/plans" \
  | jq -e '.status == "ok" and any(.data[]; .plan_code == "beta-runtime-contract-plan")' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/merchant/subscription" \
  | jq -e '.status == "ok" and (.data == null or (.data | type == "object"))' >/dev/null
assert_http_status 403 "${viewer_headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"status":"active","onboarding_completed":true}' "$base_url/api/merchant/status"
curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"status":"active","onboarding_completed":true}' "$base_url/api/merchant/status" \
  | jq -e '.status == "ok" and .data.tenant_id == "beta-local" and .data.status == "active"' >/dev/null
merchant_order_response=$(curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"plan_code":"beta-runtime-contract-plan"}' "$base_url/api/merchant/payment_orders")
merchant_order_id=$(jq -er '.data.id' <<<"$merchant_order_response")
jq -e '.status == "ok" and .data.plan_code == "beta-runtime-contract-plan" and .data.status == "pending"' <<<"$merchant_order_response" >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/merchant/payment_orders/latest" \
  | jq -e --arg id "$merchant_order_id" '.status == "ok" and .data.id == $id' >/dev/null
curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d "{\"order_id\":\"$merchant_order_id\"}" "$base_url/api/merchant/payment_orders/mark_paid" \
  | jq -e '.status == "ok" and .data.status == "paid"' >/dev/null
curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d "{\"order_id\":\"$merchant_order_id\",\"tx_hash\":\"beta-contract-tx\"}" \
  "$base_url/api/merchant/payment_orders/submit_tx" \
  | jq -e '.status == "ok" and .data.tx_hash == "beta-contract-tx" and .data.status == "paid"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/merchant/payment_config" \
  | jq -e '.status == "ok" and (.data == null or (.data | type == "object"))' >/dev/null

# Monitor contracts use one real monitor and two tenant-scoped transactions.
contract_monitor=$(curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"address":"TBetaContractMonitor","network":"TRC20","remark":"contract-before"}' \
  "$base_url/api/monitors")
contract_monitor_id=$(jq -er '.data.monitor_id' <<<"$contract_monitor")
jq -e '.status == "ok" and .data.address == "TBetaContractMonitor" and .data.is_active == true' <<<"$contract_monitor" >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/monitors" \
  | jq -e --argjson id "$contract_monitor_id" '.status == "ok" and any(.data[]; .monitor_id == $id and .address == "TBetaContractMonitor")' >/dev/null
curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"remark":"contract-after"}' "$base_url/api/monitors/$contract_monitor_id/remark" \
  | jq -e '.status == "ok" and .data.remark == "contract-after"' >/dev/null
contract_now_ms=$(date +%s%3N)
psql "$database_url" -v ON_ERROR_STOP=1 -v monitor_id="$contract_monitor_id" -v now_ms="$contract_now_ms" <<'SQL'
INSERT INTO monitor_transactions
  (tx_id, monitor_id, user_id, tenant_id, address, network, direction, amount, token_symbol, tx_timestamp, counterparty)
VALUES
  ('beta-contract-in', :monitor_id, 10001, 'beta-local', 'TBetaContractMonitor', 'TRC20', 'in', 12.50, 'USDT', :now_ms, 'TContractIn'),
  ('beta-contract-out', :monitor_id, 10001, 'beta-local', 'TBetaContractMonitor', 'TRC20', 'out', 2.50, 'USDT', :now_ms - 1000, 'TContractOut');
SQL
curl -fsS "${headers[@]}" "$base_url/api/monitors/pnl_summary?period=month" \
  | jq -e '.status == "ok" and .data.inflow == 12.5 and .data.outflow == 2.5 and .data.tx_count == 2' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/monitors/transactions?period=month&monitor_id=$contract_monitor_id" \
  | jq -e '.status == "ok" and .count == 2 and ([.data[].tx_id] | sort) == ["beta-contract-in","beta-contract-out"]' >/dev/null
curl -fsS "${other_headers[@]}" "$base_url/api/monitors" \
  | jq -e '.status == "ok" and .total == 0 and (.data | length) == 0' >/dev/null
curl -fsS "${headers[@]}" -X DELETE "$base_url/api/monitors/$contract_monitor_id" \
  | jq -e '.status == "ok"' >/dev/null
psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
DELETE FROM monitor_transactions
WHERE tx_id IN ('beta-contract-in', 'beta-contract-out');
SQL

# Platform contracts, including merchant, plan, payment-config, and order APIs.
platform_merchants_response=$(curl -sS "${platform_headers[@]}" "$base_url/api/platform/merchants")
jq -e '.status == "ok" and (.data | type == "array") and (.total | type == "number") and .total >= 2' \
  <<<"$platform_merchants_response" >/dev/null
curl -fsS "${platform_headers[@]}" "$base_url/api/platform/runtime_statuses" \
  | jq -e '.status == "ok" and (.data | type == "array") and (.total | type == "number") and .source == "rust_webhook_registry" and (.worker_telemetry_available | type == "boolean") and all(.data[]; (.source == "rust_webhook_registry" and (.worker_telemetry_available | type == "boolean")))' \
  >/dev/null
platform_owner_merchants_status=$(curl -sS -o "$contract_response" -w '%{http_code}' "${headers[@]}" "$base_url/api/platform/merchants")
[[ "$platform_owner_merchants_status" == 403 ]]
platform_merchant_response=$(curl -fsS "${platform_headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"merchant_name":"Beta Contract Merchant","tenant_id":"beta-contract-merchant"}' \
  "$base_url/api/platform/merchants")
platform_merchant_id=$(jq -er '.data.id' <<<"$platform_merchant_response")
jq -e '.status == "ok" and .data.tenant_id == "beta-contract-merchant" and .data.status == "pending"' <<<"$platform_merchant_response" >/dev/null
curl -fsS "${platform_headers[@]}" -X POST -H 'content-type: application/json' \
  -d "{\"merchant_id\":\"$platform_merchant_id\",\"note\":\"runtime contract\"}" \
  "$base_url/api/platform/merchants/confirm_onboarding" \
  | jq -e '.status == "ok" and .data.onboarding_completed == true and .data.status == "active_free"' >/dev/null
curl -fsS "${platform_headers[@]}" -X DELETE "$base_url/api/platform/merchants/$platform_merchant_id" \
  | jq -e '.status == "ok"' >/dev/null
curl -fsS "${platform_headers[@]}" "$base_url/api/platform/subscriptions" \
  | jq -e '.status == "ok" and (.data | type == "array") and (.total | type == "number")' >/dev/null
curl -fsS "${platform_headers[@]}" "$base_url/api/platform/payment_orders" \
  | jq -e --arg id "$merchant_order_id" '.status == "ok" and any(.data[]; .id == $id)' >/dev/null
curl -fsS "${platform_headers[@]}" -X POST -H 'content-type: application/json' \
  -d "{\"order_id\":\"$merchant_order_id\"}" "$base_url/api/platform/payment_orders/confirm" \
  | jq -e '.status == "ok" and .data.status == "confirmed"' >/dev/null
curl -fsS "${platform_headers[@]}" -X POST -H 'content-type: application/json' \
  -d "{\"order_id\":\"$merchant_order_id\"}" "$base_url/api/platform/payment_orders/reject" \
  | jq -e '.status == "ok"' >/dev/null
assert_http_status 404 "${platform_headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"order_id":"00000000-0000-4000-8000-000000000000"}' "$base_url/api/platform/payment_orders/reject"
curl -fsS "${platform_headers[@]}" "$base_url/api/platform/plans" \
  | jq -e '.status == "ok" and any(.data[]; .plan_code == "beta-runtime-contract-plan")' >/dev/null
curl -fsS "${platform_headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"plan_code":"beta-runtime-contract-upsert","plan_name":"Beta Upsert","price_usdt":1.5,"duration_days":7,"features_json":{"beta":true},"status":"active"}' \
  "$base_url/api/platform/plans" \
  | jq -e '.status == "ok" and .data.plan_code == "beta-runtime-contract-upsert" and .data.features_json.beta == true' >/dev/null
assert_http_status 403 "${headers[@]}" "$base_url/api/platform/payment_config"
curl -fsS "${platform_headers[@]}" "$base_url/api/platform/payment_config" \
  | jq -e '.status == "ok" and (.data == null or (.data | type == "object"))' >/dev/null
curl -fsS "${platform_headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"chain_type":"TRC20","payment_address":"TbetaRuntimePaymentAddress","enabled":true}' \
  "$base_url/api/platform/payment_config" \
  | jq -e '.status == "ok" and .data.payment_address == "TbetaRuntimePaymentAddress" and .data.enabled == true' >/dev/null
curl -fsS "${platform_headers[@]}" "$base_url/api/platform/payment_config" \
  | jq -e '.status == "ok" and .data.payment_address == "TbetaRuntimePaymentAddress"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/merchant/payment_config" \
  | jq -e '.status == "ok" and .data.payment_address == "TbetaRuntimePaymentAddress"' >/dev/null

# Unknown ad scan IDs must be tenant-scoped and fail closed.
assert_http_status 404 "${headers[@]}" "$base_url/api/agent/ad-scans/beta-contract-missing"
assert_http_status 403 "${platform_headers[@]}" "$base_url/api/agent/ad-scans/beta-contract-missing"

# Legacy n8n advertisement compatibility routes must persist/query real
# tenant-scoped group messages; they must not return fixed empty success data.
curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"group_id":-10001,"tg_msg_id":92001,"sender_user_id":30001,"content":"beta ad contact @runtime_sales"}' \
  "$base_url/api/ad-pipeline/events/group-message" \
  | jq -e '.status == "ok" and .data.created == true and .data.tenant_id == "beta-local" and .data.mode == "postgres.group_messages"' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE tenant_id = 'beta-local' AND group_id = -10001 AND tg_msg_id = 92001 AND content = 'beta ad contact @runtime_sales'") == 1 ]]
curl -fsS "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"days":2,"limit":20}' "$base_url/api/ad-pipeline/backfill" \
  | jq -e '.status == "ok" and .data.scanned >= 1 and (.data.clusters | type == "array")' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/advertisements?group_id=-10001&limit=20" \
  | jq -e '.status == "ok" and .success == true and (.data.clusters | type == "array") and .data_complete == true' >/dev/null
assert_http_status 404 "${headers[@]}" "$base_url/api/advertisements/beta-contract-missing"
runtime_contract_matrix_status=passed

curl -fsS "${headers[@]}" "$base_url/api/users" \
  | jq -e '.total == 1 and .data[0].user_id == 30001 and .data[0].tenant_id == "beta-local"' >/dev/null
curl -fsS "${other_headers[@]}" "$base_url/api/users" \
  | jq -e '.total == 2 and ([.data[].tenant_id] | unique) == ["other-tenant"]' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/messages/30001?page=1" \
  | jq -e '.total == 1 and .data[0].content == "beta inbound" and .data[0].tg_msg_id == 450' >/dev/null
assert_http_status 400 "${platform_headers[@]}" "$base_url/api/messages/30001"
curl -fsS "${platform_headers[@]}" "$base_url/api/messages/30001?tenant_id=beta-local" \
  | jq -e '.status == "ok" and (.data | type == "array") and .total == 1' >/dev/null
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
  "$base_url/api/ai/chat" | jq -e '.reply == "mock:hello beta" and .data.executed == false' >/dev/null
assert_http_status 400 "${headers[@]}" -X POST -H 'content-type: application/json' \
  -d '{"content":"   "}' "$base_url/api/ai/chat"
ad_scan_response=$(curl -sS "${headers[@]}" -H 'content-type: application/json' \
  -d '{"content":"全量扫描广告","user_id":-10001}' "$base_url/api/ai/chat")
jq -e '.status == "ok" and .data.executed == false and (.tool_results.list_ad_scan_groups.data.groups | type == "array") and all(.tool_results.list_ad_scan_groups.data.groups[]; .capabilities_known == false and .capabilities_source == "not_checked")' \
  <<<"$ad_scan_response" >/dev/null
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
  -d '{"operation":"message_existence_probe","chat_id":-10001,"tg_msg_id":601,"staging_chat_id":-30001}' \
  "$base_url/api/agent/tools/telegram-sdk-query" \
  | jq -e '.success == false and .data_complete == false and .data.exists == null and .data.error_kind == "telegram_api_error"' >/dev/null
curl -fsS "${headers[@]}" "$base_url/api/openai/models" \
  | jq -e '.status == "ok" and any(.data[]; .id == "gpt-4o-mini" and .object == "model")' >/dev/null
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
  (.[0].text | contains("ref_xyz") and contains("未执行任何操作")) and
  .[0].reply_markup == null
' >/dev/null
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 30005 AND to_userid = 777000 AND tg_msg_id = 805") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM chat.messages WHERE tenant_id = 'beta-local' AND from_userid = 777000 AND to_userid = 30005 AND quoted_msg_id = 805 AND is_read = true AND content LIKE '%ref_xyz%' AND content LIKE '%未执行任何操作%'") == 1 ]]
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
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.ad_scan_jobs WHERE tenant_id = 'beta-local' AND source = 'realtime'") == 0 ]]

psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL'
INSERT INTO groups.ad_group_policies (tenant_id, group_id, is_enabled, activated_by, activated_at)
VALUES ('beta-local', -10001, true, 'beta-runtime-check', now())
ON CONFLICT (tenant_id, group_id) DO UPDATE
SET is_enabled = true, activated_by = EXCLUDED.activated_by, activated_at = EXCLUDED.activated_at;
SQL
runtime_message_ts=$(date +%s)
send_beta_webhook "{\"update_id\":130,\"message\":{\"message_id\":810,\"from\":{\"id\":30005,\"is_bot\":false,\"first_name\":\"新用户\",\"username\":\"beta_inbound\"},\"chat\":{\"id\":-10001,\"type\":\"supergroup\",\"title\":\"Beta Group One\"},\"date\":$runtime_message_ts,\"caption\":\"实时广告治理图片消息\",\"photo\":[{\"file_id\":\"rt-small-1\",\"file_unique_id\":\"rt-photo-unique\",\"width\":10,\"height\":10},{\"file_id\":\"rt-large-1\",\"file_unique_id\":\"rt-photo-unique\",\"width\":100,\"height\":100}]}}" \
  | jq -e '. == {}' >/dev/null
send_beta_webhook "{\"update_id\":131,\"message\":{\"message_id\":811,\"from\":{\"id\":30006,\"is_bot\":false,\"first_name\":\"新用户二\",\"username\":\"beta_inbound_two\"},\"chat\":{\"id\":-10001,\"type\":\"supergroup\",\"title\":\"Beta Group One\"},\"date\":$runtime_message_ts,\"caption\":\"实时广告治理图片消息\",\"photo\":[{\"file_id\":\"rt-large-2\",\"file_unique_id\":\"rt-photo-unique\",\"width\":100,\"height\":100}]}}" \
  | jq -e '. == {}' >/dev/null
send_beta_webhook "{\"update_id\":132,\"message\":{\"message_id\":812,\"from\":{\"id\":30007,\"is_bot\":false,\"first_name\":\"新用户三\",\"username\":\"beta_inbound_three\"},\"chat\":{\"id\":-10001,\"type\":\"supergroup\",\"title\":\"Beta Group One\"},\"date\":$runtime_message_ts,\"caption\":\"实时广告治理图片消息\",\"photo\":[{\"file_id\":\"rt-large-3\",\"file_unique_id\":\"rt-photo-unique\",\"width\":100,\"height\":100}]}}" \
  | jq -e '. == {}' >/dev/null
realtime_scan_status=
for _ in $(seq 1 150); do
  realtime_scan_status=$(psql "$database_url" -Atc "SELECT status FROM groups.ad_scan_jobs WHERE tenant_id = 'beta-local' AND group_id = -10001 AND source = 'realtime' AND job_id LIKE '%_-10001_812_%' ORDER BY created_at DESC LIMIT 1")
  [[ "$realtime_scan_status" == completed ]] && break
  sleep 0.1
done
[[ "$realtime_scan_status" == completed ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.group_messages WHERE tenant_id = 'beta-local' AND group_id = -10001 AND tg_msg_id BETWEEN 810 AND 812 AND media_unique_id = 'rt-photo-unique' AND message_ts = $runtime_message_ts AND length(ad_text_hash) = 32") == 3 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.ad_scan_jobs WHERE tenant_id = 'beta-local' AND group_id = -10001 AND source = 'realtime'") == 3 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM job_queue WHERE tenant_id = 'beta-local' AND job_type = 'ad_scan_job' AND payload ->> 'source' = 'realtime' AND payload ->> 'group_id' = '-10001' AND payload ->> 'recent_only' = 'true'") == 3 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.ad_scan_jobs WHERE tenant_id = 'beta-local' AND job_id LIKE '%_-10001_812_%' AND status = 'completed' AND total_messages = 3 AND candidate_clusters = 1 AND confirmed_clusters = 1 AND summary_json::jsonb ->> 'pending_clusters' = '0' AND summary_json::jsonb ->> 'recent_only' = 'true'") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.ad_governance_clusters WHERE tenant_id = 'beta-local' AND job_id LIKE '%_-10001_812_%' AND classification = 'ad' AND classification_source = 'llm' AND confidence >= 0.95 AND occurrence_count = 3 AND recent_count = 3 AND sender_count = 3 AND action_tier = 'warn'") == 1 ]]
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.ad_governance_cluster_messages cluster_message JOIN groups.ad_governance_clusters cluster USING (cluster_id) WHERE cluster.tenant_id = 'beta-local' AND cluster.job_id LIKE '%_-10001_812_%' AND cluster_message.tg_msg_id BETWEEN 810 AND 812 AND cluster_message.is_actionable = true") == 3 ]]
# Jobs are non-dry-run, but the process explicitly unsets AD_AUTO_ENFORCE_ENABLED
# and AD_DELETE_ENABLED. Classification must persist without any Telegram action.
[[ $(psql "$database_url" -Atc "SELECT count(*) FROM groups.ad_governance_actions WHERE tenant_id = 'beta-local' AND job_id IN (SELECT job_id FROM groups.ad_scan_jobs WHERE tenant_id = 'beta-local' AND source = 'realtime')") == 0 ]]
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

mkdir -p "$runtime_report_dir"
cp "$runtime_request_log" "$runtime_request_trace_path"
node "$script_directory/contract-runtime-report.mjs" \
  --static "$runtime_report_dir/api-static-registration.json" \
  --requests "$runtime_request_trace_path" \
  --output "$runtime_contract_report_path"
runtime_contract_static_only_cases=$(jq -er '.static_only_count' "$runtime_contract_report_path")
runtime_contract_runtime_seen=$(jq -er '.runtime_request_seen_count' "$runtime_contract_report_path")

write_runtime_report passed
echo "Beta runtime gates passed: migrations, tenant-isolated webhook ingestion and chat, private commands and tenant auto-replies, group management and broadcast delivery, reminder cleanup, scheduled push tasks, finance, AI, config, Telegram secrets/webhooks, and group assignment APIs."
