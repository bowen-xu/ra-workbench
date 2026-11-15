create table agent_paper_reading_history_migrations (
    paper_id text not null references bibliographic_items(id) on delete cascade,
    legacy_session_id text not null,
    conversation_id text not null unique references agent_conversations(id) on delete cascade,
    expected_event_count integer not null check(expected_event_count >= 0),
    status text not null check(status in ('pending', 'complete')),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at text,
    primary key(paper_id, legacy_session_id)
);
create index agent_paper_reading_history_migrations_paper_idx
    on agent_paper_reading_history_migrations(paper_id);
