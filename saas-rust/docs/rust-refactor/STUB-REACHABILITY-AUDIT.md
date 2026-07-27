# Formal release reachability audit

Date: 2026-07-27

This is the static reachability record for the `tgbot-app` Beta target. It is
not a claim that every historical crate is production-ready.

## Formal app graph

`cargo tree -p tgbot-app` reaches `tgbot-http`, `tgbot-domain`, `tgbot-repos`,
`tgbot-services`, `tgbot-agent`, `tgbot-integrations`, and `tgbot-workers`.
The standalone `tgbot-bot` package has no reverse dependency and is compiled
only with its explicit `legacy-bot` feature. Historical empty modules are
similarly excluded by default:

- `tgbot-agent/legacy-stubs`: empty `generate_content` module;
- `tgbot-integrations/legacy-stubs`: empty `bscscan` module;
- `tgbot-workers/legacy-workers`: historical AI/monitor/group-policy/OCR/
  payment workers and monitor cron helpers;
- `tgbot-services/legacy-stubs` and `tgbot-repos/legacy-stubs`: old service and
  repository placeholders.

The default `tgbot-app` scheduler registers only push tasks, group-message
policy, ad governance, and explicitly enabled Telegram command/auto-reply
workers. An excluded feature must not be used as release evidence.

## Fail-closed checks

The reachable Agent and HTTP paths now reject missing tenant context, missing
Telegram/LLM clients, unknown tools, unknown ad-scan IDs, unsupported broadcast
media, and unavailable model catalogs instead of returning a success-shaped
placeholder. Advertisement compatibility routes persist and query tenant-
scoped PostgreSQL data. Webhook ingestion enqueues the idempotent realtime ad
scan outbox, and the runtime gate proves worker completion.

The monitor cron helper, when explicitly enabled for a legacy audit, queries
real rows and enqueues `monitor_check` jobs; database or enqueue errors now
propagate instead of becoming `Ok(0)`.

## Remaining non-release evidence

The following are still outside the release claim and require separate work or
real Telegram acceptance: old `tgbot-bot` handlers, the feature-gated legacy
workers, per-tenant worker telemetry (the runtime route exposes only the
PostgreSQL/Webhook registry snapshot and explicitly reports telemetry as
unavailable), and real Bot verification of webhook, commands, files, AI,
payments, retries, and rollback.
