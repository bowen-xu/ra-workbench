-- Complete replacement windows are separate from immutable conversation events.
-- Installing a window never deletes evidence or advances the journal watermark.
create table agent_context_checkpoints (
    id text primary key not null,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    turn_id text not null,
    source_watermark integer not null check(source_watermark > 0),
    previous_window_id text,
    schema_version integer not null check(schema_version = 1),
    checkpoint_json text not null check(json_valid(checkpoint_json)),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    unique(conversation_id, id)
);
create table agent_active_context_windows (
    conversation_id text primary key not null references agent_conversations(id) on delete cascade,
    window_id text not null,
    foreign key(conversation_id, window_id) references agent_context_checkpoints(conversation_id, id)
);
create index agent_context_checkpoints_history
    on agent_context_checkpoints(conversation_id, source_watermark, id);
create trigger agent_context_checkpoints_immutable
before update on agent_context_checkpoints
begin
    select raise(abort, 'context checkpoints are immutable');
end;
