-- Hook context is trusted input, separate from status audit and chat events.
create table agent_compact_hook_contexts (
    audit_seq integer primary key references agent_compaction_hook_audit(seq) on delete cascade,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    window_id text not null references agent_context_checkpoints(id) on delete cascade,
    turn_id text not null,
    boundary_seq integer not null check(boundary_seq > 0),
    boundary_ordinal integer generated always as (boundary_seq - 1) stored,
    contexts_json text not null check(json_valid(contexts_json) and json_type(contexts_json)='array'),
    foreign key(conversation_id,boundary_ordinal) references agent_items(conversation_id,ordinal) on delete cascade
);
create index agent_compact_hook_contexts_window on agent_compact_hook_contexts(conversation_id,window_id,boundary_seq);
create trigger agent_compact_hook_contexts_source
before insert on agent_compact_hook_contexts
begin
    select case when not exists (
        select 1 from agent_compaction_hook_audit a
        where a.seq=new.audit_seq and a.conversation_id=new.conversation_id
          and a.operation_id=new.window_id and a.turn_id=new.turn_id
          and a.stage='session_start_compact' and a.notification='completed'
          and a.outcome in ('continue','stop')
    ) then raise(abort,'context lacks completed SessionStart') end;
end;
create trigger agent_compact_hook_contexts_no_update
before update on agent_compact_hook_contexts
begin select raise(abort,'hook contexts are immutable'); end;
