alter table paper_ai_reading_sessions
    add column bootstrap_kind text not null default 'fresh_paper'
        check(bootstrap_kind in ('fresh_paper', 'structured_history'));

alter table paper_ai_reading_sessions
    add column source_workflow_id text
        references paper_analysis_workflows(id) on delete restrict;

alter table paper_ai_reading_sessions
    add column source_workflow_revision integer
        check(source_workflow_revision is null or source_workflow_revision >= 1);

alter table paper_ai_reading_sessions
    add column bootstrap_messages_json text not null default '[]'
        check(json_valid(bootstrap_messages_json)
            and json_type(bootstrap_messages_json) = 'array');

alter table paper_ai_reading_sessions
    add column bootstrap_hash text
        check(bootstrap_hash is null or length(bootstrap_hash) = 64)
        check(
            (bootstrap_kind = 'fresh_paper'
                and source_workflow_id is null
                and source_workflow_revision is null)
            or
            (bootstrap_kind = 'structured_history'
                and source_workflow_id is not null
                and source_workflow_revision is not null
                and bootstrap_hash is not null)
        );

create trigger paper_ai_reading_session_bootstrap_no_update
before update of bootstrap_kind, source_workflow_id, source_workflow_revision,
    bootstrap_messages_json, bootstrap_hash on paper_ai_reading_sessions
when old.bootstrap_kind != new.bootstrap_kind
    or old.source_workflow_id is not new.source_workflow_id
    or old.source_workflow_revision is not new.source_workflow_revision
    or old.bootstrap_messages_json != new.bootstrap_messages_json
    or old.bootstrap_hash is not new.bootstrap_hash
begin
    select raise(abort, 'paper reading bootstrap is immutable');
end;
