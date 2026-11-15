create table agent_input_queues (
    conversation_id text primary key references agent_conversations(id) on delete cascade,
    revision integer not null check (revision >= 0),
    snapshot_json text not null check (json_valid(snapshot_json))
);
create table agent_input_queue_receipts (
    request_id text primary key,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    command_json text not null check (json_valid(command_json)),
    result_json text not null check (json_valid(result_json))
);
create table agent_input_queue_identities (
    input_id text primary key,
    conversation_id text not null references agent_conversations(id) on delete cascade
);
