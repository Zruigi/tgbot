# Legacy migration skip and BYPASSRLS audit

This is a release-gate artifact, not an assertion that the legacy database has
been inspected in this workspace. No production credentials or database were
read during the Beta checks. The audit below was executed read-only against the
production SQLite source and an isolated local PostgreSQL cluster; the
production `/home/ubuntu/tgbot/.env` `SUPABASE_DATABASE_URL` was never used.

## Historical context

The committed migration progress note records run `b1c5f14` as
`25,892 / 25,894` rows with two skips described as source-data-integrity
problems. It did not contain the source table, primary key, error text, or a
durable per-row export. Small-table migration (`migrate_small_tables.rs`) was
first introduced by that same commit, so the two unaccounted rows originated
from the small tables.

## Concrete audit run (2026-08-07, local isolated PostgreSQL)

Executed with the current fail-closed migration binary
(`target/release/migrate`, built 2026-08-05) against:

- source: `/home/ubuntu/tgbot/crypto_bot.sqlite3` (opened read-only via
  `file:...?mode=ro` in all sqlite3 checks; migration itself opens it via
  rusqlite read path, never modified);
- target: a fresh database `tgbot_rust_beta_audit` on the local isolated
  PostgreSQL cluster (port 15432, `beta-local-app.sh` / manual `initdb`
  cluster, `target/beta-postgres`), migrations 0001-0031 applied;
- audit env: `MIGRATION_AUDIT_PATH=…/target/beta-reports/migration-skip-audit.json`,
  `MIGRATION_AUDIT_OPERATOR=workbuddy-audit`.

Result: the binary exited non-zero and wrote
`target/beta-reports/migration-skip-audit.json` with
`status=requires_disposition`. Exactly two rows were unaccounted.

- run_id: `9f3ff962-e8e8-47df-b5a8-fc30e5b8e823`
- generated_at: `2026-08-06T20:47:34Z` (UTC)
- checksum_sha256: `9fdb2b14c0e2364f485976a4c0d105b749d69619dc950a3b177e1d2757867331`
- phase stats: all phases `accounted == total_source` except
  `address_monitors` (source 6, migrated 5, errors 1) and
  `monitor_transactions` (source 29, migrated 28, errors 1).
- overall verification: source 28,044 / target 28,042 (all other tables exact:
  merchants 2/2, messages 1296/1296, groups 15/15, members 5887/5887,
  group_messages 20622/20622, users 5/5, admin_inbox 180/180 with 0 virtual
  assistants excluded, plans 2/2).

## The two skipped/unaccounted rows (row-level evidence)

### 1) `address_monitors` — source key `tenant_id=default,monitor_id=4`

- Error text (from audit JSON):
  `error returned from database: duplicate key value violates unique constraint "address_monitors_address_network_tenant_id_key"`
- Source row (read-only sqlite3):
  - monitor_id = `4`, user_id = `7513843783`, owner_type = `user`
  - address = `TMQJX2THLbY8fwTBHTrp47iK9AqW6GcX7C`, network = `trc20`,
    tenant_id = `default`
  - created_at = `2026-03-24 06:42:24`, last_tx_timestamp = `1774621203000`,
    is_active = `0`, risk_level = `high`, remark = `监测`
- Root cause: this row is a duplicate of `monitor_id = 1` under the target
  unique index `UNIQUE (address, network, tenant_id)`:
  - monitor_id = `1`, user_id = `6889612019`, owner_type = `admin_own`,
    same address/network/tenant, created_at identical
    `2026-03-24 06:42:24`, last_tx_timestamp = `1774284702000`.
  - Two different owners monitor the same TRON address. The target schema has
    two unique indexes: `address_monitors_tenant_monitor_key`
    (`UNIQUE (tenant_id, monitor_id)`, covered by the migrator's
    `ON CONFLICT (monitor_id)`) and
    `address_monitors_address_network_tenant_id_key`
    (`UNIQUE (address, network, tenant_id)`, **not** covered by the upsert).
    The second row therefore violates the business-level unique index.
- Counts: `address_monitors` source=6, target=5.

### 2) `monitor_transactions` — source key `tenant_id=default,tx_id=a6c58518ac066f5acd6ff8d541d0e9e0ddb586c87883485f0d49f65dff5f47f9`

- Error text (from audit JSON):
  `error returned from database: insert or update on table "monitor_transactions" violates foreign key constraint "monitor_transactions_tenant_monitor_fkey"`
- Source row (read-only sqlite3):
  - tx_id = `a6c58518ac066f5acd6ff8d541d0e9e0ddb586c87883485f0d49f65dff5f47f9`,
    monitor_id = `4`, user_id = `7513843783`, tenant_id = `default`
  - address = `TMQJX2THLbY8fwTBHTrp47iK9AqW6GcX7C`, network = `TRC20`
  - direction = `in`, amount = `0.8`, token_symbol = `USDT`,
    tx_timestamp = `1774621203000`,
    counterparty = `TQcjpMUrcmM4JWPyoidkuKLJ31HAW4m3HH`, risk_level = `high`
- Root cause: this transaction references `monitor_id = 4` via FK
  `monitor_transactions_tenant_monitor_fkey`
  (`FOREIGN KEY (tenant_id, monitor_id) REFERENCES address_monitors
  (tenant_id, monitor_id) ON DELETE CASCADE`). Because row 1
  (`address_monitors.monitor_id = 4`) could not be inserted, this transaction's
  parent row does not exist in the target, so the FK check fails. This is a
  cascading consequence of the duplicate-address problem, not an independent
  source defect (monitor_id 4 exists in the source `address_monitors`).
- Counts: `monitor_transactions` source=29, target=28.

## Read-only source data quality sweep (all clean except the two rows above)

Run with `sqlite3 "file:…?mode=ro"` on the production source:

- NULL primary keys: 0 in merchants/messages/tracked_groups/group_members/
  group_messages(msg_id)/users/admin_inbox/plans/address_monitors/
  monitor_transactions.
- Duplicate keys on every ON CONFLICT target (legacy_merchant_id/id,
  messages.trace_id, groups.group_id, members(group_id,user_id), users.user_id,
  admin_inbox(from_userid,tenant_id), plans.plan_code,
  monitor_transactions(tenant_id,tx_id),
  group_messages(group_id,tg_msg_id) partial): all 0. The only duplicate found
  is the `address_monitors (address, network, tenant_id)` pair described above.
- Unparseable timestamps (checked with `strftime('%Y', …) IS NULL`): 0 across
  all required and optional timestamp columns of the ten migrated tables
  (merchants, messages, tracked_groups, group_members, group_messages, users,
  admin_inbox, plans, address_monitors, monitor_transactions).
- `monitor_transactions.monitor_id` NULL: 0. `tx_timestamp <= 0`: 0.
- Source `monitor_transactions` has no genuinely dangling monitor_id reference
  (all monitor_ids exist in source `address_monitors`); the FK failure is
  purely the cascading effect of row 1.

## Disposition recommendation

Both rows are genuine source-data issues and remain `unresolved`; they are a
release blocker until a release owner records an explicit disposition. The
concrete options, in recommended order:

1. `explicit-omit` the pair (default, lowest risk):
   - `address_monitors.monitor_id=4` duplicates address coverage already owned
     by `monitor_id=1`; omitting it loses no distinct address/network/tenant.
   - `monitor_transactions.tx_id=a6c58518…` belongs to the omitted monitor and
     must be omitted with it (document both rows in the acceptance record).
   - Expected target counts: address_monitors 5, monitor_transactions 28.
2. `transform` (if per-owner monitoring is required):
   - Re-point the transaction to `monitor_id=1` (same address) and migrate it
     while keeping the surviving address monitor row. Expected target counts:
     address_monitors 5, monitor_transactions 29.
3. `repair` the source before migrating (needs a product/data-owner decision):
   - Decide which owner of address `TMQJX2THLbY8fwTBHTrp47iK9AqW6GcX7C` is
     authoritative, deduplicate in the SQLite source, or relax the target
     `address_monitors_address_network_tenant_id_key` unique index. Both are
     schema/business decisions, not migration transforms. Expected target
     counts if both rows are then kept: address_monitors 6,
     monitor_transactions 29.

## Target count re-verification

After any disposition, re-run the migration against a fresh snapshot target and
re-verify with `verify_all_counts` (exact source==target equality). The
phase-stats accounting requires `migrated + skipped + errors + excluded ==
total_source` for every phase; an explicit omission must therefore be carried
as an `excluded`/documented disposition so the fail-closed gate is not rearmed
by an unexplained gap. Recheck counts per table, not just the aggregate, and
attach the new run_id + audit checksum to the acceptance record.

The migration binary requires an explicit `SQLITE_DB_PATH`; it will not infer a
source SQLite path from the Beta worktree or a sibling production directory.
`MIGRATION_AUDIT_PATH` / `MIGRATION_AUDIT_OPERATOR` (and optionally
`MIGRATION_RUN_ID`) must never contain a database URL, password, token, or other
credential.

## BYPASSRLS role lifecycle

`crates/migration/src/bin/migrate.rs` now performs a role preflight without
logging a connection string. When `MIGRATOR_DATABASE_URL` is used, the current
role must satisfy:

- `rolbypassrls = true`;
- `rolsuper = false`, `rolcreaterole = false`, and `rolcreatedb = false`;
- it is dedicated to the migration and is not the application runtime role.

When only `DATABASE_URL` is supplied, the tool logs a warning and does not
pretend that a dedicated BYPASSRLS audit passed. The following read-only checks
belong in the deployment evidence (run by an authorized operator against the
target database, never committed with secrets):

```sql
SELECT current_user, rolsuper, rolcreaterole, rolcreatedb, rolbypassrls
FROM pg_roles WHERE rolname = current_user;

SELECT n.nspname, p.proname, p.prosecdef, pg_get_userbyid(p.proowner) AS owner
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname = 'auth'
  AND p.proname IN ('is_platform_admin', 'current_user_is_platform_admin');
```

After migration, revoke or rotate the dedicated credential, remove it from
operators' environments, and verify that the app role cannot inherit it. Do not
put a password or PAT in a Git remote URL, bundle, log, or this document.
