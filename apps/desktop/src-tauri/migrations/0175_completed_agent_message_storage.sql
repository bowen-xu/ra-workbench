-- Promote the existing complete-message projection to the canonical text store.
-- Journal positions remain physical rows because review/context receipts reference
-- them. Completed rows keep only a byte range; their old delta text is removed.
alter table agent_search_messages rename to agent_messages;
create view agent_search_messages as
select conversation_id,item_id,turn_id,role,start_ordinal,last_ordinal,completed,body
from agent_messages;

alter table agent_items rename to agent_journal_records;
create view agent_items as
select r.conversation_id,r.ordinal,r.schema_version,
    case when json_extract(r.payload_json,'$.payload.type')='storedTextSlice' then
        json_remove(json_set(r.payload_json,
            '$.payload.type','textDelta',
            '$.payload.text',cast(substr(cast(m.body as blob),
                json_extract(r.payload_json,'$.payload.byteOffset')+1,
                json_extract(r.payload_json,'$.payload.byteLength')) as text)),
            '$.payload.byteOffset','$.payload.byteLength')
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

-- ALTER TABLE redirects old views to the physical records. Search and its live
-- append triggers must continue to observe the logical event contract instead.
drop view agent_search_events;
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

create index agent_journal_message_order on agent_journal_records(
    conversation_id,json_extract(payload_json,'$.payload.itemId'),ordinal);

-- Keep byte lengths while walking an item: earlier rows may already be slices
-- when a later row is updated. This also preserves Unicode and embedded NULs.
create trigger agent_message_text_complete after insert on agent_journal_records
when json_extract(new.payload_json,'$.payload.type')='itemCompleted'
    and exists(select 1 from agent_messages m where m.conversation_id=new.conversation_id
        and m.item_id=json_extract(new.payload_json,'$.payload.itemId')
        and m.turn_id=json_extract(new.payload_json,'$.payload.turnId'))
begin
    update agent_journal_records set payload_json=json_set(
        json_remove(payload_json,'$.payload.text'),
        '$.payload.type','storedTextSlice',
        '$.payload.byteLength',length(cast(json_extract(payload_json,'$.payload.text') as blob)),
        '$.payload.byteOffset',coalesce((select sum(coalesce(
            json_extract(p.payload_json,'$.payload.byteLength'),
            length(cast(json_extract(p.payload_json,'$.payload.text') as blob))))
            from agent_journal_records p
            where p.conversation_id=agent_journal_records.conversation_id
                and json_extract(p.payload_json,'$.payload.itemId')=json_extract(agent_journal_records.payload_json,'$.payload.itemId')
                and p.ordinal<agent_journal_records.ordinal
                and json_extract(p.payload_json,'$.payload.type') in ('textDelta','storedTextSlice')),0))
    where conversation_id=new.conversation_id and ordinal<new.ordinal
        and json_extract(payload_json,'$.payload.itemId')=json_extract(new.payload_json,'$.payload.itemId')
        and json_extract(payload_json,'$.payload.turnId')=json_extract(new.payload_json,'$.payload.turnId')
        and json_extract(payload_json,'$.payload.type')='textDelta'
        and json_type(payload_json,'$.payload.text')='text';
end;

-- Backfill only complete, valid messages. Active/partial text remains recoverable
-- as raw deltas until its own completion. No journal numbering or receipt changes.
update agent_journal_records set payload_json=json_set(
    json_remove(payload_json,'$.payload.text'),
    '$.payload.type','storedTextSlice',
    '$.payload.byteLength',length(cast(json_extract(payload_json,'$.payload.text') as blob)),
    '$.payload.byteOffset',coalesce((select sum(coalesce(
        json_extract(p.payload_json,'$.payload.byteLength'),
        length(cast(json_extract(p.payload_json,'$.payload.text') as blob))))
        from agent_journal_records p
        where p.conversation_id=agent_journal_records.conversation_id
            and json_extract(p.payload_json,'$.payload.itemId')=json_extract(agent_journal_records.payload_json,'$.payload.itemId')
            and p.ordinal<agent_journal_records.ordinal
            and json_extract(p.payload_json,'$.payload.type') in ('textDelta','storedTextSlice')),0))
where json_extract(payload_json,'$.payload.type')='textDelta'
    and json_type(payload_json,'$.payload.text')='text'
    and exists(select 1 from agent_messages m
        where m.conversation_id=agent_journal_records.conversation_id and m.completed=1
            and m.item_id=json_extract(agent_journal_records.payload_json,'$.payload.itemId')
            and m.turn_id=json_extract(agent_journal_records.payload_json,'$.payload.turnId')
            and m.last_ordinal>agent_journal_records.ordinal);
