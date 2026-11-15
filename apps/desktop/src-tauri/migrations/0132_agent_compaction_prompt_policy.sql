-- Preserve frozen policy bytes while admitting bounded custom compaction prompts.
drop trigger agent_context_policies_no_update;
alter table agent_context_policies rename to agent_context_policies_0131;
create table agent_context_policies (
    turn_id text primary key references agent_submissions(turn_id) on delete cascade,
    policy_json text not null check (json_valid(policy_json) and length(policy_json) <= 131072)
);
insert into agent_context_policies select turn_id,policy_json from agent_context_policies_0131;
drop table agent_context_policies_0131;
create trigger agent_context_policies_no_update
before update on agent_context_policies
begin
    select raise(abort, 'frozen context policy cannot change');
end;
