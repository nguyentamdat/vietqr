CREATE TABLE api_keys (
    id BIGSERIAL PRIMARY KEY,
    shop_id BIGINT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ NULL
);
