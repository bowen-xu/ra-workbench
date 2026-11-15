-- Portrait and structured-reading stages share a paper-owned current conversation.
-- Keep the pointer after archival/deletion so restoring older history cannot select it.
create table paper_generation_conversations (
    paper_id text primary key references bibliographic_items(id) on delete cascade,
    conversation_id text references agent_conversations(id) on delete set null
);

insert into paper_generation_conversations(paper_id, conversation_id)
select paper_id, conversation_id from (
    select paper_id, conversation_id,
           row_number() over (partition by paper_id order by created_at desc, conversation_order desc) as position
    from (
        select s.parent_object_id as paper_id, b.conversation_id, b.created_at, c.rowid as conversation_order
        from agent_legacy_ai_run_bindings b
        join ai_runs r on r.id = b.ai_run_id
        join ai_scope_items s on s.run_id = r.id and s.ordinal = 0
        join agent_conversations c on c.id = b.conversation_id
        where r.task_kind = 'paper_profile_classification'
          and s.parent_object_id in (select id from bibliographic_items)
        union all
        select b.paper_id, b.conversation_id, b.created_at, c.rowid
        from agent_paper_analysis_bindings b
        join agent_conversations c on c.id = b.conversation_id
    )
) where position = 1;

-- Multiple reading generations may append independent submissions to one conversation.
create table agent_paper_analysis_bindings_next (
    workflow_id text primary key references paper_analysis_workflows(id) on delete cascade,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    context_snapshot_id text not null references paper_context_snapshots(id) on delete restrict,
    created_at text not null
);
insert into agent_paper_analysis_bindings_next
    (workflow_id, conversation_id, paper_id, context_snapshot_id, created_at)
select workflow_id, conversation_id, paper_id, context_snapshot_id, created_at
from agent_paper_analysis_bindings;
drop table agent_paper_analysis_bindings;
alter table agent_paper_analysis_bindings_next rename to agent_paper_analysis_bindings;
create index agent_paper_analysis_bindings_paper_idx
    on agent_paper_analysis_bindings(paper_id, created_at desc);
create index agent_paper_analysis_bindings_conversation_idx
    on agent_paper_analysis_bindings(conversation_id);
