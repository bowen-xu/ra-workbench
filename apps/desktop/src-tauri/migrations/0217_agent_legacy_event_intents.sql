-- A new import claim records the exact event intent before appending any batch.
-- Existing pending imports have no such proof and are deliberately not backfilled.
alter table agent_paper_reading_history_migrations add column event_intent_required integer not null default 0 check(event_intent_required in (0, 1));
create table agent_legacy_event_intents (
    conversation_id text primary key not null references agent_conversations(id) on delete cascade,
    paper_id text not null,
    legacy_session_id text not null,
    epoch text not null,
    session_json text not null check(json_valid(session_json) and json_type(session_json) = 'object'),
    turns_json text not null check(json_valid(turns_json) and json_type(turns_json) = 'array'),
    source_sha256 text not null check(length(source_sha256) = 64),
    event_count integer not null check(event_count >= 0),
    payloads_json text not null check(json_valid(payloads_json) and json_type(payloads_json) = 'array'),
    payloads_sha256 text not null check(length(payloads_sha256) = 64)
);
