-- A queued hook keeps its producing window in the audit, while its context
-- belongs to the active descendant window in which it is actually delivered.
drop trigger agent_compact_hook_contexts_source;
create trigger agent_compact_hook_contexts_source
before insert on agent_compact_hook_contexts
begin
    select case when not exists (
        select 1 from agent_compaction_hook_audit a
        where a.seq=new.audit_seq and a.conversation_id=new.conversation_id
          and a.turn_id=new.turn_id
          and a.stage='session_start_compact' and a.notification='completed'
          and a.outcome in ('continue','stop')
          and exists (
            with recursive ancestors(id) as (
                select new.window_id
                union
                select c.previous_window_id
                from agent_context_checkpoints c join ancestors parent on c.id=parent.id
                where c.conversation_id=new.conversation_id and c.previous_window_id is not null
            )
            select 1 from ancestors where id=a.operation_id
          )
    ) then raise(abort,'context lacks completed SessionStart') end;
end;
