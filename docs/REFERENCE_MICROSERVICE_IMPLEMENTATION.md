# Reference Microservice Implementation — reusable patterns

1. **Workspace**: `crates/{domain,application,infrastructure,api}` + thin
   `src/main.rs` composition root. Domain has zero infra deps.
2. **DDD**: aggregates own invariants; effective-dated relations as
   separate temporal records, not fields. UUIDv7 ids, version counters
   for optimistic locking.
3. **Ports**: `OrgStore` trait — reads on pool, writes tx-scoped
   (`Tx<'_>` = sqlx Transaction). Application may reference the sqlx tx
   type (same precedent as the idempotency port).
4. **Transactions**: one tx per command = state + outbox + audit +
   idempotency reserve/complete. Routes begin/commit; rollback on error.
5. **Hierarchy safety**: cycle check in app +
   `LOCK TABLE ... SHARE ROW EXCLUSIVE` for mutual exclusion of
   structural writes.
6. **Idempotency**: `Idempotency-Key` header → durable reserve inside the
   business tx; replay returns stored response; fingerprint mismatch → 409.
7. **Security**: Bearer JWT extractor (JWKS or gated dev bypass),
   `permissions::require` per command, 401/403/503 mapping, fail-closed.
8. **Errors**: `DomainError` → `AppError` → problem-details JSON; no
   infra leakage.
9. **Migrations**: sequential `YYYYMMDDHHMMSS_name.sql`, sqlx migrate at
   boot; partial unique indexes for temporal invariants.
10. **Events**: versioned `domain.entity-action.v1` names into outbox
    in-tx; publisher = SKIP LOCKED + backoff + LogTransport until broker
    decided.
11. **Git/CI**: `development` branch for work; workflows trigger ONLY on
    push to `main`.
12. **Tests**: domain unit + `DATABASE_URL`-gated real-PG integration
    incl. concurrency and temporal scenarios.

Domain-specific (do NOT share): aggregates, business rules, event
payloads, permission names. Shared-crate candidates: security/JWKS,
outbox publisher, idempotency store, health/middleware — via
`iaop-rust-common` pinned SHA.
