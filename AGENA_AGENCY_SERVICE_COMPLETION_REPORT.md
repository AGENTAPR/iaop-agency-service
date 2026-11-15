# AGENA — Agency Service Completion & Enterprise Readiness Report

Date: 2026-11-15
Repository: `AGENTAPR/iaop-agency-service`
Delivery branch: `feature/agency-resource-authz` (existing PR #13 reused, not merged)
Final HEAD: `147d62a394442e0ea71093c7bfc85a2c1c29b790`
Environment: existing InstaBox DEV (`iaop-keycloak-dev`, machine `m_agq5lpbszgbva3ygddcbarv6ea`)

---

## A. Repository

- Work delivered on `feature/agency-resource-authz` (the branch of existing PR #13). Direct push to `development` was not attempted; branch protection/PR policy was not bypassed. All commits are on the remote feature branch.
- Key commits this phase:
  - `cd9059b` — AG-04/05/06: least-privilege write guards, dev-bypass gating, OpenAPI security scheme
  - `dc7ba29` — utoipa 5 `Modify` addon for `bearerAuth`
  - `1cf6563` — authenticated Agent Service validation (client credentials + token cache) + snake_case response fix
  - `703ac31` — db_tests fixture carries `IAOP_PLATFORM_ADMIN`
  - `147d62a` — pub request types, clippy clean
- Files changed (this phase): `crates/application/src/{authz,commands,queries,principal,permissions}.rs`, `crates/api/src/{routes,openapi,dto}.rs`, `crates/infrastructure/src/{agent_validation,config}.rs`, `src/main.rs`, `tests/db_tests.rs`, DTO `ToSchema` derives across crates.

## B. Capability matrix

| Capability | Implemented | Tested | Deployed | Remaining Issue |
|---|---|---|---|---|
| Agency CRUD + lifecycle guards | Yes | Unit+DB | Yes | Status-transition endpoints for suspend/terminate not exposed (see §10.1 note) |
| Organizational units + hierarchy | Yes | Unit+DB (cycle, reparent, temporal) | Yes | Unit status-update endpoint not exposed |
| Effective-dated assignments | Yes | DB tests (overlap, history) | Yes | — |
| Transfers (immediate-effect) | Yes | DB tests | Yes | **AG-07: approval workflow NOT implemented** — see §C |
| Leadership appointments | Yes | DB tests | Yes | — |
| Resource-level org authorization | Yes | DB negative tests + live 403 | Yes | Scoped-agent path blocked on missing `agent_id` claim (see §F) |
| Historical `as_of` authz separation | Yes | DB tests | Yes | — |
| Transactional outbox + Kafka | Yes | Live broker test + topic/consumer evidence | Yes | Partition 0 only (documented) |
| OpenAPI contract | Yes (15 paths) | Runtime doc verified live | Yes | Scalar catalog entry for agency-service pending verification in portal |
| Agent Service validation | Yes | Live SA-token call verified | Yes | DEV `agents` table was empty; seeded one QA agent + identity link |

## C. Security findings

| Finding | Fix | Test Evidence | Status |
|---|---|---|---|
| AG-01 read queries lacked resource scope | `resolve_scope` + per-record filtering in queries | `resource_authz_negative_tests` (DB), live 403 | FIXED |
| AG-02 commands lacked org authz | `require_global`/`require_agency_scope`/`require_unit_scope` on all mutations | DB tests | FIXED |
| AG-03 `as_of` could restore expired authority | Scope resolved at `Utc::now()`; `as_of` used only for data | `temporal_hierarchy_as_of`, authz tests | FIXED |
| AG-04 broad admin bypass | `GLOBAL_STAFF_ROLES` (read) vs `GLOBAL_WRITE_ROLES` (write) split; auditor cannot mutate | clippy+DB tests; live platform-admin 200 / agent 403 | FIXED |
| AG-05 dev auth bypass | `AUTH_MODE=dev` gated to isolated local env; deployed runs `auth=Jwt` (verified in startup log) | Live: no-auth 401, wrong-aud 401 | FIXED |
| AG-06 empty OpenAPI | utoipa `#[path]` on all 18 handlers + `Modify` security addon | Live `/api/docs/openapi.json`: 15 paths, `bearerAuth` | FIXED |
| AG-07 transfer approval workflow | **NOT implemented** — transfers are immediate-effect authorized commands. Approval states (`REQUESTED/APPROVED/…`) do not exist in the domain model; inventing them would violate "do not invent business policy". Documented as requirements decision needed. | — | OPEN (policy blocker) |
| AG-08 Agent Registry integration | Validator now uses Keycloak client-credentials (`iaop-agency-service` SA client, `agent:read` only) + cached token; fails closed on 401/403/unavailable. DEV DB `agents` was empty — seeded `AGT-QA-0001` + ACTIVE identity link for `demo.agent.1`; SA-token `GET /api/v1/agents/{id}` verified 200 | Live token + HTTP 200 | FIXED (DEV data seeded manually) |
| AG-09 assignment status semantics | Existing `PENDING/ACTIVE/ENDED/CANCELLED` CHECK constraints and temporal queries verified in DB tests | db_tests | VERIFIED |
| AG-10 Kafka/outbox reliability | Outbox publisher with lease/fencing/retry running; 97/101 events `published_at` set; live consumer read real envelope from `iaop.agency.created.v1`; live-broker unit test passes (`KAFKA_BROKERS=127.0.0.1:29092`) | kafka-topics list + console-consumer | VERIFIED |
| AG-11 docs inconsistent | This report + code comments; `docs/` updates deferred (see §I) | — | PARTIAL |
| AG-12 verification evidence | This report §G | — | THIS DOCUMENT |

## D. Database

- Migrations `20261113000001`–`20261114000008` applied (verified via `_sqlx_migrations` on deployed DB; service logs "database migrations applied").
- Constraints present: status CHECKs, temporal overlap rejection, leadership uniqueness, idempotency uniqueness, optimistic concurrency (`version`), outbox lease columns.
- DEV data: 27 agencies, 101 outbox events (97 published).
- `EXPLAIN (ANALYZE)` benchmarking on representative data: **not performed** — flagged as remaining work, not claimed.

## E. API contract

Runtime OpenAPI (`GET /api/docs/openapi.json`, live): **15 paths**, `bearerAuth` security scheme:

```
/api/v1/agencies, /api/v1/agencies/{id}
/api/v1/organization-units, /{id}, /{id}/parent, /{id}/ancestors,
  /{id}/descendants, /{id}/assignments, /{id}/leaders
/api/v1/agent-assignments, /api/v1/agents/{agent_id}/assignments,
  /api/v1/agents/{agent_id}/current-assignment
/api/v1/agency-transfers, /api/v1/leadership-appointments,
  /api/v1/leadership-appointments/{id}/end
```

Every operation has method/path/operationId/summary/request+response schemas/error responses via utoipa. Scalar catalog inclusion for agency-service under Backend APIs: client `iaop-scalar-dev` already has `/reference/backend/agency-service` redirect registered; portal-side catalog entry verification pending (portal repo, separate task).

## F. Integrations

- **Keycloak**: `AUTH_MODE=jwt`, issuer/JWKS/audience (`iaop-agency-api`) validated. Live: valid token → 200; missing audience → 401; no token → 401; authenticated agent without `agent_id` claim → **403 `unlinked principal has no organizational scope`** (fail-closed, correct).
- **Agent Service**: service-account client `iaop-agency-service` (client credentials, `agent:read` role only, audience mapper). Live SA-token `GET http://agent-service:8080/api/v1/agents/{id}` → 200 ACTIVE.
- **PostgreSQL**: `agency_db` on shared `iaop-dev-db`; per-service role `agency_svc`.
- **Kafka**: `EVENT_TRANSPORT=kafka`, brokers `kafka:9092`, 8 agency topics auto-created, partition 0 (ordering-friendly single partition — acceptable for DEV; noted for scale review).

## G. Tests

| Suite | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (0 warnings) |
| `cargo test --workspace` (unit+api) | 33 passed, 0 failed |
| `tests/db_tests.rs` — real PostgreSQL (`agency_db` on InstaBox) | 10 passed (migrations, CRUD, temporal hierarchy, cycle rejection, overlap, transfers, outbox+audit atomicity, optimistic concurrency, idempotency, concurrent reparent, resource-authz negatives) |
| Kafka live-broker test | PASS (`kafka_delivery_succeeds_against_live_broker`) |
| Live API smoke | ready=200, no-auth=401, wrong-aud=401, admin list=200 (real data), agent-no-claim=403 |
| Security negative matrix | Covered in db_tests (`resource_authz_negative_tests`) + live 401/403 evidence; forged-signature/expiry cases covered by JWKS validator unit tests |
| Performance (p50/p95, EXPLAIN) | **NOT RUN** — honest gap |

## H. Deployment

- Source: `feature/agency-resource-authz` @ `147d62a`
- Image: `iaop/agency-service:dev` = `sha256:9eee07723167…` (built on InstaBox from clean worktree `/data/iaop-dev/src/iaop-agency-authz`)
- Deploy: `docker compose up -d agency-service` only; Postgres/Kafka/Keycloak/other services untouched; no volumes reset.
- Startup log confirms: `env=development auth=Jwt activation=FailClosed`, migrations applied, outbox publisher leased, listening `0.0.0.0:8080`.
- Health: `/health/ready` → `{"status":"ready"}`.

## I. Remaining risks / honest gaps

1. **AG-07 transfer approval**: no REQUESTED/APPROVED workflow exists; transfers execute immediately after authorization. Requires a requirements decision before implementing states.
2. **Scoped-agent access is currently unreachable in DEV**: Keycloak does not issue an `agent_id`/`agent_scope` claim, so non-staff agents get 403 (fail-closed — secure, but the scoped path is only exercised in DB tests, not live). Needs a protocol mapper or claims resolver feeding the authoritative Agent Service identity link.
3. **Agency/unit status-update and promotion endpoints** not exposed — policy not defined in domain docs; extension points exist.
4. **Performance benchmarks not run**; N+1 review in scope computation not benchmarked.
5. **InstaBox root disk ~93% full** — operational risk for future builds.
6. Docs (`docs/api.md`, `SECURITY_MODEL.md`, etc.) not fully synchronized this phase.
7. `demo.agent.1` Keycloak password was reset during verification (DEV test user); DEV `agents` table seeded manually with one QA agent — not reproducible via migrations.
8. Direct push to `development` not performed; delivery is on `feature/agency-resource-authz` (PR #13 branch).

## J. Final verdict

**CONDITIONALLY DEV READY**

Core P0 security findings (AG-01…AG-06, AG-08…AG-10) are fixed, tested against real Postgres/Kafka/Keycloak, built, deployed, and verified live. The service fails closed everywhere verified. Blocking items before "VERIFIED": transfer-approval policy decision (AG-07), Keycloak `agent_id` claim issuance for scoped agents, and performance evidence.
