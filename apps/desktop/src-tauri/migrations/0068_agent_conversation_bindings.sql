-- Host associations select conversations; they do not authorize context access.
-- Resource IDs remain snapshots if a host resource is removed. Deleting a binding
-- never removes the host resource or the conversation's durable history.
create table agent_conversation_bindings (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    resource_kind text not null check (resource_kind in ('manuscript', 'paper', 'note')),
    resource_id text not null check (length(resource_id) between 1 and 256),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    primary key (conversation_id, resource_kind, resource_id)
);
create index agent_conversation_bindings_resource_idx
    on agent_conversation_bindings(resource_kind, resource_id, conversation_id);
