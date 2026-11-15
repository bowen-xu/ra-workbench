-- Preserve complete schema-1 windows and their active pointers while allowing
-- schema-2 MidTurn windows with independently identified canonical context.
-- The migration owner disables foreign keys and verifies them before commit.
create table agent_context_checkpoints_0127 (
    id text primary key not null,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    turn_id text not null,
    source_watermark integer not null check(source_watermark >= 0),
    previous_window_id text,
    schema_version integer not null check(schema_version in (1, 2)),
    checkpoint_json text not null check(json_valid(checkpoint_json)),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    unique(conversation_id, id)
);
insert into agent_context_checkpoints_0127(id, conversation_id, turn_id,
    source_watermark, previous_window_id, schema_version, checkpoint_json, created_at)
select id, conversation_id, turn_id, source_watermark, previous_window_id,
    schema_version, checkpoint_json, created_at from agent_context_checkpoints;
drop table agent_context_checkpoints;
alter table agent_context_checkpoints_0127 rename to agent_context_checkpoints;
create index agent_context_checkpoints_history
    on agent_context_checkpoints(conversation_id, source_watermark, id);
create trigger agent_context_checkpoints_immutable
before update on agent_context_checkpoints
begin
    select raise(abort, 'context checkpoints are immutable');
end;
