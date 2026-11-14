# Test Strategy

| Layer | Where | Status |
|---|---|---|
| Domain unit | `crates/domain/src/*.rs` tests | 16 tests PASS |
| Application/infra unit | messaging/outbox/idempotency module tests | PASS |
| API smoke | `tests/integration/main.rs` | 2 tests PASS |
| Real PostgreSQL | `tests/db_tests.rs` (DATABASE_URL gated) | 8 tests PASS on PG16 |
| Concurrency | `concurrent_reparent_one_wins_or_both_valid` | PASS (caught+fixed real cycle race) |
| Temporal | `temporal_hierarchy_as_of`, assignment as-of in overlap test | PASS |
| Idempotency | `idempotency_replay_and_conflict` + E2E header replay | PASS |
| Security negative | JWT negative matrix | NOT_VERIFIED (dev-bypass only locally) |
| E2E | manual curl scenario vs live service+PG | PASS (see E2E_TEST_RESULTS) |
| Performance | — | NOT_VERIFIED |

Real-DB tests skip silently when `DATABASE_URL` is unset — they were
executed locally against `postgres:16` container `agency_db`.
