create table agent_paper_analysis_bindings (
    workflow_id text primary key references paper_analysis_workflows(id) on delete cascade,
    conversation_id text not null unique references agent_conversations(id) on delete cascade,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    context_snapshot_id text not null references paper_context_snapshots(id) on delete restrict,
    created_at text not null
);

create index agent_paper_analysis_bindings_paper_idx
    on agent_paper_analysis_bindings(paper_id, created_at desc);
