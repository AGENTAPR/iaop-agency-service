-- Extend outbox envelope with source/correlation/causation per IAOP event contract.
ALTER TABLE outbox_events ADD COLUMN IF NOT EXISTS source TEXT;
ALTER TABLE outbox_events ADD COLUMN IF NOT EXISTS correlation_id UUID;
ALTER TABLE outbox_events ADD COLUMN IF NOT EXISTS causation_id UUID;
