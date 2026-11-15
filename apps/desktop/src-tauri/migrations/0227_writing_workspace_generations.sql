-- Later working copies have independent, immutable identity and predecessor.
-- The original execution workspace remains the generation-zero authority.
create table writing_agent_workspace_generations (
    conversation_id text not null,
    request_id text not null,
    generation integer not null check(generation between 1 and 2147483647),
    turn_id text not null,
    context_snapshot_id text not null,
    manuscript_id text not null references manuscripts(id) on delete cascade,
    lease_id text not null unique,
    manifest_hash text not null check(length(manifest_hash)=64),
    manifest_json text not null check(json_valid(manifest_json)),
    predecessor_delivery_id text not null unique references writing_agent_delivery_results(delivery_id) on delete cascade,
    state text not null default 'preparing' check(state in ('preparing','prepared','approved','revoked')),
    primary key(conversation_id,request_id,generation),
    foreign key(conversation_id,request_id) references agent_submissions(conversation_id,request_id) on delete cascade
);
create unique index writing_agent_workspace_generation_active
on writing_agent_workspace_generations(conversation_id,request_id)
where state!='revoked';
create trigger writing_agent_workspace_generation_identity_immutable
before update of conversation_id,request_id,generation,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json,predecessor_delivery_id on writing_agent_workspace_generations
begin select raise(abort,'workspace generation identity is immutable'); end;
create trigger writing_agent_workspace_generation_state_guard
before update of state on writing_agent_workspace_generations
when not (new.state=old.state or new.state='revoked'
    or (old.state='preparing' and new.state='prepared')
    or (old.state='prepared' and new.state='approved'))
begin select raise(abort,'invalid workspace generation state'); end;
create trigger writing_agent_workspace_generation_no_delete
before delete on writing_agent_workspace_generations
when exists(select 1 from agent_submissions where conversation_id=old.conversation_id and request_id=old.request_id)
begin select raise(abort,'workspace generation cannot be deleted'); end;
create trigger writing_agent_workspace_generation_turn_finished
after insert on agent_journal_records
when json_extract(new.payload_json,'$.payload.type')='turnFinished'
begin
    update writing_agent_workspace_generations set state='revoked'
    where conversation_id=new.conversation_id and turn_id=json_extract(new.payload_json,'$.payload.turnId');
end;
