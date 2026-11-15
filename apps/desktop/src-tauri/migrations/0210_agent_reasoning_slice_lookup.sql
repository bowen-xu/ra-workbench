-- Reuse the complete part projection populated by the message trigger before
-- the accepting journal trigger resumes. Lookup is indexed by exact part identity.
create unique index agent_reasoning_parts_identity
    on agent_reasoning_parts(conversation_id,item_id,summary_index);
drop trigger agent_message_canonical_accept;
create trigger agent_message_canonical_accept after insert on agent_journal_records
when json_extract(new.payload_json,'$.payload.type')='itemAccepted'
    and json_extract(new.payload_json,'$.payload.kind.type') in ('assistantMessage','reasoningSummary')
begin
    insert into agent_messages(conversation_id,item_id,turn_id,role,
        start_ordinal,last_ordinal,completed,body)
    values(new.conversation_id,json_extract(new.payload_json,'$.payload.itemId'),
        json_extract(new.payload_json,'$.payload.turnId'),json_extract(new.payload_json,'$.payload.kind.type'),
        new.ordinal,new.ordinal,1,json_extract(new.payload_json,'$.payload.text'))
    on conflict(conversation_id,item_id) do update set
        last_ordinal=excluded.last_ordinal,completed=1,body=excluded.body
    where agent_messages.completed=0 and agent_messages.turn_id=excluded.turn_id;

    -- Assistant text retains 206's immutable prefix scan and byte proof.
    update agent_journal_records set payload_json=ranges.compacted_json
    from (
        with spans as materialized (
            select ordinal,payload_json,coalesce(sum(coalesce(
                json_extract(payload_json,'$.payload.byteLength'),
                length(cast(json_extract(payload_json,'$.payload.text') as blob))))
                over (order by ordinal rows between unbounded preceding and 1 preceding),0) as byte_offset
            from agent_journal_records
            where conversation_id=new.conversation_id and ordinal<new.ordinal
                and json_extract(new.payload_json,'$.payload.kind.type')='assistantMessage'
                and json_extract(payload_json,'$.payload.itemId')=json_extract(new.payload_json,'$.payload.itemId')
                and json_extract(payload_json,'$.payload.turnId')=json_extract(new.payload_json,'$.payload.turnId')
                and json_extract(payload_json,'$.payload.type') in ('textDelta','storedTextSlice')
        )
        select s.ordinal,json_set(json_remove(s.payload_json,'$.payload.text'),
            '$.payload.type','storedTextSlice',
            '$.payload.byteLength',length(cast(json_extract(s.payload_json,'$.payload.text') as blob)),
            '$.payload.byteOffset',s.byte_offset) as compacted_json
        from spans s join agent_messages m on m.conversation_id=new.conversation_id
            and m.item_id=json_extract(new.payload_json,'$.payload.itemId')
            and m.turn_id=json_extract(new.payload_json,'$.payload.turnId') and m.completed=1
        where json_extract(s.payload_json,'$.payload.type')='textDelta'
            and json_type(s.payload_json,'$.payload.text')='text'
            and cast(substr(cast(m.body as blob),s.byte_offset+1,
                length(cast(json_extract(s.payload_json,'$.payload.text') as blob))) as blob)
                =cast(json_extract(s.payload_json,'$.payload.text') as blob)
    ) as ranges
    where agent_journal_records.conversation_id=new.conversation_id
        and agent_journal_records.ordinal=ranges.ordinal;

    -- Summary parts can arrive interleaved and their final text may replace the
    -- draft. Offset each part independently and compact only exactly proven bytes.
    -- Freeze three small identity fields once; repeated JSON extraction would
    -- otherwise reparse the large canonical item for every delta.
    update agent_journal_records set payload_json=ranges.compacted_json
    from (
        with accepted_identity as materialized (
            select json_extract(new.payload_json,'$.payload.itemId') as item_id,
                json_extract(new.payload_json,'$.payload.turnId') as turn_id,
                json_extract(new.payload_json,'$.payload.kind.type') as kind
        ), spans as materialized (
            select ordinal,payload_json,coalesce(sum(coalesce(
                json_extract(payload_json,'$.payload.byteLength'),
                length(cast(json_extract(payload_json,'$.payload.text') as blob))))
                over (partition by json_extract(payload_json,'$.payload.summaryIndex')
                    order by ordinal rows between unbounded preceding and 1 preceding),0) as part_offset
            from agent_journal_records
            where conversation_id=new.conversation_id and ordinal<new.ordinal
                and (select kind from accepted_identity)='reasoningSummary'
                and json_extract(payload_json,'$.payload.itemId')=(select item_id from accepted_identity)
                and json_extract(payload_json,'$.payload.turnId')=(select turn_id from accepted_identity)
                and json_extract(payload_json,'$.payload.type') in ('reasoningSummaryDelta','storedReasoningSlice')
        )
        select s.ordinal,json_set(json_remove(s.payload_json,'$.payload.text'),
            '$.payload.type','storedReasoningSlice',
            '$.payload.byteLength',length(cast(json_extract(s.payload_json,'$.payload.text') as blob)),
            '$.payload.byteOffset',p.start_byte+s.part_offset) as compacted_json
        from spans s join agent_messages m on m.conversation_id=new.conversation_id
            and m.item_id=(select item_id from accepted_identity)
            and m.turn_id=(select turn_id from accepted_identity) and m.completed=1
        join agent_reasoning_parts p
            on p.conversation_id=new.conversation_id
            and p.item_id=(select item_id from accepted_identity)
            and p.summary_index=json_extract(s.payload_json,'$.payload.summaryIndex')
        where json_extract(s.payload_json,'$.payload.type')='reasoningSummaryDelta'
            and json_type(s.payload_json,'$.payload.text')='text'
            and s.part_offset+length(cast(json_extract(s.payload_json,'$.payload.text') as blob))
                <=p.len_byte
            and cast(substr(cast(m.body as blob),p.start_byte+s.part_offset+1,
                length(cast(json_extract(s.payload_json,'$.payload.text') as blob))) as blob)
                =cast(json_extract(s.payload_json,'$.payload.text') as blob)
    ) as ranges
    where agent_journal_records.conversation_id=new.conversation_id
        and agent_journal_records.ordinal=ranges.ordinal;

    update agent_journal_records set payload_json=json_set(
        json_remove(payload_json,'$.payload.text'),'$.payload.type','storedItemAccepted')
    where conversation_id=new.conversation_id and ordinal=new.ordinal
        and json_type(new.payload_json,'$.payload.text')='text'
        and exists(select 1 from agent_messages m where m.conversation_id=new.conversation_id
            and m.item_id=json_extract(new.payload_json,'$.payload.itemId')
            and m.turn_id=json_extract(new.payload_json,'$.payload.turnId') and m.completed=1
            and cast(m.body as blob)=cast(json_extract(new.payload_json,'$.payload.text') as blob));
end;
