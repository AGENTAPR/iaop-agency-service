-- Durable idempotency for mutation endpoints (same contract as Agent Service).
CREATE TABLE IF NOT EXISTS idempotency_records (
    id                  UUID PRIMARY KEY,
    idempotency_key     VARCHAR(255) NOT NULL,
    principal_scope     VARCHAR(255) NOT NULL,
    operation           VARCHAR(120) NOT NULL,
    request_fingerprint VARCHAR(128) NOT NULL,
    response_status     INTEGER,
    response_body       JSONB,
    state               VARCHAR(32) NOT NULL DEFAULT 'IN_PROGRESS'
                        CHECK (state IN ('IN_PROGRESS','COMPLETED','FAILED')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ,
    UNIQUE (principal_scope, operation, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idempotency_records_expires_idx
    ON idempotency_records (expires_at) WHERE expires_at IS NOT NULL;
