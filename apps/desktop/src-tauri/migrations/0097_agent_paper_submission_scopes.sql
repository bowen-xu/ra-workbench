create table agent_paper_submission_scopes (
    conversation_id text not null,
    request_id text not null,
    turn_id text not null unique,
    paper_id text not null references bibliographic_items(id) on delete cascade,
    paper_context_snapshot_id text not null references paper_context_snapshots(id),
    context_hash text not null check(length(context_hash) = 64),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    primary key(conversation_id, request_id),
    foreign key(conversation_id, request_id)
        references agent_submissions(conversation_id, request_id) on delete cascade
);
create index agent_paper_submission_scopes_paper_idx
    on agent_paper_submission_scopes(paper_id, conversation_id);
