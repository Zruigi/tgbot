# Formal release reachability audit

Date: 2026-08-01

This is the static reachability record for the `tgbot-app` Beta target. It is
not a claim that every historical crate is production-ready.

## Formal app graph

`cargo tree -p tgbot-app` reaches `tgbot-http`, `tgbot-domain`, `tgbot-repos`,
`tgbot-services`, `tgbot-agent`, `tgbot-integrations`, and `tgbot-workers`.
The standalone `tgbot-bot` package has no reverse dependency and is compiled
only with its explicit `legacy-bot` feature. Historical empty modules are
similarly excluded by default:

- `tgbot-agent/legacy-stubs`: empty `generate_content` module;
- `tgbot-workers/legacy-workers`: historical AI/monitor/group-policy/OCR/
  payment workers and monitor cron helpers;
- `tgbot-services/legacy-stubs` and `tgbot-repos/legacy-stubs`: old service and
  repository placeholders.

The default `tgbot-app` scheduler registers push tasks, group-message policy,
ad governance, and explicitly enabled Telegram command/auto-reply workers. The
formal address monitor worker and its cron are also registered only when
`TELEGRAM_WALLET_MONITORS_ENABLED=true`; the default remains disabled, and an
excluded feature must not be used as release evidence.
`scripts/beta-check.sh` now checks the resolved `tgbot-app` feature graph and
fails if any `legacy-bot`, `legacy-workers`, or `legacy-stubs` feature becomes
reachable.

## Fail-closed checks

The reachable Agent and HTTP paths now reject missing tenant context, missing
Telegram/LLM clients, unknown tools, unknown ad-scan IDs, invalid broadcast
media, and unavailable model catalogs instead of returning a success-shaped
placeholder. Advertisement compatibility routes persist and query tenant-
scoped PostgreSQL data. Webhook ingestion enqueues the idempotent realtime ad
scan outbox, and the runtime gate proves worker completion.

The historical monitor cron helper, when explicitly enabled for a legacy audit,
queries the old `wallet_monitors` table and enqueues `monitor_check` jobs;
database or enqueue errors now propagate instead of becoming `Ok(0)`. The
formal `schedule_address_monitor_checks` path instead queries tenant-scoped
`address_monitors` and rejects duplicate pending/running jobs per tenant.

The formal `AddressMonitorWorker` validates the claimed tenant again, loads the
active monitor by `(tenant_id, monitor_id)`, parses only the configured TRC20
or BSC transfer shape, and resolves the alert Bot by tenant. Its chain clients
are runtime-keyed and are not initialized while the feature gate is disabled.

The Supabase REST count helper likewise fails closed when PostgREST omits or
malforms `Content-Range`; missing count metadata is not reported as zero.

## Current Beta audit

On 2026-08-01, `cargo tree -p tgbot-app -e features` resolved the formal app
graph without `tgbot-bot` or any `legacy-*` feature. A source scan of the
reachable crates found no `TODO`, `FIXME`, `unimplemented!`, or `todo!`
implementation. The remaining placeholder wording is limited to
legacy-gated modules and a media-detection regex description, so the
reachable-stub gate is satisfied for the current Beta target.

## Remaining non-release evidence

The following are still outside the release claim and require separate work or
real Telegram acceptance: old `tgbot-bot` handlers, the feature-gated legacy
workers, real chain-provider responses, and real Bot verification of webhook,
commands, files, AI, payments, retries, and rollback. The runtime route now exposes in-process scheduler and
PostgreSQL-persisted global and tenant worker counters, but they cannot replace
external gray-release telemetry or a real acceptance record.
