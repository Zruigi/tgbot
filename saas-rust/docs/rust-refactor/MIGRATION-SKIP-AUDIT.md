# Legacy migration skip and BYPASSRLS audit

This is a release-gate artifact, not an assertion that the legacy database has
been inspected in this workspace. No production credentials or database were
read during the Beta checks.

## Known evidence

The committed migration progress note records run `b1c5f14` as `25,892 / 25,894`
rows with two skips described as source-data-integrity problems. It does not
contain the source table, primary key, error text, or a durable per-row export.
The migration binaries currently report aggregate `migrated`, `skipped`, and
`errors` counters; those counters are not sufficient to prove that the two
rows are safe to omit.

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
