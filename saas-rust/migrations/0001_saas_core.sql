CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS saas_user_roles (
    user_id uuid PRIMARY KEY REFERENCES auth.users(id) ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('platform_admin')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS merchants (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    legacy_merchant_id bigint UNIQUE,
    tenant_id text NOT NULL UNIQUE,
    owner_user_id uuid REFERENCES auth.users(id) ON DELETE SET NULL,
    owner_tg_user_id bigint,
    owner_username text,
    merchant_name text NOT NULL,
    bot_id bigint,
    bot_username text,
    status text NOT NULL DEFAULT 'pending',
    plan_type text NOT NULL DEFAULT 'free',
    subscription_status text NOT NULL DEFAULT 'free',
    subscription_expires_at timestamptz,
    onboarding_completed boolean NOT NULL DEFAULT false,
    runtime_version integer NOT NULL DEFAULT 1,
    config_version integer NOT NULL DEFAULT 1,
    feature_version integer NOT NULL DEFAULT 1,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS tenant_memberships (
    user_id uuid NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    tenant_id text NOT NULL REFERENCES merchants(tenant_id) ON DELETE CASCADE,
    merchant_id uuid NOT NULL REFERENCES merchants(id) ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('merchant_owner', 'merchant_admin', 'merchant_viewer')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, tenant_id)
);

CREATE TABLE IF NOT EXISTS subscription_plans (
    plan_code text PRIMARY KEY,
    plan_name text NOT NULL,
    price_usdt numeric(18, 6) NOT NULL DEFAULT 0,
    duration_days integer NOT NULL DEFAULT 30,
    features_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    status text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS merchant_subscriptions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    merchant_id uuid NOT NULL REFERENCES merchants(id) ON DELETE CASCADE,
    tenant_id text NOT NULL REFERENCES merchants(tenant_id) ON DELETE CASCADE,
    plan_code text NOT NULL REFERENCES subscription_plans(plan_code),
    status text NOT NULL DEFAULT 'active',
    start_at timestamptz NOT NULL DEFAULT now(),
    end_at timestamptz,
    payment_order_id uuid,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS payment_orders (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    merchant_id uuid NOT NULL REFERENCES merchants(id) ON DELETE CASCADE,
    tenant_id text NOT NULL REFERENCES merchants(tenant_id) ON DELETE CASCADE,
    plan_code text NOT NULL REFERENCES subscription_plans(plan_code),
    status text NOT NULL DEFAULT 'pending',
    amount_usdt numeric(18, 6) NOT NULL DEFAULT 0,
    base_amount_usdt numeric(18, 6) NOT NULL DEFAULT 0,
    amount_offset_usdt numeric(18, 6) NOT NULL DEFAULT 0,
    payment_address text,
    tx_hash text,
    expire_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS platform_payment_configs (
    id integer PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    chain_type text NOT NULL DEFAULT 'TRC20',
    payment_address text NOT NULL DEFAULT '',
    qr_image_path text,
    enabled boolean NOT NULL DEFAULT true,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS merchant_features (
    tenant_id text NOT NULL REFERENCES merchants(tenant_id) ON DELETE CASCADE,
    merchant_id uuid NOT NULL REFERENCES merchants(id) ON DELETE CASCADE,
    feature_key text NOT NULL,
    enabled boolean NOT NULL DEFAULT false,
    limits_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, feature_key)
);

CREATE TABLE IF NOT EXISTS merchant_ai_quota (
    tenant_id text PRIMARY KEY REFERENCES merchants(tenant_id) ON DELETE CASCADE,
    merchant_id uuid NOT NULL REFERENCES merchants(id) ON DELETE CASCADE,
    daily_quota integer NOT NULL DEFAULT 0,
    monthly_quota integer NOT NULL DEFAULT 0,
    daily_used integer NOT NULL DEFAULT 0,
    monthly_used integer NOT NULL DEFAULT 0,
    reset_at timestamptz
);

CREATE TABLE IF NOT EXISTS tenant_audit_logs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id text NOT NULL,
    merchant_id uuid,
    actor_user_id uuid,
    operator_role text NOT NULL,
    action text NOT NULL,
    resource_type text NOT NULL,
    resource_id text,
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS tenant_secret_refs (
    tenant_id text NOT NULL REFERENCES merchants(tenant_id) ON DELETE CASCADE,
    secret_key text NOT NULL,
    secret_ref text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, secret_key)
);

CREATE INDEX IF NOT EXISTS idx_merchants_owner_user ON merchants(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_merchants_owner_tg_user ON merchants(owner_tg_user_id);
CREATE INDEX IF NOT EXISTS idx_tenant_memberships_user ON tenant_memberships(user_id);
CREATE INDEX IF NOT EXISTS idx_payment_orders_tenant ON payment_orders(tenant_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_subscriptions_tenant ON merchant_subscriptions(tenant_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_tenant ON tenant_audit_logs(tenant_id, created_at DESC);

INSERT INTO subscription_plans (plan_code, plan_name, price_usdt, duration_days, features_json, status)
VALUES
    ('free', 'Free', 0, 30, '{"base_bot": true, "basic_ai": false}'::jsonb, 'active'),
    ('pro', 'Pro', 99, 30, '{"base_bot": true, "basic_ai": true, "broadcast": true, "group_chat": true}'::jsonb, 'active')
ON CONFLICT (plan_code) DO NOTHING;

ALTER TABLE merchants ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchant_subscriptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE payment_orders ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchant_features ENABLE ROW LEVEL SECURITY;
ALTER TABLE merchant_ai_quota ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_audit_logs ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS merchants_platform_or_member ON merchants;
CREATE POLICY merchants_platform_or_member ON merchants
FOR SELECT USING (
    EXISTS (SELECT 1 FROM saas_user_roles r WHERE r.user_id = auth.uid() AND r.role = 'platform_admin')
    OR EXISTS (SELECT 1 FROM tenant_memberships m WHERE m.user_id = auth.uid() AND m.tenant_id = merchants.tenant_id)
);

DROP POLICY IF EXISTS tenant_memberships_self_or_platform ON tenant_memberships;
CREATE POLICY tenant_memberships_self_or_platform ON tenant_memberships
FOR SELECT USING (
    user_id = auth.uid()
    OR EXISTS (SELECT 1 FROM saas_user_roles r WHERE r.user_id = auth.uid() AND r.role = 'platform_admin')
);
