-- A conforming server schema: tenant accounts and their entries, and one settings row.
CREATE TABLE account (
    id uuid PRIMARY KEY,
    tenant uuid NOT NULL,
    name text NOT NULL,
    created_at timestamptz NOT NULL,
    UNIQUE (tenant, name)
);

CREATE INDEX account_page ON account (tenant, created_at, id);

COMMENT ON TABLE account IS 'Retention: until the tenant deletes the account.';

CREATE TABLE entry (
    id uuid PRIMARY KEY,
    tenant uuid NOT NULL,
    account uuid NOT NULL REFERENCES account (id),
    amount bigint NOT NULL,
    recorded_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    note text
);

CREATE INDEX entry_account ON entry (account, recorded_at);
CREATE INDEX entry_expiry ON entry (tenant, expires_at);
CREATE INDEX entry_sweep ON entry (expires_at);

COMMENT ON TABLE entry IS 'Retention: until expires_at, removed in batches.';

CREATE TABLE setting (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    value text NOT NULL
);

COMMENT ON TABLE setting IS 'Retention: one row for the life of the installation.';
