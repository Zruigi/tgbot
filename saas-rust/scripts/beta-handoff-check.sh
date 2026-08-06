#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
repository_root=$(cd "$rust_root/.." && pwd)
report_directory=${BETA_REPORT_DIR:-"$rust_root/target/beta-reports"}
handoff_path=${BETA_HANDOFF_REPORT:-"$report_directory/release-handoff.json"}
runtime_report="$report_directory/runtime-gate-report.json"
static_report="$report_directory/api-static-registration.json"
contract_report="$report_directory/frontend-runtime-contract-report.json"
rollback_report="$report_directory/local-rollback-rehearsal.json"
isolation_report="$report_directory/isolation-audit.json"
binary_path="$rust_root/target/release/tgbot-app"
migration_binary_path="$rust_root/target/release/migrate"

for command in awk cargo date git jq node sha256sum tr wc; do
  command -v "$command" >/dev/null || {
    echo "Beta handoff check failed: missing required command $command." >&2
    exit 1
  }
done

for required_file in "$runtime_report" "$static_report" "$contract_report" "$binary_path" "$migration_binary_path"; do
  if [[ ! -f "$required_file" ]]; then
    echo "Beta handoff check failed: missing local evidence $required_file." >&2
    exit 1
  fi
done

bash "$script_directory/check-migration-manifest.sh" >/dev/null
node "$script_directory/check-api-parity.mjs" --report "$static_report" >/dev/null
cargo fmt --all -- --check
cargo test --manifest-path "$rust_root/Cargo.toml" -p tgbot-migration
git -C "$rust_root" diff --check -- .

runtime_status=$(jq -r '.status // "missing"' "$runtime_report")
runtime_production_credentials=$(jq -r 'if .database.production_credentials_used == null then "missing" else (.database.production_credentials_used | tostring) end' "$runtime_report")
runtime_contract_count=$(jq -r '.frontend_contracts.method_path_contracts // 0' "$runtime_report")
runtime_seen=$(jq -r '.frontend_contracts.runtime_request_seen // 0' "$runtime_report")
runtime_static_only=$(jq -r '.frontend_contracts.static_only_cases // -1' "$runtime_report")
rollback_status="not_run"
rollback_binary_sha256=""
if [[ -f "$rollback_report" ]]; then
  rollback_status=$(jq -r '.status // "invalid"' "$rollback_report")
  rollback_binary_sha256=$(jq -r '.binary_sha256 // ""' "$rollback_report")
fi
isolation_status="not_run"
if [[ -f "$isolation_report" ]]; then
  isolation_status=$(jq -r '.status // "invalid"' "$isolation_report")
fi
migration_passes=$(jq -r '.migrations.passes // 0' "$runtime_report")
migration_file_count=$(jq -r '.migrations.file_count // 0' "$runtime_report")
migration_latest=$(jq -r '.migrations.latest // "missing"' "$runtime_report")
static_missing_paths=$(jq -r '.missing_paths | length' "$static_report")
static_missing_methods=$(jq -r '.missing_methods | length' "$static_report")
working_tree_status=$(git -C "$rust_root" status --porcelain --untracked-files=all -- .)
if [[ -n "$working_tree_status" ]]; then
  working_tree_dirty=true
else
  working_tree_dirty=false
fi
tracked_change_count=$(git -C "$rust_root" status --porcelain --untracked-files=no -- . | wc -l | tr -d ' ')
head_commit=$(git -C "$rust_root" rev-parse HEAD)
binary_sha256=$(sha256sum "$binary_path" | awk '{print $1}')
migration_binary_sha256=$(sha256sum "$migration_binary_path" | awk '{print $1}')
if [[ "$rollback_status" == "passed" && "$rollback_binary_sha256" != "$binary_sha256" ]]; then
  rollback_status="stale_binary"
fi

ui_worktree_status=$(git -C "$repository_root" status --porcelain --untracked-files=all -- frontend static)
if [[ -n "$ui_worktree_status" ]]; then
  ui_gate="blocked_preexisting_worktree_changes"
elif git -C "$repository_root" rev-parse --verify origin/master >/dev/null 2>&1; then
  if git -C "$repository_root" diff --quiet origin/master -- frontend static; then
    ui_gate="passed"
  else
    ui_gate="blocked_branch_ui_changes"
  fi
else
  ui_gate="unknown_missing_origin_master"
fi

runtime_evidence="previous_report_requires_refresh"
if [[ "$runtime_status" == "passed" \
  && "$runtime_production_credentials" == "false" \
  && "$runtime_contract_count" == "102" \
  && "$runtime_seen" == "102" \
  && "$runtime_static_only" == "0" \
  && "$rollback_status" == "passed" \
  && "$isolation_status" == "passed" \
  && "$migration_passes" == "2" \
  && "$migration_file_count" == "28" \
  && "$migration_latest" == "0031_monitor_alert_outbox.sql" \
  && "$static_missing_paths" == "0" \
  && "$static_missing_methods" == "0" ]]; then
  runtime_evidence="local_mock_evidence_recorded"
fi
local_evidence_status="needs_evidence_refresh"
if [[ "$runtime_evidence" == "local_mock_evidence_recorded" ]]; then
  local_evidence_status="ready_for_review"
fi

ui_blockers='[]'
if [[ "$ui_gate" != "passed" ]]; then
  ui_blockers=$(jq -nc --arg ui_gate "$ui_gate" '["production UI/static gate: " + $ui_gate]')
fi
if [[ "$rollback_status" != "passed" ]]; then
  ui_blockers=$(jq -c --arg rollback_status "$rollback_status" '. + ["local Beta rollback/restart rehearsal: " + $rollback_status]' <<<"$ui_blockers")
fi
if [[ "$isolation_status" != "passed" ]]; then
  ui_blockers=$(jq -c --arg isolation_status "$isolation_status" '. + ["Beta isolation audit: " + $isolation_status]' <<<"$ui_blockers")
fi
release_blockers='[
  "two historical migration skips require an authorized source-snapshot row audit and checksum",
  "real Telegram/provider semantics and crash-window retry behavior are not locally authorized",
  "small-tenant canary, observation window, and rollback rehearsal are external operations"
]'
if [[ "$working_tree_dirty" == "true" ]]; then
  release_blockers=$(jq -c '. + ["working tree is not a clean release candidate"]' <<<"$release_blockers")
fi
release_blockers=$(jq -c --argjson ui_blockers "$ui_blockers" '. + $ui_blockers' <<<"$release_blockers")

mkdir -p "$(dirname "$handoff_path")"
jq -n \
  --arg generated_at_utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg head_commit "$head_commit" \
  --arg binary_path "$binary_path" \
  --arg binary_sha256 "$binary_sha256" \
  --arg migration_binary_path "$migration_binary_path" \
  --arg migration_binary_sha256 "$migration_binary_sha256" \
  --arg runtime_status "$runtime_status" \
  --arg runtime_evidence "$runtime_evidence" \
  --arg local_evidence_status "$local_evidence_status" \
  --arg ui_gate "$ui_gate" \
  --arg rollback_status "$rollback_status" \
  --arg rollback_binary_sha256 "$rollback_binary_sha256" \
  --arg isolation_status "$isolation_status" \
  --arg migration_latest "$migration_latest" \
  --argjson release_blockers "$release_blockers" \
  --argjson ui_blockers "$ui_blockers" \
  --argjson working_tree_dirty "$working_tree_dirty" \
  --argjson tracked_change_count "$tracked_change_count" \
  --argjson runtime_contract_count "$runtime_contract_count" \
  --argjson runtime_seen "$runtime_seen" \
  --argjson runtime_static_only "$runtime_static_only" \
  --argjson migration_passes "$migration_passes" \
  --argjson migration_file_count "$migration_file_count" \
  --argjson static_missing_paths "$static_missing_paths" \
  --argjson static_missing_methods "$static_missing_methods" \
  ' {
      report_type: "rust_beta_release_handoff",
      generated_at_utc: $generated_at_utc,
      candidate: {
        branch: "beta/rust-migration",
        head_commit: $head_commit,
        binary_path: $binary_path,
        binary_sha256: $binary_sha256,
        migration_binary_path: $migration_binary_path,
        migration_binary_sha256: $migration_binary_sha256,
        working_tree_dirty: $working_tree_dirty,
        tracked_working_tree_change_count: $tracked_change_count
      },
      local_evidence: {
        status: $local_evidence_status,
        static_registration: {
          method_path_contracts: $runtime_contract_count,
          runtime_requests_seen: $runtime_seen,
          static_only_cases: $runtime_static_only,
          missing_paths: $static_missing_paths,
          missing_methods: $static_missing_methods
        },
        migrations: {
          passes: $migration_passes,
          file_count: $migration_file_count,
          latest: $migration_latest,
          manifest: "scripts/beta-migrations.list"
        },
        runtime: {
          status: $runtime_status,
          evidence: $runtime_evidence,
          credentials: "disposable_local_postgresql_and_mocks_only"
        },
        rollback_rehearsal: {
          status: $rollback_status,
          binary_sha256: $rollback_binary_sha256,
          report: "target/beta-reports/local-rollback-rehearsal.json"
        },
        isolation_audit: {
          status: $isolation_status,
          report: "target/beta-reports/isolation-audit.json"
        },
        ui_gate: $ui_gate
      },
      release_approval: {
        status: "blocked",
        blockers: $release_blockers
      },
      external_gates: [
        "real isolated Telegram Bot webhook, commands, group management, files, AI, payments, and retries",
        "real chain-provider test configuration for address-monitor alerts",
        "authorized migration-role BYPASSRLS and two-row disposition evidence",
        "authorized canary, observation, and rollback rehearsal"
      ]
    }' >"$handoff_path"

echo "Beta local handoff is ready for review: $handoff_path"
echo "Formal release approval remains blocked by documented external gates."
