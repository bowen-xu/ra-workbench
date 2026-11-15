-- Compute every slice from one immutable scan before updating journal rows.
-- This replaces the per-row prefix sum without changing deployed migration 175.
drop trigger agent_message_text_complete;
create trigger agent_message_text_complete after insert on agent_journal_records
when json_extract(new.payload_json,'$.payload.type')='itemCompleted'
    and exists(select 1 from agent_messages m where m.conversation_id=new.conversation_id
        and m.item_id=json_extract(new.payload_json,'$.payload.itemId')
        and m.turn_id=json_extract(new.payload_json,'$.payload.turnId'))
begin
    update agent_journal_records set payload_json=ranges.compacted_json
    from (
        with spans as materialized (
            select ordinal,payload_json,coalesce(sum(coalesce(
                json_extract(payload_json,'$.payload.byteLength'),
                length(cast(json_extract(payload_json,'$.payload.text') as blob))))
                over (order by ordinal rows between unbounded preceding and 1 preceding),0) as byte_offset
            from agent_journal_records
            where conversation_id=new.conversation_id and ordinal<new.ordinal
                and json_extract(payload_json,'$.payload.itemId')=json_extract(new.payload_json,'$.payload.itemId')
                and json_extract(payload_json,'$.payload.turnId')=json_extract(new.payload_json,'$.payload.turnId')
                and json_extract(payload_json,'$.payload.type') in ('textDelta','storedTextSlice')
        )
        select ordinal,json_set(json_remove(payload_json,'$.payload.text'),
            '$.payload.type','storedTextSlice',
            '$.payload.byteLength',length(cast(json_extract(payload_json,'$.payload.text') as blob)),
            '$.payload.byteOffset',byte_offset) as compacted_json
        from spans where json_extract(payload_json,'$.payload.type')='textDelta'
            and json_type(payload_json,'$.payload.text')='text'
    ) as ranges
    where agent_journal_records.conversation_id=new.conversation_id
        and agent_journal_records.ordinal=ranges.ordinal;
end;
