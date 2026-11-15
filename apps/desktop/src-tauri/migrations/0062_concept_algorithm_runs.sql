create table concept_algorithm_runs (
    id text primary key,
    algorithm_kind text not null check(algorithm_kind in ('concept_evolution', 'shared_foundations')),
    algorithm_id text not null check(length(algorithm_id) between 1 and 120),
    algorithm_version integer not null check(algorithm_version >= 1),
    parameters_json text not null check(json_valid(parameters_json) and json_type(parameters_json) = 'object'),
    parameters_hash text not null check(length(parameters_hash) = 64),
    embedding_space_id text references embedding_spaces(id) on delete restrict,
    index_generation_id text references embedding_index_generations(id) on delete restrict,
    concept_revision_watermark integer not null check(concept_revision_watermark >= 0),
    relation_revision_watermark integer not null check(relation_revision_watermark >= 0),
    paper_scope_json text not null check(json_valid(paper_scope_json) and json_type(paper_scope_json) = 'array'),
    paper_scope_hash text not null check(length(paper_scope_hash) = 64),
    status text not null check(status in ('running', 'succeeded', 'failed', 'canceled', 'stale')),
    result_count integer not null default 0 check(result_count >= 0),
    error_code text check(error_code is null or length(error_code) <= 120),
    created_at text not null,
    started_at text not null,
    finished_at text,
    check((embedding_space_id is null) = (index_generation_id is null)),
    check(status = 'running' or finished_at is not null)
);

create table concept_evolution_results (
    run_id text not null references concept_algorithm_runs(id) on delete cascade,
    ordinal integer not null check(ordinal >= 0),
    concept_id text not null references concepts(id) on delete restrict,
    concept_revision integer not null check(concept_revision >= 1),
    concept_label text not null,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    paper_title text not null,
    issued_year integer,
    assertion_id text not null references paper_concept_assertions(id) on delete restrict,
    assertion_revision integer not null check(assertion_revision >= 1),
    paper_function text not null check(paper_function in ('foundational', 'prerequisite', 'focal')),
    novelty_status text not null check(novelty_status in (
        'pre_existing', 'adapted_by_paper', 'introduced_by_paper', 'unclear'
    )),
    event_kind text not null check(event_kind in ('introduced', 'adapted', 'reused', 'unclear')),
    explanation text not null,
    evidence_anchor_id text references source_anchors(id) on delete restrict,
    primary key(run_id, ordinal),
    unique(run_id, assertion_id, assertion_revision)
);

create table paper_foundation_similarity_results (
    run_id text not null references concept_algorithm_runs(id) on delete cascade,
    ordinal integer not null check(ordinal >= 0),
    left_paper_id text not null references bibliographic_items(id) on delete restrict,
    left_paper_title text not null,
    right_paper_id text not null references bibliographic_items(id) on delete restrict,
    right_paper_title text not null,
    shared_concepts_json text not null check(
        json_valid(shared_concepts_json) and json_type(shared_concepts_json) = 'array'
    ),
    weighted_jaccard real not null check(weighted_jaccard between 0 and 1),
    shared_concept_count integer not null check(shared_concept_count >= 0),
    left_foundation_count integer not null check(left_foundation_count >= 0),
    right_foundation_count integer not null check(right_foundation_count >= 0),
    primary key(run_id, ordinal),
    unique(run_id, left_paper_id, right_paper_id),
    check(left_paper_id < right_paper_id)
);

create index concept_algorithm_runs_kind_created_idx
    on concept_algorithm_runs(algorithm_kind, created_at desc);
create index concept_evolution_results_concept_idx
    on concept_evolution_results(concept_id, issued_year, ordinal);
create index paper_foundation_similarity_left_idx
    on paper_foundation_similarity_results(left_paper_id, weighted_jaccard desc);
create index paper_foundation_similarity_right_idx
    on paper_foundation_similarity_results(right_paper_id, weighted_jaccard desc);
