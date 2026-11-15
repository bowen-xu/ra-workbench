-- Compatibility mapping only: execution is being moved to shared Agent turns.
create table agent_legacy_ai_run_bindings (
    ai_run_id text primary key references ai_runs(id) on delete cascade,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    request_id text not null,
    task_kind text not null,
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    unique (conversation_id, request_id)
);

create index agent_legacy_ai_run_bindings_conversation_idx
    on agent_legacy_ai_run_bindings(conversation_id, created_at desc);
