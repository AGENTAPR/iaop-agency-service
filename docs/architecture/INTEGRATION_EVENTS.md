# Integration Events

Published via transactional outbox (`outbox_events`) in the same tx as
the mutation + audit record. Event types (v1):

- `agency.created.v1`, `agency.organization-unit-created.v1`,
  `agency.hierarchy-changed.v1`, `agency.agent-assigned.v1`,
  `agency.agent-transferred.v1`, `agency.leadership-assigned.v1`,
  `agency.leadership-ended.v1`

Envelope columns: event_id, aggregate_id/type/version, event_type,
event_version, payload(JSONB), occurred_at, source, correlation_id,
published_at, publish_attempts, last_error.

Publisher: `outbox::run_publisher` claims with `FOR UPDATE SKIP LOCKED`,
retries with backoff, terminal failure after max_attempts, graceful
shutdown. Transport currently `LogTransport` (structured log) — broker
adapter pending org decision; at-least-once semantics assumed.

Inbox: `inbox_events` table exists for consumer dedup; Agency Service
consumes no external events yet (no defined use case).
