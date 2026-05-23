ALTER TABLE merchants ADD COLUMN IF NOT EXISTS legacy_merchant_id bigint UNIQUE;
ALTER TABLE merchants ALTER COLUMN owner_user_id DROP NOT NULL;
ALTER TABLE merchants DROP CONSTRAINT IF EXISTS merchants_owner_user_id_fkey;
ALTER TABLE merchants
    ADD CONSTRAINT merchants_owner_user_id_fkey
    FOREIGN KEY (owner_user_id) REFERENCES auth.users(id) ON DELETE SET NULL;
ALTER TABLE merchants ADD COLUMN IF NOT EXISTS owner_username text;
ALTER TABLE merchants ADD COLUMN IF NOT EXISTS subscription_expires_at timestamptz;

CREATE INDEX IF NOT EXISTS idx_merchants_owner_tg_user ON merchants(owner_tg_user_id);
