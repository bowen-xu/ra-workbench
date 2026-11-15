-- Sampling observations are window/model/rollback-bound, never billed totals.
-- Existing migrations are immutable; old windows intentionally start unknown.
create table agent_context_usage_observations (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    model_json text not null check(json_valid(model_json) and length(model_json) <= 4096),
    window_id text,
    window_key text not null check(window_key = coalesce(window_id, '')),
    rollback_epoch integer not null check(rollback_epoch >= 0),
    source_ordinal integer not null check(source_ordinal >= 0),
    sample_json text not null check(json_valid(sample_json) and length(sample_json) <= 8192),
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    primary key(conversation_id, model_json, window_key, rollback_epoch),
    foreign key(conversation_id, window_id)
        references agent_context_checkpoints(conversation_id, id) on delete cascade,
    foreign key(conversation_id, source_ordinal)
        references agent_items(conversation_id, ordinal) on delete cascade
);
