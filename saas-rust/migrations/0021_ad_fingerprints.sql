BEGIN;

CREATE TABLE IF NOT EXISTS groups.ad_text_fingerprints (
    tenant_id text NOT NULL DEFAULT 'default',
    text_hash text NOT NULL,
    normalized_text text NOT NULL,
    representative_text text,
    source_msg_id bigint,
    confirmed_by text NOT NULL DEFAULT 'agent',
    confirmed_count integer NOT NULL DEFAULT 1,
    hit_count integer NOT NULL DEFAULT 0,
    first_confirmed_at timestamptz NOT NULL DEFAULT now(),
    last_confirmed_at timestamptz NOT NULL DEFAULT now(),
    last_hit_at timestamptz,
    PRIMARY KEY (tenant_id, text_hash)
);

CREATE INDEX IF NOT EXISTS idx_ad_text_fingerprints_hash
    ON groups.ad_text_fingerprints (text_hash);

CREATE TABLE IF NOT EXISTS groups.ad_sender_profiles (
    tenant_id text NOT NULL DEFAULT 'default',
    sender_id bigint NOT NULL,
    confirmed_ad_count integer NOT NULL DEFAULT 0,
    last_text_hash text,
    first_confirmed_at timestamptz NOT NULL DEFAULT now(),
    last_confirmed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, sender_id)
);

COMMIT;
