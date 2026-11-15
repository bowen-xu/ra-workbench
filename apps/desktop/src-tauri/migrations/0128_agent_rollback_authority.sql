create table agent_rollback_authority (
    request_id text primary key,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    resource_kind text not null,
    resource_id text not null
);
