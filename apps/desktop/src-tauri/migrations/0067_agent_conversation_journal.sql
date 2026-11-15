create table agent_conversations (
    id text primary key not null,
    title text not null check (length(trim(title)) between 1 and 180),
    archived integer not null default 0 check (archived in (0, 1)),
    durable_count integer not null default 0 check (durable_count >= 0),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

create table agent_items (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    ordinal integer not null check (ordinal >= 0),
    schema_version integer not null check (schema_version = 1),
    payload_json text not null check (json_valid(payload_json)),
    primary key (conversation_id, ordinal)
);

-- Batch identity survives reconnection and an ambiguous commit acknowledgement.
create table agent_journal_batches (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    batch_id text not null,
    expected_count integer not null check (expected_count >= 0),
    result_count integer not null check (result_count > expected_count),
    payload_hash text not null,
    primary key (conversation_id, batch_id)
);
