-- D4：平台价格告警表（净新增，对齐 Python crypto_price.py 的「添加价格提醒」占位能力）
-- 编号 0039（本地最新 migration 已到 0038，0024 已被 0024_fix_rls_recursion.sql 占用）
-- 幂等：IF NOT EXISTS，可安全重复执行。

BEGIN;

CREATE TABLE IF NOT EXISTS platform.price_alerts (
    alert_id          BIGSERIAL PRIMARY KEY,
    tenant_id         TEXT        NOT NULL,
    user_id           BIGINT      NOT NULL,              -- 配置告警的用户 TG ID
    symbol            TEXT        NOT NULL,              -- 如 'BTCUSDT'（Binance 无斜杠格式）
    threshold_high    NUMERIC,                            -- 二选一或同时
    threshold_low     NUMERIC,
    is_active         BOOLEAN     NOT NULL DEFAULT true,
    notify_chat_id    BIGINT,                             -- 告警推送目标（默认等于 user_id）
    last_checked_at   TIMESTAMPTZ,
    last_triggered_at TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (threshold_high IS NOT NULL OR threshold_low IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_price_alerts_active
    ON platform.price_alerts (is_active, last_checked_at)
    WHERE is_active = true;

COMMIT;
