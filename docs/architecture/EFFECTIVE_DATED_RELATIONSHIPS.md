# Effective-Dated Relationships

All structural relations use business dates (`DATE`), not timestamps:

| Relation | Table | Window |
|---|---|---|
| Unit→parent | organization_relationships | [from, to) |
| Agent→unit | agent_assignments | [from, to) |
| Leader→unit/role | leadership_appointments | [from, to) |

Queries take `?as_of=YYYY-MM-DD` (defaults to today). Historical rows are
never rewritten: `change_parent` closes the open edge at the effective
date and inserts a new one; `transfer_agent` ends the current assignment
and inserts the new one plus a transfer record.

Bitemporal: NOT implemented — `created_at`/`updated_at` provide recorded
time for audit; reconstructing "what the system believed then" beyond
that is a documented non-goal.

Concurrency: hierarchy mutations take
`LOCK TABLE organization_relationships IN SHARE ROW EXCLUSIVE MODE`
inside the business tx — cycle check + edge writes are atomic against
concurrent reparents (verified by concurrent test).
