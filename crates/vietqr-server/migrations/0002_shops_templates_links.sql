CREATE TABLE shops (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    bank_bin TEXT NOT NULL,
    account TEXT NOT NULL,
    account_name TEXT NULL,
    plan TEXT NOT NULL DEFAULT 'free',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE api_keys ADD CONSTRAINT api_keys_shop_fk FOREIGN KEY (shop_id) REFERENCES shops(id);

CREATE TABLE templates (
    id BIGSERIAL PRIMARY KEY,
    shop_id BIGINT NOT NULL REFERENCES shops(id),
    name TEXT NOT NULL,
    logo BYTEA NULL,
    fg_color TEXT NOT NULL DEFAULT '#000000',
    bg_color TEXT NOT NULL DEFAULT '#ffffff',
    frame TEXT NOT NULL DEFAULT 'none' CHECK (frame IN ('none', 'border')),
    show_shop_name BOOLEAN NOT NULL DEFAULT false,
    version INTEGER NOT NULL DEFAULT 1,
    UNIQUE (id, shop_id)
);
CREATE TABLE links (
    id BIGSERIAL PRIMARY KEY,
    shop_id BIGINT NOT NULL REFERENCES shops(id),
    slug TEXT NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9-]{3,40}$'),
    template_id BIGINT NULL,
    default_amount BIGINT NULL CHECK (default_amount BETWEEN 1 AND 9999999999999),
    default_memo TEXT NULL,
    active BOOLEAN NOT NULL DEFAULT true,
    FOREIGN KEY (template_id, shop_id) REFERENCES templates(id, shop_id)
);
CREATE INDEX templates_shop_idx ON templates(shop_id);
CREATE INDEX links_shop_idx ON links(shop_id);
