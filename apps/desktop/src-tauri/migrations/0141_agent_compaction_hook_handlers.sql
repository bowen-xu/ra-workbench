-- Per-command evidence references immutable aggregate identity/status records.
create table agent_compaction_hook_handlers (
    audit_seq integer not null references agent_compaction_hook_audit(seq) on delete cascade,
    handler_id text not null check(length(handler_id)=36),
    display_order integer not null check(display_order between 0 and 31),
    outcome text check(outcome in ('continue','stop','failed','canceled')),
    primary key(audit_seq,handler_id),
    unique(audit_seq,display_order)
);
create trigger agent_compaction_hook_handlers_validate
before insert on agent_compaction_hook_handlers
begin
    select case when not exists (
        select 1 from agent_compaction_hook_audit a where a.seq=new.audit_seq
        and ((a.notification='started' and new.outcome is null) or
             (a.notification='completed' and new.outcome is not null))
    ) then raise(abort,'handler status differs from aggregate phase') end;
    select case when new.outcome is not null and not exists (
        select 1 from agent_compaction_hook_audit a
        join agent_compaction_hook_audit s
          on s.conversation_id=a.conversation_id and s.operation_id=a.operation_id
          and s.turn_id=a.turn_id and s.stage=a.stage and s.phase=a.phase
          and s.model_json=a.model_json and s.notification='started'
        join agent_compaction_hook_handlers h on h.audit_seq=s.seq
        where a.seq=new.audit_seq and h.handler_id=new.handler_id
          and h.display_order=new.display_order
    ) then raise(abort,'handler completion lacks matching start') end;
end;
create trigger agent_compaction_hook_handlers_no_update
before update on agent_compaction_hook_handlers
begin select raise(abort,'handler audit is immutable'); end;
