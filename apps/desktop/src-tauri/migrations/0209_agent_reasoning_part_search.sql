-- Search only complete public summary parts. This projection stores byte ranges,
-- never a second copy of public text or provider-native data.
create table agent_reasoning_parts (
    conversation_id text not null,
    item_id text not null,
    part_ordinal integer not null check(part_ordinal >= 0),
    summary_index text not null,
    start_byte integer not null check(start_byte >= 0),
    len_byte integer not null check(len_byte >= 0),
    primary key(conversation_id,item_id,part_ordinal),
    foreign key(conversation_id,item_id)
        references agent_messages(conversation_id,item_id) on delete cascade
);

-- Message triggers see the accepted journal row during both sparse INSERT and
-- draft completion UPDATE, independent of AFTER-journal trigger execution order.
create trigger agent_reasoning_parts_insert after insert on agent_messages
when new.role='reasoningSummary' and new.completed=1
begin
    insert into agent_reasoning_parts(conversation_id,item_id,part_ordinal,
        summary_index,start_byte,len_byte)
    select new.conversation_id,new.item_id,cast(p.key as integer),
        json_extract(p.value,'$.index'),json_extract(p.value,'$.startByte'),json_extract(p.value,'$.lenByte')
    from agent_journal_records r,json_each(r.payload_json,'$.payload.kind.parts') p
    where r.conversation_id=new.conversation_id and r.ordinal=new.last_ordinal
        and json_extract(r.payload_json,'$.payload.type') in ('itemAccepted','storedItemAccepted')
        and json_extract(r.payload_json,'$.payload.kind.type')='reasoningSummary'
        and json_extract(r.payload_json,'$.payload.itemId')=new.item_id
        and json_extract(r.payload_json,'$.payload.turnId')=new.turn_id;
end;
create trigger agent_reasoning_parts_update after update of completed,body,last_ordinal,role on agent_messages
when new.role='reasoningSummary' or old.role='reasoningSummary'
begin
    delete from agent_reasoning_parts
    where conversation_id=old.conversation_id and item_id=old.item_id;
    insert into agent_reasoning_parts(conversation_id,item_id,part_ordinal,
        summary_index,start_byte,len_byte)
    select new.conversation_id,new.item_id,cast(p.key as integer),
        json_extract(p.value,'$.index'),json_extract(p.value,'$.startByte'),json_extract(p.value,'$.lenByte')
    from agent_journal_records r,json_each(r.payload_json,'$.payload.kind.parts') p
    where new.role='reasoningSummary' and new.completed=1
        and r.conversation_id=new.conversation_id and r.ordinal=new.last_ordinal
        and json_extract(r.payload_json,'$.payload.type') in ('itemAccepted','storedItemAccepted')
        and json_extract(r.payload_json,'$.payload.kind.type')='reasoningSummary'
        and json_extract(r.payload_json,'$.payload.itemId')=new.item_id
        and json_extract(r.payload_json,'$.payload.turnId')=new.turn_id;
end;

-- Existing 208 completions already retain kind.parts at their accepted position,
-- including storedItemAccepted records whose common text lives only in body.
insert into agent_reasoning_parts(conversation_id,item_id,part_ordinal,
    summary_index,start_byte,len_byte)
select m.conversation_id,m.item_id,cast(p.key as integer),
    json_extract(p.value,'$.index'),json_extract(p.value,'$.startByte'),json_extract(p.value,'$.lenByte')
from agent_messages m join agent_journal_records r
    on r.conversation_id=m.conversation_id and r.ordinal=m.last_ordinal,
    json_each(r.payload_json,'$.payload.kind.parts') p
where m.role='reasoningSummary' and m.completed=1
    and json_extract(r.payload_json,'$.payload.type') in ('itemAccepted','storedItemAccepted')
    and json_extract(r.payload_json,'$.payload.kind.type')='reasoningSummary'
    and json_extract(r.payload_json,'$.payload.itemId')=m.item_id
    and json_extract(r.payload_json,'$.payload.turnId')=m.turn_id;

drop view agent_search_messages;
create view agent_search_messages as
select conversation_id,item_id,turn_id,role,start_ordinal,last_ordinal,completed,body,
    item_id as search_key,-1 as part_ordinal
from agent_messages where role<>'reasoningSummary'
union all
select m.conversation_id,m.item_id,m.turn_id,m.role,m.start_ordinal,m.last_ordinal,m.completed,
    cast(substr(cast(m.body as blob),p.start_byte+1,p.len_byte) as text) as body,
    m.item_id || ':' || p.part_ordinal as search_key,p.part_ordinal
from agent_messages m join agent_reasoning_parts p
    on p.conversation_id=m.conversation_id and p.item_id=m.item_id
where m.role='reasoningSummary' and m.completed=1;
