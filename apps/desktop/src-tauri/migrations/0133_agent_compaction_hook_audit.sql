-- Append-only identities/status; no hook stdout, credentials or context text.
create table agent_compaction_hook_audit (
    seq integer primary key autoincrement,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    operation_id text not null,
    turn_id text not null,
    stage text not null check(stage in ('pre_compact','post_compact','session_start_compact')),
    phase text not null check(phase in ('manual','pre_turn','mid_turn')),
    model_json text not null check(json_valid(model_json) and length(model_json) <= 16384),
    notification text not null check(notification in ('started','completed')),
    outcome text check(outcome in ('continue','stop','failed','canceled')),
    recorded_at_ms integer not null check(recorded_at_ms >= 0),
    unique(conversation_id,operation_id,turn_id,stage,notification),
    check((notification='started' and outcome is null) or
          (notification='completed' and outcome is not null))
);
create trigger agent_compaction_hook_audit_completion
before insert on agent_compaction_hook_audit
when new.notification='completed'
begin
    select case when not exists (
        select 1 from agent_compaction_hook_audit s
        where s.conversation_id=new.conversation_id and s.operation_id=new.operation_id
          and s.turn_id=new.turn_id and s.stage=new.stage and s.notification='started'
          and s.phase=new.phase and s.model_json=new.model_json
    ) then raise(abort,'hook completion lacks matching start') end;
end;
create trigger agent_compaction_hook_audit_no_update
before update on agent_compaction_hook_audit
begin select raise(abort,'hook audit is immutable'); end;
