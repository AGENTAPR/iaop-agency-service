# Future Service Adoption Blueprint

For each IAOP service: copy the crate layout, migration conventions,
transactional command pattern, idempotency header handling, outbox/audit
emit, permission enum + require(), problem-details mapping, and the
db_tests.rs harness. Consume `iaop-security` (pinned SHA) instead of the
local JWKS validator once the shared crate is proven.

| Service | Reuse | Domain-specific |
|---|---|---|
| Recruitment | all infra patterns | candidate pipeline aggregates/events |
| Onboarding | same | onboarding cases, approvals (Flowable) |
| Eligibility | same | rule evaluation, licenses |
| Sales | same | sales contracts, premium refs |
| Portfolio | same | policy holdings refs |
| Commission | same + temporal patterns | commission hierarchies (distinct from reporting!) |
| Performance | same | metrics/targets |
| Compliance | same | regulatory cases, audit exports |

Each service owns its DB (`<svc>_db`), its events (`<svc>.*.v1`), its
permissions (`<svc>:*`), and validates Agent IDs as external refs —
never cross-service FKs or shared domain entities.
