-- Agency Service domain schema: agencies, organizational units,
-- temporal relationships, assignments, transfers.
-- UUIDv7 ids; timestamptz instants; date for business-effective days;
-- temporal semantics use [effective_from, effective_to) — effective_to
-- NULL = open-ended.

CREATE TABLE agencies (
    id          UUID PRIMARY KEY,
    code        TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'ACTIVE'
                CHECK (status IN ('ACTIVE','SUSPENDED','TERMINATED')),
    version     BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE organizational_units (
    id          UUID PRIMARY KEY,
    agency_id   UUID NOT NULL REFERENCES agencies(id),
    code        TEXT NOT NULL,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL
                CHECK (kind IN ('REGION','BRANCH','DEPARTMENT','TEAM')),
    status      TEXT NOT NULL DEFAULT 'ACTIVE'
                CHECK (status IN ('ACTIVE','INACTIVE')),
    version     BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (agency_id, code)
);
CREATE INDEX idx_units_agency ON organizational_units (agency_id);

-- Temporal parent-child edges: a child has at most one OPEN parent edge.
CREATE TABLE organization_relationships (
    id              UUID PRIMARY KEY,
    parent_id       UUID NOT NULL REFERENCES organizational_units(id),
    child_id        UUID NOT NULL REFERENCES organizational_units(id),
    effective_from  DATE NOT NULL,
    effective_to    DATE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (effective_to IS NULL OR effective_to > effective_from),
    CHECK (parent_id <> child_id)
);
-- One open parent per child.
CREATE UNIQUE INDEX idx_rel_open_parent
    ON organization_relationships (child_id) WHERE effective_to IS NULL;
CREATE INDEX idx_rel_parent ON organization_relationships (parent_id);
CREATE INDEX idx_rel_child  ON organization_relationships (child_id);
-- Prevent exact duplicate open edges.
CREATE UNIQUE INDEX idx_rel_unique_open
    ON organization_relationships (parent_id, child_id)
    WHERE effective_to IS NULL;

CREATE TABLE agent_assignments (
    id              UUID PRIMARY KEY,
    agent_id        UUID NOT NULL,  -- external ref: Agent Registry owns identity
    unit_id         UUID NOT NULL REFERENCES organizational_units(id),
    assignment_type TEXT NOT NULL DEFAULT 'PRIMARY'
                    CHECK (assignment_type IN ('PRIMARY','SECONDARY')),
    status          TEXT NOT NULL DEFAULT 'ACTIVE'
                    CHECK (status IN ('PENDING','ACTIVE','ENDED','CANCELLED')),
    effective_from  DATE NOT NULL,
    effective_to    DATE,
    version         BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (effective_to IS NULL OR effective_to > effective_from)
);
CREATE INDEX idx_assign_agent ON agent_assignments (agent_id, effective_from);
CREATE INDEX idx_assign_unit  ON agent_assignments (unit_id);

CREATE TABLE agency_transfers (
    id              UUID PRIMARY KEY,
    agent_id        UUID NOT NULL,
    from_unit_id    UUID REFERENCES organizational_units(id),
    to_unit_id      UUID NOT NULL REFERENCES organizational_units(id),
    effective_from  DATE NOT NULL,
    reason          TEXT,
    status          TEXT NOT NULL DEFAULT 'ACTIVE'
                    CHECK (status IN ('PENDING','ACTIVE','ENDED','CANCELLED')),
    version         BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_transfers_agent ON agency_transfers (agent_id, created_at);
