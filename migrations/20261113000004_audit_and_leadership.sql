-- Audit trail for organizational mutations + leadership appointments.

CREATE TABLE agency_audit_events (
    id              UUID PRIMARY KEY,
    actor_subject   TEXT NOT NULL,
    action          TEXT NOT NULL,
    aggregate_type  TEXT NOT NULL,
    aggregate_id    UUID NOT NULL,
    previous_state  JSONB,
    new_state       JSONB,
    effective_date  DATE,
    reason          TEXT,
    correlation_id  UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_aggregate ON agency_audit_events (aggregate_type, aggregate_id, created_at);
CREATE INDEX idx_audit_actor     ON agency_audit_events (actor_subject, created_at);

CREATE TABLE leadership_appointments (
    id              UUID PRIMARY KEY,
    unit_id         UUID NOT NULL REFERENCES organizational_units(id),
    agent_id        UUID NOT NULL,           -- external ref: Agent Registry
    role            TEXT NOT NULL
                    CHECK (role IN ('UNIT_HEAD','REGIONAL_MANAGER','AGENCY_DIRECTOR')),
    status          TEXT NOT NULL DEFAULT 'ACTIVE'
                    CHECK (status IN ('PENDING','ACTIVE','ENDED','CANCELLED')),
    effective_from  DATE NOT NULL,
    effective_to    DATE,
    version         BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (effective_to IS NULL OR effective_to > effective_from)
);
-- At most one open leadership appointment per (unit, role).
CREATE UNIQUE INDEX idx_leadership_open
    ON leadership_appointments (unit_id, role) WHERE effective_to IS NULL;
CREATE INDEX idx_leadership_agent ON leadership_appointments (agent_id, effective_from);
