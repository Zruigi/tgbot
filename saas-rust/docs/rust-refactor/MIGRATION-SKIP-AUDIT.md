# Legacy migration skip and BYPASSRLS audit

This is a release-gate artifact, not an assertion that the legacy database has
been inspected in this workspace. No production credentials or database were
read during the Beta checks.

## Known evidence

The committed migration progress note records run `b1c5f14` as `25,892 / 25,894`
rows with two skips described as source-data-integrity problems. It does not
contain the source table, primary key, error text, or a durable per-row export.
The migration binary now exits non-zero when any phase reports skipped, failed,
or unaccounted source rows, and message rows without a legacy `trace_id` receive
a deterministic migration identity. Full verification and `quick_verify` use
exact source/target counts; a source-sized migrated subset does not mask extra
target rows. Every migration phase now retains a source key and exact row
error. When row failures, phase accounting gaps, or verification mismatches
exist, the binary requires `MIGRATION_AUDIT_PATH` and
`MIGRATION_AUDIT_OPERATOR` and writes a checksummed JSON report with
`status=requires_disposition`; phase statistics are retained even when the
gap has no row-level converter error. The report is still an audit of failed or
unaccounted rows, not an automatic acceptance of them.

Decision: the two skips are **not accepted and are not marked fixed**. Before a
release owner accepts them, run the migration in read-only/source-snapshot mode
and export one row per skip with:

- source table and primary key (or a deterministic row fingerprint);
- source value/nullability problem and exact error text;
- source count, target count, and the intended disposition (repair, transform,
  or explicitly omit);
- migration run ID, timestamp, operator, and checksum of the export.

The export must be attached to the acceptance record and the target row counts
rechecked after any repair. Until then, the missing two rows remain an open
launch blocker.

The migration binary requires an explicit `SQLITE_DB_PATH`; it will not infer a
source SQLite path from the Beta worktree or a sibling production directory.

For a disposable/source-snapshot run, set `MIGRATION_AUDIT_PATH` to an audit
file path, set `MIGRATION_AUDIT_OPERATOR` to the responsible operator name, and
optionally set `MIGRATION_RUN_ID` to a stable run identifier. These variables
must never contain a database URL, password, token, or other credential.

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
