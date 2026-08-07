-- Monitor balance snapshots for trend / PnL charts
CREATE TABLE IF NOT EXISTS balance_monitor_balance_snapshots (
    snapshot_id        bigserial PRIMARY KEY,
    tenant_id          text NOT NULL,
    monitor_id         bigint NOT NULL,
    address            text NOT NULL,
    network            text NOT NULL,
    token_symbol       text NOT NULL,
    snapshot_at        timestamp with time zone NOT NULL DEFAULT now(),
    balance            numeric(40, 18) NOT NULL DEFAULT 0,
    usdt_value         numeric(30, 8),
    source             text NOT NULL DEFAULT 'worker'
);

CREATE INDEX IF NOT EXISTS idx_balance_snapshots_tenant_monitor
    ON balance_monitor_balance_snapshots (tenant_id, monitor_id, snapshot_at DESC);

CREATE INDEX IF NOT EXISTS idx_balance_snapshots_tenant_token
    ON balance_monitor_balance_snapshots (tenant_id, token_symbol, snapshot_at DESC);