-- Rebuildable projection of durable user/assistant text only. The journal remains authoritative.
create table agent_search_messages (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    item_id text not null,
    turn_id text not null,
    role text not null check (role in ('userMessage', 'assistantMessage')),
    start_ordinal integer not null check (start_ordinal >= 0),
    last_ordinal integer not null check (last_ordinal >= start_ordinal),
    completed integer not null default 0 check (completed in (0, 1)),
    body text not null default '',
    primary key (conversation_id, item_id)
);
create index agent_search_messages_order on agent_search_messages(conversation_id, start_ordinal);

-- Guard the normalized envelope before extracting message payloads. JSON is validated by agent_items.
create view agent_search_events as
select conversation_id, ordinal,
       json_extract(payload_json, '$.payload.type') as event_type,
       json_extract(payload_json, '$.payload.itemId') as item_id,
       json_extract(payload_json, '$.payload.turnId') as turn_id,
       json_extract(payload_json, '$.payload.kind.type') as role,
       case when json_type(payload_json, '$.payload.text') = 'text'
            then json_extract(payload_json, '$.payload.text') end as body
from agent_items
where schema_version = 1
  and json_extract(payload_json, '$.schemaVersion') = 1
  and json_extract(payload_json, '$.conversationId') = conversation_id
  and json_extract(payload_json, '$.seq') = ordinal + 1
  and json_type(payload_json, '$.payload.itemId') = 'text'
  and json_type(payload_json, '$.payload.turnId') = 'text';

-- Reconstruct old split deltas in journal order, including committed tails of active messages.
-- Materialize once: repeatedly expanding the JSON view would scan each conversation tail per item.
create temp table agent_search_backfill_events as select * from agent_search_events;
create index agent_search_backfill_order
    on agent_search_backfill_events(conversation_id, item_id, turn_id, ordinal, event_type);
with starts as (
    select conversation_id, item_id, turn_id, role, min(ordinal) as start_ordinal
    from agent_search_backfill_events
    where event_type = 'itemStarted' and role in ('userMessage', 'assistantMessage')
    group by conversation_id, item_id
), messages as (
    select s.*, min(e.ordinal) as completed_ordinal
    from starts s left join agent_search_backfill_events e
      on e.conversation_id = s.conversation_id and e.item_id = s.item_id
      and e.turn_id = s.turn_id and e.ordinal > s.start_ordinal and e.event_type = 'itemCompleted'
    group by s.conversation_id, s.item_id
)
insert into agent_search_messages(conversation_id, item_id, turn_id, role, start_ordinal, last_ordinal, completed, body)
select m.conversation_id, m.item_id, m.turn_id, m.role, m.start_ordinal,
       coalesce(m.completed_ordinal, max(d.ordinal), m.start_ordinal),
       m.completed_ordinal is not null, coalesce(group_concat(d.body, '' order by d.ordinal), '')
from messages m left join agent_search_backfill_events d
  on d.conversation_id = m.conversation_id and d.item_id = m.item_id and d.turn_id = m.turn_id
  and d.ordinal > m.start_ordinal and (m.completed_ordinal is null or d.ordinal < m.completed_ordinal)
  and d.event_type = 'textDelta' and d.body is not null
group by m.conversation_id, m.item_id;
drop table agent_search_backfill_events;

-- These writes are part of the same transaction as their durable events and watermark.
create trigger agent_search_message_start after insert on agent_items
begin
    insert into agent_search_messages(conversation_id, item_id, turn_id, role, start_ordinal, last_ordinal)
    select conversation_id, item_id, turn_id, role, ordinal, ordinal from agent_search_events
    where conversation_id = new.conversation_id and ordinal = new.ordinal
      and event_type = 'itemStarted' and role in ('userMessage', 'assistantMessage');
end;
create trigger agent_search_message_delta after insert on agent_items
when json_extract(new.payload_json, '$.payload.type') = 'textDelta'
begin
    update agent_search_messages set
      body = body || (select body from agent_search_events where conversation_id = new.conversation_id and ordinal = new.ordinal),
      last_ordinal = new.ordinal
    where conversation_id = new.conversation_id
      and item_id = json_extract(new.payload_json, '$.payload.itemId')
      and completed = 0 and last_ordinal < new.ordinal
      and exists(select 1 from agent_search_events e where e.conversation_id = new.conversation_id
        and e.ordinal = new.ordinal and e.item_id = agent_search_messages.item_id
        and e.turn_id = agent_search_messages.turn_id and e.body is not null);
end;
create trigger agent_search_message_complete after insert on agent_items
when json_extract(new.payload_json, '$.payload.type') = 'itemCompleted'
begin
    update agent_search_messages set completed = 1, last_ordinal = new.ordinal
    where conversation_id = new.conversation_id
      and item_id = json_extract(new.payload_json, '$.payload.itemId')
      and completed = 0 and last_ordinal < new.ordinal
      and exists(select 1 from agent_search_events e where e.conversation_id = new.conversation_id
        and e.ordinal = new.ordinal and e.item_id = agent_search_messages.item_id
        and e.turn_id = agent_search_messages.turn_id);
end;
