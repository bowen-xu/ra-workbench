-- Legacy proofs remain readable without a fabricated tool-call association.
create table writing_agent_delivery_tool_calls (
    delivery_id text primary key references writing_agent_delivery_proofs(delivery_id) on delete cascade,
    item_id text not null check(length(item_id)=36),
    call_id text not null check(length(call_id)=36),
    unique(call_id)
);
create trigger writing_agent_delivery_tool_calls_immutable
before update on writing_agent_delivery_tool_calls
begin select raise(abort, 'delivery tool call is immutable'); end;
create trigger writing_agent_delivery_tool_calls_no_delete
before delete on writing_agent_delivery_tool_calls
when exists(select 1 from writing_agent_delivery_proofs where delivery_id=old.delivery_id)
begin select raise(abort, 'delivery tool call cannot be deleted'); end;
