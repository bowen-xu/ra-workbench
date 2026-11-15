create table relation_assertions_next (
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
        'related_to', 'depends_on', 'builds_on', 'extends', 'redefines',
        'contrasts_with', 'part_of'
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

insert into relation_assertions_next select * from relation_assertions;
drop table relation_assertions;
alter table relation_assertions_next rename to relation_assertions;

create index relation_assertions_project_idx
    on relation_assertions(project_id, updated_at desc);
create index relation_assertions_subject_idx
    on relation_assertions(subject_kind, subject_id, updated_at desc);
create index relation_assertions_object_idx
    on relation_assertions(object_kind, object_id, updated_at desc);

create table paper_concept_assertions (
    id text primary key,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    concept_id text references concepts(id) on delete restrict,
    term_usage_id text references term_usages(id) on delete restrict,
    current_revision integer not null default 1 check(current_revision >= 1),
    created_at text not null,
    updated_at text not null,
    check((concept_id is not null) != (term_usage_id is not null))
);

create table paper_concept_assertion_revisions (
    id text primary key,
    assertion_id text not null references paper_concept_assertions(id) on delete restrict,
    revision integer not null check(revision >= 1),
    paper_function text not null check(paper_function in ('foundational', 'prerequisite', 'focal')),
    novelty_status text not null check(novelty_status in (
        'pre_existing', 'adapted_by_paper', 'introduced_by_paper', 'unclear'
    )),
    explanation_status text not null check(explanation_status in (
        'explained_in_paper', 'mentioned_only', 'insufficient_evidence'
    )),
    explanation text not null check(length(explanation) <= 4000),
    role_rationale text not null check(length(trim(role_rationale)) between 1 and 2000),
    novelty_rationale text not null check(length(novelty_rationale) <= 2000),
    source_candidate_id text not null references ai_candidates(id) on delete restrict,
    source_candidate_revision integer not null check(source_candidate_revision >= 1),
    source_local_concept_id text not null check(length(source_local_concept_id) between 1 and 80),
    created_at text not null,
    unique(assertion_id, revision),
    unique(source_candidate_id, source_candidate_revision, source_local_concept_id),
    foreign key(source_candidate_id, source_candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict
);

create index paper_concept_assertions_paper_idx
    on paper_concept_assertions(paper_id, updated_at desc);
create index paper_concept_assertions_concept_idx
    on paper_concept_assertions(concept_id, updated_at desc);

create trigger paper_concept_assertion_revisions_no_update
before update on paper_concept_assertion_revisions begin
    select raise(abort, 'paper concept assertion revisions are immutable');
end;

create trigger paper_concept_assertion_revisions_no_delete
before delete on paper_concept_assertion_revisions begin
    select raise(abort, 'paper concept assertion revisions are immutable');
end;
