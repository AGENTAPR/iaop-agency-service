# Domain Model

Aggregates (each owns its invariants; IDs are UUIDv7 `EntityId`):

- **Agency** — code (1..32, unique), name, status ACTIVE→SUSPENDED→TERMINATED.
- **OrgUnit** — belongs to Agency; kind REGION/BRANCH/DEPARTMENT/TEAM;
  status ACTIVE/INACTIVE. Parent is NOT a field — hierarchy lives in
  temporal `OrgRelationship` edges.
- **OrgRelationship** — temporal edge (parent_id, child_id,
  effective_from, effective_to exclusive). `covers(date)`;
  `would_create_cycle` guards reparenting.
- **Assignment** — agent_id (external ref to Agent Registry), unit_id,
  PRIMARY/SECONDARY, PENDING/ACTIVE/ENDED/CANCELLED, effective window.
- **Transfer** — record of agent move; preserves history.
- **LeadershipAppointment** — unit+role+agent, effective-dated;
  one open appointment per (unit, role) enforced by partial unique index.

Reporting hierarchy (OrgRelationship) is deliberately separate from
compensation hierarchy — Agency Service does not model compensation.

Temporal semantics: `[effective_from, effective_to)`, `effective_to`
NULL = open-ended. Backdated corrections are allowed and preserved as
immutable history (no updates to closed edges).
