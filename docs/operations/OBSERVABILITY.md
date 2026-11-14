# Observability

- `tracing` JSON logs, request-context middleware (correlation id).
- `/health/live` (process), `/health/ready` (DB ping when configured).
- Outbox backlog observable via `outbox_events WHERE published_at IS NULL`;
  `outbox::backlog_depth(pool)` helper exists.
- No PII in logs; tokens never logged.
- OpenTelemetry metrics/tracing hooks: NOT implemented (PARTIAL).
