-- Pending compact SessionStart belongs to a newly installed window, not journal replay.
create table agent_compact_session_start (
    conversation_id text primary key not null references agent_conversations(id) on delete cascade,
    window_id text not null,
    foreign key(conversation_id, window_id)
        references agent_context_checkpoints(conversation_id, id) on delete cascade
);
