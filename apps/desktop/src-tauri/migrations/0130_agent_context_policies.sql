create table agent_context_policies (
    turn_id text primary key references agent_submissions(turn_id) on delete cascade,
    policy_json text not null check (json_valid(policy_json) and length(policy_json) <= 8192)
);

create trigger agent_context_policies_no_update
before update on agent_context_policies
begin
    select raise(abort, 'frozen context policy cannot change');
end;
