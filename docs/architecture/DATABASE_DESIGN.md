# Database Design (agency_db, Postgres 16)

- UUID PKs (UUIDv7 app-generated), TIMESTAMPTZ instants, DATE business dates.
- `agencies` — code UNIQUE, status CHECK, version>0.
- `organizational_units` — FK agency, UNIQUE(agency_id, code), kind/status CHECKs.
- `organization_relationships` — FKs to units, `parent_id<>child_id`,
  `effective_to>effective_from`, partial unique index = one OPEN parent
  per child + one open edge per (parent,child).
- `agent_assignments` — agent_id is an external reference (no FK to
  Agent Registry DB). Overlap of PRIMARY assignments is enforced in the
  application layer inside the tx (documented policy: one open primary).
- `agency_transfers`, `leadership_appointments` (partial unique open
  role index), `agency_audit_events`, `outbox_events` (extended with
  source/correlation_id/causation_id), `inbox_events`,
  `idempotency_records` (UNIQUE principal_scope+operation+key).

Migrations: `migrations/2026111300000{1..6}_*.sql`, applied by
`sqlx::migrate!` at startup. Verified on postgres:16 container.
