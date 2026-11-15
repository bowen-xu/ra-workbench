-- Stop hook admission and results are immutable facts. An admission does not
-- claim a process started; a missing finish remains explicit after a crash.
create table agent_stop_hook_admissions (
    invocation_id text primary key check(length(invocation_id)=36),
    conversation_id text not null references agent_conversations(id) on delete cascade,
    turn_id text not null check(length(turn_id)=36),
    event_name text not null check(event_name in ('stop','subagent_stop')),
    model_json text not null check(json_valid(model_json) and length(model_json)<=16384),
    boundary_seq integer not null check(boundary_seq>=0),
    admitted_at_ms integer not null check(admitted_at_ms>=0)
);
create index agent_stop_hook_admissions_conversation_turn
    on agent_stop_hook_admissions(conversation_id,turn_id,boundary_seq);
create table agent_stop_hook_handlers (
    invocation_id text not null references agent_stop_hook_admissions(invocation_id) on delete cascade,
    ordinal integer not null check(ordinal between 0 and 31),
    handler_id text not null check(length(handler_id)=36),
    run_id text not null unique check(length(run_id)=36),
    primary key(invocation_id,ordinal),
    unique(invocation_id,handler_id),
    unique(invocation_id,ordinal,run_id)
);
create table agent_stop_hook_results (
    invocation_id text not null,
    ordinal integer not null,
    run_id text not null,
    outcome_json text not null check(json_valid(outcome_json)),
    primary key(invocation_id,ordinal),
    foreign key(invocation_id,ordinal,run_id)
        references agent_stop_hook_handlers(invocation_id,ordinal,run_id) on delete cascade
);
create table agent_stop_hook_finishes (
    invocation_id text primary key references agent_stop_hook_admissions(invocation_id) on delete cascade,
    canceled integer not null check(canceled in (0,1)),
    finished_at_ms integer not null check(finished_at_ms>=0)
);
create trigger agent_stop_hook_finish_complete
before insert on agent_stop_hook_finishes
begin
    select case when (select count(*) from agent_stop_hook_handlers where invocation_id=new.invocation_id)
        != (select count(*) from agent_stop_hook_results where invocation_id=new.invocation_id)
        then raise(abort,'stop hook finish lacks handler results') end;
end;
create trigger agent_stop_hook_result_before_finish
before insert on agent_stop_hook_results
when exists(select 1 from agent_stop_hook_finishes where invocation_id=new.invocation_id)
begin select raise(abort,'stop hook already finished'); end;
create trigger agent_stop_hook_handler_before_finish
before insert on agent_stop_hook_handlers
when exists(select 1 from agent_stop_hook_finishes where invocation_id=new.invocation_id)
begin select raise(abort,'stop hook already finished'); end;
create trigger agent_stop_hook_admissions_immutable before update on agent_stop_hook_admissions
begin select raise(abort,'stop hook admission is immutable'); end;
create trigger agent_stop_hook_handlers_immutable before update on agent_stop_hook_handlers
begin select raise(abort,'stop hook handler is immutable'); end;
create trigger agent_stop_hook_results_immutable before update on agent_stop_hook_results
begin select raise(abort,'stop hook result is immutable'); end;
create trigger agent_stop_hook_finishes_immutable before update on agent_stop_hook_finishes
begin select raise(abort,'stop hook finish is immutable'); end;
create trigger agent_stop_hook_admissions_no_delete before delete on agent_stop_hook_admissions
when exists(select 1 from agent_conversations where id=old.conversation_id)
begin select raise(abort,'stop hook admission cannot be deleted'); end;
create trigger agent_stop_hook_handlers_no_delete before delete on agent_stop_hook_handlers
when exists(select 1 from agent_stop_hook_admissions where invocation_id=old.invocation_id)
begin select raise(abort,'stop hook handler cannot be deleted'); end;
create trigger agent_stop_hook_results_no_delete before delete on agent_stop_hook_results
when exists(select 1 from agent_stop_hook_admissions where invocation_id=old.invocation_id)
begin select raise(abort,'stop hook result cannot be deleted'); end;
create trigger agent_stop_hook_finishes_no_delete before delete on agent_stop_hook_finishes
when exists(select 1 from agent_stop_hook_admissions where invocation_id=old.invocation_id)
begin select raise(abort,'stop hook finish cannot be deleted'); end;
