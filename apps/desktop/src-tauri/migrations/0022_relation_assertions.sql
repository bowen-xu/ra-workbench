create table research_object_revisions_next (
    id text primary key,
    object_kind text not null check(object_kind in (
        'claim', 'reading_unit', 'term', 'concept', 'term_usage', 'definition_claim',
        'relation_assertion'
    )),
    object_id text not null,
    revision integer not null check(revision >= 1),
    snapshot_json text not null check(json_valid(snapshot_json)),
    editor_kind text not null check(editor_kind in ('human', 'migration', 'ai_review', 'system')),
    change_reason text not null default '',
    created_at text not null,
    unique(object_kind, object_id, revision)
);

insert into research_object_revisions_next select * from research_object_revisions;
drop table research_object_revisions;
alter table research_object_revisions_next rename to research_object_revisions;

create trigger research_object_revisions_no_update
before update on research_object_revisions
begin
    select raise(abort, 'research object revisions are immutable');
end;

create trigger research_object_revisions_no_delete
before delete on research_object_revisions
begin
    select raise(abort, 'research object revisions are immutable');
end;

create index research_object_revisions_object_idx
    on research_object_revisions(object_kind, object_id, revision desc);

create table relation_assertions (
    id text primary key,
    project_id text references research_projects(id) on delete set null,
    subject_kind text not null check(subject_kind in (
        'concept', 'definition_claim', 'reading_unit', 'paper', 'note_page'
    )),
    subject_id text not null,
    subject_label_snapshot text not null check(length(subject_label_snapshot) between 1 and 1000),
    predicate_kind text not null check(predicate_kind in (
        'broader_than', 'narrower_than', 'competes_with', 'historically_derived_from',
        'approximately_corresponds_to', 'defines', 'discusses', 'supports', 'opposes',
        'related_to'
    )),
    object_kind text not null check(object_kind in (
        'concept', 'definition_claim', 'reading_unit', 'paper', 'note_page'
    )),
    object_id text not null,
    object_label_snapshot text not null check(length(object_label_snapshot) between 1 and 1000),
    scope_note text not null default '' check(length(scope_note) <= 4000),
    provenance_kind text not null default 'human_original' check(provenance_kind in (
        'human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit'
    )),
    epistemic_role text not null default 'assertion' check(epistemic_role = 'assertion'),
    revision integer not null default 1 check(revision >= 1),
    created_at text not null,
    updated_at text not null,
    check(subject_kind != object_kind or subject_id != object_id)
);

create table relation_assertion_evidence_links (
    id text primary key,
    relation_assertion_id text not null references relation_assertions(id) on delete cascade,
    anchor_id text not null references source_anchors(id) on delete restrict,
    stance text not null check(stance in ('supports', 'opposes', 'context')),
    rationale text not null default '' check(length(rationale) <= 2000),
    created_at text not null,
    unique(relation_assertion_id, anchor_id, stance)
);

create index relation_assertions_project_idx
    on relation_assertions(project_id, updated_at desc);
create index relation_assertions_subject_idx
    on relation_assertions(subject_kind, subject_id, updated_at desc);
create index relation_assertions_object_idx
    on relation_assertions(object_kind, object_id, updated_at desc);
create index relation_assertion_evidence_anchor_idx
    on relation_assertion_evidence_links(anchor_id);
