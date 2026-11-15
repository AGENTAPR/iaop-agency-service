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

---

# Addendum — 2026-11-15 (second pass): BFF contract fix, identity resolution, lifecycle endpoints, contract resync

Base: `development` @ `0805728` (post-merge HEAD). All changes merged via PRs #16–#19 under `protect-development`.

## What changed since the report above

| Area | Before | After | Evidence |
|---|---|---|---|
| `GET /agents/{id}/current-assignment` | Returned JSON array of all as-of assignments; OpenAPI declared single `AssignmentDto`; BFF expected `Option<...>` | Returns single PRIMARY+ACTIVE `AssignmentDto`, 404 when absent | `queries::agent_current_assignment`; `cargo check` PASSED (Docker rust:slim) |
| Agent BFF `AgencyClient` | `#[serde(rename_all="camelCase")]` on all view DTOs — deserialization of snake_case upstream payloads failed; `get_ancestors` expected `Vec<OrgUnitView>` but upstream returns `[string]` IDs | snake_case DTOs; `get_ancestors -> Vec<Uuid>` resolved via `get_unit` in hierarchy + workspace flows; 7 consumer contract tests | `cargo test -p iaop_agent_bff-infrastructure` 23/23 PASSED |
| Agent identity resolution | Claim-less agent tokens -> 403 (fail-closed, no path to scoped access) | Server-side resolution: own `(issuer, subject)` -> Agent Service `identity-links/resolve`, service-account auth, 60s TTL cache, staff exempt, 503 on registry outage | `auth.rs`, `agent_validation.rs`, `ports.rs`; `cargo check` PASSED |
| Agency lifecycle | Domain methods existed, no commands/endpoints | `POST /api/v1/agencies/{id}/lifecycle` (SUSPEND/REINSTATE/TERMINATE), global-write authz, optimistic concurrency, outbox `agency.status-changed.v1` + audit | `commands::transition_agency`, route + OpenAPI registered |
| Unit lifecycle | Domain methods existed, no endpoints | `POST /api/v1/organization-units/{id}/lifecycle` (ACTIVATE/DEACTIVATE), unit scope + HierarchyManage, outbox `agency.unit-status-changed.v1` | `commands::transition_unit` |
| OpenAPI | 15 paths | 17 paths (lifecycle x2); deterministic export via `dump_openapi_spec` test | `target/agency_service_openapi.json` generated & synced |
| Central contract | 15-path spec @ old SHA | 17-path spec @ `0805728`, `sourceCommit`/`apiVersion 0.2.0` pinned | `iaop-api-contracts` PR #16 MERGED; validator SUCCESS (19/19) |

## Test results this pass

- `cargo check --workspace` (iaop-agency-service, Docker `rust:slim-bookworm`): PASSED
- `cargo test -p iaop_agency_service-api`: PASSED (incl. openapi path/security + dump)
- `cargo test --workspace --lib` (agency): 8 passed, 1 failed — `kafka_delivery_succeeds_against_live_broker` requires live broker (env-dependent; previously verified 9/9 on InstaBox)
- `cargo check -p iaop_agent_bff-infrastructure -p iaop_agent_bff-api`: PASSED
- `cargo test -p iaop_agent_bff-infrastructure --lib`: 23/23 PASSED (7 new contract tests)
- `python tools/validate_contracts.py --env development`: SUCCESS, 19 docs / 19 services

## Still open (honest status)

- **InstaBox DEV redeploy NOT performed** — `instabox` CLI unavailable in this environment; deployed image still runs pre-lifecycle code. Runtime verification of new endpoints + identity resolution is PENDING.
- Live E2E agent-token -> scoped 200 flow unverified (needs deployed build + linked agent identity).
- Transfer approval workflow: still `REQUIREMENT_UNRESOLVED` (immediate-effect only, documented).
- Promotion/rank-change: no approved mapping exists — not implemented by design.
- DB integration tests for new lifecycle endpoints not added (domain unit tests cover transitions; endpoint-level DB tests pending).
- Performance benchmarks, Kafka failure-injection matrix: not rerun this pass.
- Scalar portal content verification blocked by SSO (401 without session).

## Verdict this pass

`CONDITIONALLY DEV READY` — P0 contract/identity defects fixed and compile+unit verified; deployment and live E2E verification outstanding.

---

## Addendum 2 — InstaBox DEV live verification (2026-11-15)

Deployed and verified on `iaop-keycloak-dev` (`/data/iaop-dev/compose`).

### Source baseline

| Repo | Deployed SHA |
|---|---|
| iaop-agency-service | `a57afbb` (incl. PR #21 idempotency fingerprint fix) |
| iaop-agent-bff | `fe00193` (incl. PR #18 agent client contract fix) |
| iaop-infrastructure | compose env PR #13 (service-account wiring) |

### Defects found and fixed during live verification

1. **Idempotency fingerprint overflow (P0, new)** — `request_fingerprint`
   stored raw JSON bodies into `varchar(128)`; any POST body >128 chars
   returned HTTP 500. Reproduced live, root-caused via PostgreSQL log
   (`value too long for type character varying(128)`). Fixed by storing
   SHA-256 hex (64 chars) — agency PR #21, deployed `a57afbb`.
2. **Agent client DTO mismatch (P0, new)** — BFF `agent_client.rs` used
   `camelCase` serde renames for `AgentView`/`AppointmentView`/
   `IdentityLinkView` while Agent Service emits snake_case; identity-link
   resolution failed at login (`error decoding response body`), leaving
   `agentId=null` and candidate-only workspace. Also corrected
   `AppointmentView` types (Option refs, NaiveDate). BFF PR #18,
   deployed `fe00193`. 5 new contract tests PASSED.

### Live evidence (DEV, real Keycloak OIDC code+PKCE flow)

- `POST /api/v1/agent-assignments` (e2e-agent-admin, platform admin):
  **201** — assignment `01a1debf-…` created for agent
  `11111111-…` / unit `01a1dd66-…` (previously 500).
- Claim-less user token (`demo.agent-1`, sub `4a6ba5c0-…`):
  `GET /agents/{id}/current-assignment` → **200** single `AssignmentDto`
  object (identity resolved via Agent Service `identity-links/resolve`,
  service-account credentials from compose env).
- BFF session login → callback **303**, `iaop_session` cookie issued.
- `GET /api/v1/me` → **200**: `agentId`, `agentCode=AGT-QA-0001`,
  `lifecycleStatus=ACTIVE`, `activeWorkspace=<unit id>`.
- `GET /api/v1/me/workspaces` → **200**: real workspace "Unit R"
  (agencyCode `R`, role PRIMARY) — no longer candidate-only.
- `GET /api/v1/agency/hierarchy` → **200**: ancestor IDs resolved to
  unit objects (`agencyId=R`, `agencyName=Unit R`, level REGION).
- `GET /api/v1/agency/my-team` → **200** empty (agent has no reports).
- Negative: BFF `/api/v1/me` without cookie → **401**; forged cookie →
  **401**; Agency `/api/v1/organization-units` no-auth → **401**.

### Test commands executed

- `cargo check --workspace` (Docker `rust:slim-bookworm`): PASSED
  (agency `a57afbb`, bff `fe00193`).
- `cargo test -p iaop_agent_bff-infrastructure agent_client`: 5/5 PASSED.
- Live curl evidence above against deployed containers.

### Still open

- Transfer approval workflow: `REQUIREMENT_UNRESOLVED`.
- Promotion/rank-change: no approved mapping — not implemented.
- Kafka failure-injection matrix, DB concurrency suite, performance
  benchmarks: not rerun this pass.
- Scalar SSO-gated content verification still pending.
- Lifecycle endpoints deployed but not yet exercised live.

## Verdict (updated)

`CONDITIONALLY DEV READY` — the full Keycloak → BFF session → Agency
Service → Agent Service identity path is now verified live on DEV with a
real agent; remaining gaps are business-scope decisions and unrun
non-functional suites, not P0 defects.

---

## Addendum 3 — Scalar portal sync + error-mapping fix (2026-11-15, continued)

### Central Scalar / Developer Portal — VERIFIED

- Portal `iaop-dev-api-docs` (Rust portal in `iaop-api-contracts/portal`)
  rebuilt from `development@7b38a77`; catalog baked at image build.
- `GET /api/v1/catalog/backend` (portal session) → Agency Service entry:
  `sourceCommit=a57afbb3`, `apiVersion=0.2.0`, `verificationStatus=e2e_verified`,
  `maturity=implemented_verified`, `deploymentStatus=deployed`.
- `GET /api/v1/services/agency-service/spec` → 307 → canonical
  `specs/development/agency-service/openapi.json` (17 paths / 20 ops).
- `GET /reference/backend/agency-service` → 200, Scalar UI loads; spec
  `servers` rewritten to `https://8083-…instabox.tech/dev-api/agency`.
- **Route parity**: live `/api/docs/openapi.json` vs canonical spec —
  20/20 operations identical (0 live-only, 0 canon-only).
- **Dev testing proxy verified**: `GET /dev-api/agency/api/v1/agents/{id}/current-assignment`
  - no portal session → 401; portal session only → 401 (upstream problem+json);
  - portal session + `Authorization: Bearer <iaop-agency-api JWT>` → **200**
    with live assignment data. Domain API still requires its own audience
    token — portal session alone is correctly insufficient.

### Kafka / outbox — VERIFIED (live)

- `outbox_events`: 102 rows; `agency.agent-assigned.v1` (created 10:27)
  `published_at` set after 2 attempts — first attempt hit
  `UnknownTopicOrPartition` (topic auto-create lag), retry succeeded.
  Confirms at-least-once outbox + bounded retry + eventual delivery.

### Defect fixed: domain error mapping (P1, PR #23, deployed `ac0d3f3`)

- `respond()` in `crates/api/src/routes.rs` lacked arms for
  `AppError::Domain`, `AgentNotFound`, `PreconditionFailed` → all fell to
  `500 INTERNAL`. Reproduced live: invalid unit lifecycle transition
  returned 500.
- After fix (live-verified):
  - invalid transition (ACTIVATE on ACTIVE unit) → **409 CONFLICT**
  - stale `expected_version` → **412 PRECONDITION_FAILED**
  - missing `expected_version` → **422** validation
- Note: `error_mapping.rs` (`ApiError::from(AppError)`) already mapped
  these correctly; only the legacy `respond()` path was wrong.

### Deployment parity

| Component | Deployed SHA | development HEAD |
|---|---|---|
| agency-service | `ac0d3f3` | `ac0d3f3` (code; docs addendum pending) |
| agent-bff | `fe00193` | `fe00193` |
| api-docs (portal) | image @ `7b38a77` | `7b38a77` |

### Verdict (updated)

`CONDITIONALLY DEV READY` — all P0 integration items now have live DEV
evidence (BFF contract, identity resolution, resource authz, runtime
OpenAPI parity, central Scalar catalog + testing proxy, security
negatives, outbox delivery). Remaining gaps are P1 business-scope
(transfer approval `REQUIREMENT_UNRESOLVED`, promotion mapping) and
unrun non-functional suites (DB concurrency, perf benchmarks, Kafka
failure-injection matrix).
