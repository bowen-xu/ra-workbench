-- Preserve existing pending hooks while allowing one ordered entry per window.
alter table agent_compact_session_start rename to agent_compact_session_start_0220;
create table agent_compact_session_start (
    ordinal integer primary key autoincrement,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    window_id text not null,
    unique(conversation_id, window_id),
    foreign key(conversation_id, window_id)
        references agent_context_checkpoints(conversation_id, id) on delete cascade
);
insert into agent_compact_session_start(conversation_id, window_id)
select conversation_id, window_id from agent_compact_session_start_0220;
drop table agent_compact_session_start_0220;
create index agent_compact_session_start_conversation_order
    on agent_compact_session_start(conversation_id, ordinal);
