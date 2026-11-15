-- Independent fork snapshots: provenance is not a new hook execution.
create table agent_inherited_hook_contexts (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    window_id text not null references agent_context_checkpoints(id) on delete cascade,
    source_audit_seq integer not null check(source_audit_seq > 0),
    source_conversation_id text not null,
    turn_id text not null,
    boundary_seq integer not null check(boundary_seq > 0),
    boundary_ordinal integer generated always as (boundary_seq - 1) stored,
    contexts_json text not null check(json_valid(contexts_json) and json_type(contexts_json)='array'),
    primary key(conversation_id,window_id,source_audit_seq),
    foreign key(conversation_id,boundary_ordinal) references agent_items(conversation_id,ordinal) on delete cascade
);
create trigger agent_inherited_hook_contexts_no_update
before update on agent_inherited_hook_contexts
begin select raise(abort,'inherited hook contexts are immutable'); end;
