# E2E Test Results (local, 2026-11-13)

Service: debug build, `AUTH_MODE=dev`, `DATABASE_URL` → docker
`local-postgresql16`/`agency_db`. Migrations applied by service at boot.

Verified via curl:
- POST /agencies → 201; repeat with same `Idempotency-Key` → stored
  replay (identical body, different key order = JSONB replay).
- POST organization-units (Region→BranchA/BranchB) → 201s; ancestors of
  BranchA = [Region].
- POST agent-assignments (synthetic agent UUID) → ACTIVE.
- POST agency-transfers BA→BB eff. 2026-04-01 → 201; as_of queries show
  correct windows.
- PATCH reparent Region under BranchB → 409 CONFLICT (cycle rejected).
- GET /agencies without token → 401.
- DB evidence: 6 outbox_events + 6 agency_audit_events rows matching the
  successful mutations; failed cycle produced none (atomicity).

NOT verified: live Keycloak JWT, broker delivery, browser flow,
restart-persistence (implied by durable PG but not explicitly retested).
