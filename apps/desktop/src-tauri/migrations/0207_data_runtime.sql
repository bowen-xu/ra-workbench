-- The SQLite prefix and Raft suffix form one database, identified by epoch.
create table data_runtime_state (
    singleton integer primary key check(singleton = 1),
    schema_version integer not null check(schema_version = 1),
    epoch text not null,
    materialized_position integer not null check(materialized_position >= 0),
    bootstrap_state text not null check(bootstrap_state in ('preparing', 'ready'))
);
create table data_operation_receipts (
    operation_id text primary key,
    epoch text not null,
    position integer not null unique check(position > 0),
    fingerprint blob not null check(length(fingerprint) = 32),
    result_json text not null check(json_valid(result_json))
);
create table data_projection_control (
    singleton integer primary key check(singleton = 1),
    coalesce_text integer not null check(coalesce_text in (0, 1))
);
insert into data_projection_control values (1, 0);

-- The owner folds adjacent deltas before updating the growing message body.
-- Compatibility SQL writers retain the deployed per-event behavior.
drop trigger agent_search_message_delta;
create trigger agent_search_message_delta after insert on agent_journal_records
when json_extract(new.payload_json, '$.payload.type') = 'textDelta'
 and (select coalesce_text from data_projection_control where singleton = 1) = 0
begin
    update agent_messages set
      body = body || (select body from agent_search_events where conversation_id = new.conversation_id and ordinal = new.ordinal),
      last_ordinal = new.ordinal
    where conversation_id = new.conversation_id
      and item_id = json_extract(new.payload_json, '$.payload.itemId')
      and completed = 0 and last_ordinal < new.ordinal
      and exists(select 1 from agent_search_events e where e.conversation_id = new.conversation_id
        and e.ordinal = new.ordinal and e.item_id = agent_messages.item_id
        and e.turn_id = agent_messages.turn_id and e.body is not null);
end;
