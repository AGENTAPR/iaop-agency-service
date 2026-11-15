# AGENA — Agent & Agency Integration Report

Date: 2026-11-15 · Environment: InstaBox DEV

## A. Source baseline

| Repo | development SHA |
|---|---|
| iaop-agent-service | `f40f0240` |
| iaop-agency-service | `17a33c7d` |
| iaop-agent-bff | `fe001934` |
| iaop-api-contracts | `329fc1dd` |
| iaop-event-contracts | `3e2b3631` |

## B. Verified integration paths (live DEV evidence)

1. **Keycloak → BFF → Agency → Agent (identity)**
   - OIDC code+PKCE login as `demo.agent.1` → BFF callback resolves
     `(iss, sub)` via Agent Service `identity-links/resolve` →
     `agentId=11111111-…` ACTIVE.
   - `/api/v1/me` → agent profile + real workspace; `/me/workspaces` →
     "Unit R"; `/agency/hierarchy` → ancestor unit objects;
     `/agency/my-team` → 200 empty.
   - Agency `current-assignment` for claim-less token → identity
     resolution → single `AssignmentDto` 200.

2. **Agency → Agent Service validation**
   - `POST /agent-assignments` (admin) → 201; `HttpAgentValidator`
     uses `iaop-agency-service` client credentials (compose env wired,
     infra PR #13); fail-closed on outage/non-active agent.

3. **Developer Portal → Agency (testing proxy)**
   - `/dev-api/agency/*` requires portal session AND upstream Bearer
     JWT; verified 200 with valid token, 401 without either.

4. **Contracts**
   - BFF agency client snake_case DTOs + contract tests (PR #17).
   - BFF agent client snake_case DTOs + 5 contract tests (PR #18) —
     fixed login-time identity resolution deserialization.
   - Agency `current-assignment` returns single optional object
     matching OpenAPI (PR #16).
   - Route parity runtime↔canonical: 20/20 operations.
   - Event schemas aligned to producer payloads (event-contracts PR #7).

## C. Security posture

- `AUTH_MODE=dev`, `ACTIVATION_MODE=fixture` → `ENVIRONMENT=local` only
  (agent#17, agency#25; unit-tested for dev/uat/prod/staging).
- Domain errors map correctly (agency#23): 409/412/422 verified live.
- Idempotency fingerprint hashed (agency#21) — fixed live 500s.
- No public route to domain services; only BFF (8082) and portal
  (8083) exposed via instbox edge; agency/agent internal-only.

## D. Database evidence

Isolated PostgreSQL 16 containers, disposable DBs:
- agent-service `db_tests`: 2/2 PASSED.
- agency-service `db_tests`: 10/10 PASSED (incl. concurrent reparent,
  authz negatives, idempotency, temporal hierarchy, outbox fencing).

## E. Kafka evidence

- Agency outbox: 102 events; `agency.agent-assigned.v1` delivered after
  bounded retry; lease/fencing/poison covered by db_tests.
- Event type ↔ schema parity established in event-contracts PR #7
  (9 agency + 8 agent payload schemas; envelope pattern fixed for
  `.v1` names).

## F. Open items (honest)

- Command-level PG lifecycle E2E for agent activation (needs
  appointment + eligibility decision) — MISSING_TEST.
- Live Kafka failure-injection (broker kill, worker restart) —
  NOT_EXECUTED.
- Performance benchmarks — NOT_EXECUTED.
- BUSINESS_POLICY_UNRESOLVED: transfer approval workflow, promotion/
  rank mapping, appointment write-path ownership.

## G. Verdicts

- Agent Service: `CONDITIONALLY DEV READY`
- Agency Service: `CONDITIONALLY DEV READY`
- Integration boundary (Keycloak→BFF→Domain): verified live; remaining
  gaps are test-coverage and business-policy items, not defects.
