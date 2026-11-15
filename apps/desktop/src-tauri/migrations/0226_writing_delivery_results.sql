-- Result commitment is distinct from file application and from turn completion.
create table writing_agent_delivery_results (
    delivery_id text primary key references writing_agent_delivery_tool_calls(delivery_id) on delete cascade,
    result_item_id text not null unique check(length(result_item_id)=36)
);
create trigger writing_agent_delivery_results_immutable
before update on writing_agent_delivery_results
begin select raise(abort, 'delivery result is immutable'); end;
create trigger writing_agent_delivery_results_no_delete
before delete on writing_agent_delivery_results
when exists(select 1 from writing_agent_delivery_tool_calls where delivery_id=old.delivery_id)
begin select raise(abort, 'delivery result cannot be deleted'); end;
