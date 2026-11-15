-- Complete native items share the canonical message body with legacy text slices.
-- Keep every physical journal position, envelope and acceptance metadata intact.
drop view agent_items;
create view agent_items as
select r.conversation_id,r.ordinal,r.schema_version,
    case json_extract(r.payload_json,'$.payload.type')
        when 'storedTextSlice' then json_remove(json_set(r.payload_json,
            '$.payload.type','textDelta',
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

drop trigger agent_message_canonical_accept;
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

    -- As in migration 176, freeze all offsets before replacing any delta.
    -- Byte comparisons preserve anomalous old rows instead of reinterpreting them.
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

    update agent_journal_records set payload_json=json_set(
        json_remove(payload_json,'$.payload.text'),'$.payload.type','storedItemAccepted')
    where conversation_id=new.conversation_id and ordinal=new.ordinal
        and json_type(new.payload_json,'$.payload.text')='text'
        and exists(select 1 from agent_messages m where m.conversation_id=new.conversation_id
            and m.item_id=json_extract(new.payload_json,'$.payload.itemId')
            and m.turn_id=json_extract(new.payload_json,'$.payload.turnId') and m.completed=1
            and cast(m.body as blob)=cast(json_extract(new.payload_json,'$.payload.text') as blob));
end;

-- Upgrade already accepted 205 items with the same immutable, linear range scan.
update agent_journal_records set payload_json=ranges.compacted_json
from (
    with spans as materialized (
        select r.conversation_id,r.ordinal,r.payload_json,m.body,coalesce(sum(coalesce(
            json_extract(r.payload_json,'$.payload.byteLength'),
            length(cast(json_extract(r.payload_json,'$.payload.text') as blob))))
            over (partition by r.conversation_id,
                json_extract(r.payload_json,'$.payload.itemId'),json_extract(r.payload_json,'$.payload.turnId')
                order by r.ordinal rows between unbounded preceding and 1 preceding),0) as byte_offset
        from agent_journal_records r join agent_messages m on m.conversation_id=r.conversation_id
            and m.item_id=json_extract(r.payload_json,'$.payload.itemId')
            and m.turn_id=json_extract(r.payload_json,'$.payload.turnId')
        where m.completed=1 and r.ordinal<m.last_ordinal
            and json_extract(r.payload_json,'$.payload.type') in ('textDelta','storedTextSlice')
            and exists(select 1 from agent_journal_records accepted
                where accepted.conversation_id=m.conversation_id and accepted.ordinal=m.last_ordinal
                    and json_extract(accepted.payload_json,'$.payload.type')='itemAccepted'
                    and json_extract(accepted.payload_json,'$.payload.itemId')=m.item_id
                    and json_extract(accepted.payload_json,'$.payload.turnId')=m.turn_id
                    and json_extract(accepted.payload_json,'$.payload.kind.type')='assistantMessage'
                    and json_type(accepted.payload_json,'$.payload.text')='text'
                    and cast(json_extract(accepted.payload_json,'$.payload.text') as blob)=cast(m.body as blob))
    )
    select conversation_id,ordinal,json_set(json_remove(payload_json,'$.payload.text'),
        '$.payload.type','storedTextSlice',
        '$.payload.byteLength',length(cast(json_extract(payload_json,'$.payload.text') as blob)),
        '$.payload.byteOffset',byte_offset) as compacted_json
    from spans where json_extract(payload_json,'$.payload.type')='textDelta'
        and json_type(payload_json,'$.payload.text')='text'
        and cast(substr(cast(body as blob),byte_offset+1,
            length(cast(json_extract(payload_json,'$.payload.text') as blob))) as blob)
            =cast(json_extract(payload_json,'$.payload.text') as blob)
) as ranges
where agent_journal_records.conversation_id=ranges.conversation_id
    and agent_journal_records.ordinal=ranges.ordinal;

update agent_journal_records set payload_json=json_set(
    json_remove(payload_json,'$.payload.text'),'$.payload.type','storedItemAccepted')
where json_extract(payload_json,'$.payload.type')='itemAccepted'
    and json_extract(payload_json,'$.payload.kind.type')='assistantMessage'
    and json_type(payload_json,'$.payload.text')='text'
    and exists(select 1 from agent_messages m where m.conversation_id=agent_journal_records.conversation_id
        and m.item_id=json_extract(agent_journal_records.payload_json,'$.payload.itemId')
        and m.turn_id=json_extract(agent_journal_records.payload_json,'$.payload.turnId') and m.completed=1
        and cast(m.body as blob)=cast(json_extract(agent_journal_records.payload_json,'$.payload.text') as blob));
