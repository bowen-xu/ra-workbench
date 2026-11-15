-- A completed native item is one journal acceptance, not split metadata/body/order.
-- Keep the deployed delta and storedTextSlice readers unchanged for old history.
create trigger agent_message_canonical_accept after insert on agent_journal_records
when json_extract(new.payload_json,'$.payload.type')='itemAccepted'
    and json_extract(new.payload_json,'$.payload.kind.type')='assistantMessage'
begin
    insert into agent_messages(conversation_id,item_id,turn_id,role,
        start_ordinal,last_ordinal,completed,body)
    values(new.conversation_id,json_extract(new.payload_json,'$.payload.itemId'),
        json_extract(new.payload_json,'$.payload.turnId'),'assistantMessage',
        new.ordinal,new.ordinal,1,json_extract(new.payload_json,'$.payload.text'))
    on conflict(conversation_id,item_id) do update set
        last_ordinal=excluded.last_ordinal,completed=1,body=excluded.body
    where agent_messages.completed=0 and agent_messages.turn_id=excluded.turn_id;
end;
