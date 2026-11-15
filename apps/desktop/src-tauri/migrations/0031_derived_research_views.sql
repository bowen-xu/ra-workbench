create table derived_research_views (
    id text primary key,
    project_id text not null references research_projects(id) on delete cascade,
    view_kind text not null check(view_kind in (
        'taxonomy', 'historical_context', 'gap_hypothesis', 'innovation_hypothesis'
    )),
    title text not null check(length(trim(title)) between 1 and 240),
    scope_note text not null default '' check(length(scope_note) <= 4000),
    inputs_json text not null check(json_valid(inputs_json)),
    items_json text not null check(json_valid(items_json)),
    review_state text not null check(review_state in (
        'unreviewed', 'accepted', 'rejected', 'deferred'
    )),
    provenance_kind text not null check(provenance_kind in (
        'human_original', 'ai_suggestion', 'ai_accepted_after_edit'
    )),
    provider_id text,
    model_id text,
    generated_at text not null,
    current_revision integer not null check(current_revision >= 1),
    created_at text not null,
    updated_at text not null
);

create table derived_research_view_revisions (
    id text primary key,
    view_id text not null references derived_research_views(id) on delete restrict,
    revision integer not null check(revision >= 1),
    snapshot_json text not null check(json_valid(snapshot_json)),
    editor_kind text not null check(editor_kind in ('human', 'ai')),
    change_reason text not null check(length(change_reason) between 1 and 120),
    created_at text not null,
    unique(view_id, revision)
);

create index derived_research_views_project_idx
    on derived_research_views(project_id, view_kind, updated_at desc);

create trigger derived_research_view_revisions_no_update
before update on derived_research_view_revisions begin
    select raise(abort, 'derived research view revisions are immutable');
end;
create trigger derived_research_view_revisions_no_delete
before delete on derived_research_view_revisions begin
    select raise(abort, 'derived research view revisions are immutable');
end;
