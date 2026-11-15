create table agent_submissions (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    request_id text not null,
    turn_id text not null unique,
    context_snapshot_id text not null unique,
    schema_version integer not null check (schema_version = 1),
    input_json text not null,
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    primary key (conversation_id, request_id)
);
