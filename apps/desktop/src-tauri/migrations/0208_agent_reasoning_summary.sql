-- Expand the canonical text store without changing deployed journal positions,
-- receipts or old migration bytes. The runner owns FK handling and the transaction.
drop trigger agent_search_message_start;
drop trigger agent_search_message_delta;
drop trigger agent_search_message_complete;
drop trigger agent_message_text_complete;
drop trigger agent_message_canonical_accept;
drop view agent_search_events;
drop view agent_items;
drop view agent_search_messages;

create table agent_messages_208 (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    item_id text not null,
    turn_id text not null,
    role text not null check(role in ('userMessage', 'assistantMessage', 'reasoningSummary')),
    start_ordinal integer not null check(start_ordinal >= 0),
    last_ordinal integer not null check(last_ordinal >= start_ordinal),
    completed integer not null default 0 check(completed in (0, 1)),
    body text not null default '',
    primary key(conversation_id, item_id)
);
insert into agent_messages_208(rowid,conversation_id,item_id,turn_id,role,
    start_ordinal,last_ordinal,completed,body)
select rowid,conversation_id,item_id,turn_id,role,start_ordinal,last_ordinal,completed,body
from agent_messages;
drop table agent_messages;
alter table agent_messages_208 rename to agent_messages;
create index agent_search_messages_order on agent_messages(conversation_id,start_ordinal);

create view agent_search_messages as
select conversation_id,item_id,turn_id,role,start_ordinal,last_ordinal,completed,body
from agent_messages;
create view agent_items as
select r.conversation_id,r.ordinal,r.schema_version,
    case json_extract(r.payload_json,'$.payload.type')
        when 'storedTextSlice' then json_remove(json_set(r.payload_json,
            '$.payload.type','textDelta',
            '$.payload.text',cast(substr(cast(m.body as blob),
                json_extract(r.payload_json,'$.payload.byteOffset')+1,
                json_extract(r.payload_json,'$.payload.byteLength')) as text)),
            '$.payload.byteOffset','$.payload.byteLength')
        when 'storedReasoningSlice' then json_remove(json_set(r.payload_json,
            '$.payload.type','reasoningSummaryDelta',
            '$.payload.text',cast(substr(cast(m.body as blob),
                json_extract(r.payload_json,'$.payload.byteOffset')+1,
                json_extract(r.payload_json,'$.payload.byteLength')) as text)),
            '$.payload.byteOffset','$.payload.byteLength')
        when 'storedItemAccepted' then json_set(r.payload_json,
            '$.payload.type','itemAccepted',
            '$.payload.text',case when m.completed=1
                and m.turn_id=json_extract(r.payload_json,'$.payload.turnId')
                then m.body end)
        else r.payload_json end as payload_json
from agent_journal_records r
left join agent_messages m on m.conversation_id=r.conversation_id
    and m.item_id=json_extract(r.payload_json,'$.payload.itemId');

create trigger agent_items_insert instead of insert on agent_items begin
    insert into agent_journal_records(conversation_id,ordinal,schema_version,payload_json)
    values(new.conversation_id,new.ordinal,new.schema_version,new.payload_json);
end;
create trigger agent_items_update instead of update on agent_items begin
    update agent_journal_records set conversation_id=new.conversation_id,
        ordinal=new.ordinal,schema_version=new.schema_version,payload_json=new.payload_json
    where conversation_id=old.conversation_id and ordinal=old.ordinal;
end;
create trigger agent_items_delete instead of delete on agent_items begin
    delete from agent_journal_records
    where conversation_id=old.conversation_id and ordinal=old.ordinal;
end;

create view agent_search_events as
select conversation_id,ordinal,
    json_extract(payload_json,'$.payload.type') as event_type,
    json_extract(payload_json,'$.payload.itemId') as item_id,
    json_extract(payload_json,'$.payload.turnId') as turn_id,
    json_extract(payload_json,'$.payload.kind.type') as role,
    case when json_type(payload_json,'$.payload.text')='text'
        then json_extract(payload_json,'$.payload.text') end as body
from agent_items
where schema_version=1 and json_extract(payload_json,'$.schemaVersion')=1
    and json_extract(payload_json,'$.conversationId')=conversation_id
    and json_extract(payload_json,'$.seq')=ordinal+1
    and json_type(payload_json,'$.payload.itemId')='text'
    and json_type(payload_json,'$.payload.turnId')='text';

create trigger agent_search_message_start after insert on agent_journal_records
begin
    insert into agent_messages(conversation_id,item_id,turn_id,role,start_ordinal,last_ordinal)
    select conversation_id,item_id,turn_id,role,ordinal,ordinal from agent_search_events
    where conversation_id=new.conversation_id and ordinal=new.ordinal
        and event_type='itemStarted'
        and role in ('userMessage','assistantMessage','reasoningSummary');
end;
create trigger agent_search_message_delta after insert on agent_journal_records
when json_extract(new.payload_json,'$.payload.type') in ('textDelta','reasoningSummaryDelta')
    and (select coalesce_text from data_projection_control where singleton=1)=0
begin
    update agent_messages set
        body=body || (select body from agent_search_events
            where conversation_id=new.conversation_id and ordinal=new.ordinal),
        last_ordinal=new.ordinal
    where conversation_id=new.conversation_id
        and item_id=json_extract(new.payload_json,'$.payload.itemId')
        and completed=0 and last_ordinal<new.ordinal
        and exists(select 1 from agent_search_events e where e.conversation_id=new.conversation_id
            and e.ordinal=new.ordinal and e.item_id=agent_messages.item_id
            and e.turn_id=agent_messages.turn_id and e.body is not null);
end;
create trigger agent_search_message_complete after insert on agent_journal_records
when json_extract(new.payload_json,'$.payload.type')='itemCompleted'
begin
    update agent_messages set completed=1,last_ordinal=new.ordinal
    where conversation_id=new.conversation_id
        and item_id=json_extract(new.payload_json,'$.payload.itemId')
        and completed=0 and last_ordinal<new.ordinal
        and exists(select 1 from agent_search_events e where e.conversation_id=new.conversation_id
            and e.ordinal=new.ordinal and e.item_id=agent_messages.item_id
            and e.turn_id=agent_messages.turn_id);
end;

-- Preserve the deployed legacy text-completion path. Summary ranges are only
-- proven against a canonical acceptance below, never guessed from a draft.
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
    update agent_journal_records set payload_json=ranges.compacted_json
    from (
        with spans as materialized (
            select ordinal,payload_json,coalesce(sum(coalesce(
                json_extract(payload_json,'$.payload.byteLength'),
                length(cast(json_extract(payload_json,'$.payload.text') as blob))))
                over (partition by json_extract(payload_json,'$.payload.summaryIndex')
                    order by ordinal rows between unbounded preceding and 1 preceding),0) as part_offset
            from agent_journal_records
            where conversation_id=new.conversation_id and ordinal<new.ordinal
                and json_extract(new.payload_json,'$.payload.kind.type')='reasoningSummary'
                and json_extract(payload_json,'$.payload.itemId')=json_extract(new.payload_json,'$.payload.itemId')
                and json_extract(payload_json,'$.payload.turnId')=json_extract(new.payload_json,'$.payload.turnId')
                and json_extract(payload_json,'$.payload.type') in ('reasoningSummaryDelta','storedReasoningSlice')
        )
        select s.ordinal,json_set(json_remove(s.payload_json,'$.payload.text'),
            '$.payload.type','storedReasoningSlice',
            '$.payload.byteLength',length(cast(json_extract(s.payload_json,'$.payload.text') as blob)),
            '$.payload.byteOffset',json_extract(p.value,'$.startByte')+s.part_offset) as compacted_json
        from spans s join agent_messages m on m.conversation_id=new.conversation_id
            and m.item_id=json_extract(new.payload_json,'$.payload.itemId')
            and m.turn_id=json_extract(new.payload_json,'$.payload.turnId') and m.completed=1
        join json_each(new.payload_json,'$.payload.kind.parts') p
            on json_extract(p.value,'$.index')=json_extract(s.payload_json,'$.payload.summaryIndex')
        where json_extract(s.payload_json,'$.payload.type')='reasoningSummaryDelta'
            and json_type(s.payload_json,'$.payload.text')='text'
            and s.part_offset+length(cast(json_extract(s.payload_json,'$.payload.text') as blob))
                <=json_extract(p.value,'$.lenByte')
            and cast(substr(cast(m.body as blob),json_extract(p.value,'$.startByte')+s.part_offset+1,
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
