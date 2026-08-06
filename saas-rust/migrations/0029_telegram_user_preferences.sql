BEGIN;

CREATE TABLE IF NOT EXISTS chat.telegram_user_preferences (
    tenant_id text NOT NULL REFERENCES merchants(tenant_id) ON DELETE CASCADE,
    telegram_user_id bigint NOT NULL CHECK (telegram_user_id > 0),
    locale text NOT NULL DEFAULT 'zh-CN' CHECK (locale IN ('zh-CN', 'en')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, telegram_user_id)
);

CREATE INDEX IF NOT EXISTS idx_telegram_user_preferences_user
    ON chat.telegram_user_preferences(telegram_user_id, updated_at DESC);

ALTER TABLE chat.telegram_user_preferences ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS telegram_user_preferences_tenant
    ON chat.telegram_user_preferences;
CREATE POLICY telegram_user_preferences_tenant ON chat.telegram_user_preferences
FOR ALL
USING (
    auth.current_user_is_platform_admin()
    OR tenant_id = current_setting('app.tenant_id', true)
)
WITH CHECK (
    auth.current_user_is_platform_admin()
    OR tenant_id = current_setting('app.tenant_id', true)
);

COMMIT;
