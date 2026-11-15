create table agent_paper_analysis_submissions (
    attempt_id text primary key references paper_analysis_turn_attempts(id) on delete cascade,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    request_id text not null,
    turn_id text not null,
    context_snapshot_id text not null,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    created_at text not null,
    unique(conversation_id, request_id),
    unique(conversation_id, turn_id)
);

create index agent_paper_analysis_submissions_conversation_idx
    on agent_paper_analysis_submissions(conversation_id, created_at desc);
