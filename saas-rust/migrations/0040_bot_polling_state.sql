-- Polling receive mode: per-bot offset persistence (P2 polling 改造)。
-- 可选迁移：TGBOT_POLLING_ENABLED=true 时，bot_polling_state 记录每个 bot
-- 已确认的最后 update_id，重启后从 last_update_id+1 继续，避免重复回放。
-- 幂等：IF NOT EXISTS，可安全重复执行。

BEGIN;

CREATE TABLE IF NOT EXISTS bot_polling_state (
    bot_id         BIGINT PRIMARY KEY,
    last_update_id BIGINT NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMIT;
