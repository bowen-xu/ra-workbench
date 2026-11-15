create table embedding_spaces (
    id text primary key check(length(id) between 1 and 160),
    provider_id text not null check(length(trim(provider_id)) between 1 and 120),
    region text not null default '' check(length(region) <= 120),
    model_id text not null check(length(trim(model_id)) between 1 and 240),
    model_revision text not null check(length(trim(model_revision)) between 1 and 240),
    dimension integer not null check(dimension between 1 and 65536),
    scalar_type text not null check(scalar_type = 'float32'),
    distance_metric text not null check(distance_metric = 'cosine'),
    normalization text not null check(normalization in ('l2', 'none')),
    document_input_role text not null check(length(trim(document_input_role)) between 1 and 120),
    query_input_role text not null check(length(trim(query_input_role)) between 1 and 120),
    instruction_set_version text not null check(length(trim(instruction_set_version)) between 1 and 120),
    created_at text not null,
    retired_at text,
    unique(
        provider_id, region, model_id, model_revision, dimension, scalar_type,
        distance_metric, normalization, document_input_role, query_input_role,
        instruction_set_version
    )
);

create table semantic_representations (
    id text primary key check(length(id) between 1 and 160),
    subject_kind text not null check(subject_kind in (
        'concept', 'definition_claim', 'paper_concept_assertion',
        'source_anchor', 'context_item', 'concept_candidate'
    )),
    subject_id text not null check(length(trim(subject_id)) between 1 and 200),
    subject_revision integer not null check(subject_revision >= 1),
    representation_kind text not null check(representation_kind in (
        'concept_identity', 'definition_claim', 'paper_concept_assertion',
        'source_passage', 'concept_candidate'
    )),
    namespace text not null check(namespace in (
        'accepted_concepts', 'candidate_staging', 'definition_claims', 'source_passages'
    )),
    language text not null default '' check(length(language) <= 32),
    render_version text not null check(length(trim(render_version)) between 1 and 120),
    rendered_text text not null check(length(trim(rendered_text)) between 1 and 32000),
    input_sha256 text not null check(length(input_sha256) = 64),
    created_at text not null,
    unique(subject_kind, subject_id, subject_revision, representation_kind, language, render_version),
    unique(input_sha256, representation_kind, language, render_version)
);

create trigger semantic_representations_no_update
before update on semantic_representations begin
    select raise(abort, 'semantic representations are immutable');
end;

create trigger semantic_representations_no_delete
before delete on semantic_representations begin
    select raise(abort, 'semantic representations are immutable');
end;

create table embedding_index_generations (
    id text primary key check(length(id) between 1 and 160),
    namespace text not null check(namespace in (
        'accepted_concepts', 'candidate_staging', 'definition_claims', 'source_passages'
    )),
    embedding_space_id text not null references embedding_spaces(id) on delete restrict,
    backend_kind text not null check(backend_kind = 'lancedb'),
    generation integer not null check(generation >= 1),
    status text not null check(status in (
        'building', 'ready', 'active', 'superseded', 'failed'
    )),
    relative_database_path text not null check(
        length(trim(relative_database_path)) between 1 and 500
        and instr(relative_database_path, '..') = 0
    ),
    lance_table text not null check(lance_table in (
        'accepted_concepts', 'candidate_staging', 'definition_claims', 'source_passages'
    )),
    lance_dataset_version integer check(lance_dataset_version is null or lance_dataset_version >= 1),
    source_high_watermark integer not null default 0 check(source_high_watermark >= 0),
    index_kind text not null check(index_kind in ('exact', 'ivf_flat', 'ivf_pq', 'ivf_hnsw')),
    config_json text not null check(json_valid(config_json) and json_type(config_json) = 'object'),
    member_count integer not null default 0 check(member_count >= 0),
    created_at text not null,
    activated_at text,
    superseded_at text,
    unique(namespace, embedding_space_id, generation),
    check(status != 'active' or activated_at is not null),
    check(status != 'superseded' or superseded_at is not null)
);

create unique index embedding_index_one_active_idx
    on embedding_index_generations(namespace, embedding_space_id)
    where status = 'active';

create table embedding_records (
    id text primary key check(length(id) between 1 and 160),
    representation_id text not null references semantic_representations(id) on delete restrict,
    embedding_space_id text not null references embedding_spaces(id) on delete restrict,
    index_generation_id text references embedding_index_generations(id) on delete restrict,
    lance_table text not null check(lance_table in (
        'accepted_concepts', 'candidate_staging', 'definition_claims', 'source_passages'
    )),
    lance_row_id text not null check(length(lance_row_id) between 1 and 160),
    lance_dataset_version integer check(lance_dataset_version is null or lance_dataset_version >= 1),
    vector_encoding text not null check(vector_encoding = 'float32'),
    vector_norm real check(vector_norm is null or vector_norm >= 0),
    generation_run_id text references ai_runs(id) on delete restrict,
    status text not null check(status in (
        'queued', 'running', 'ready', 'failed', 'stale', 'tombstoned'
    )),
    error_code text check(error_code is null or length(error_code) <= 120),
    created_at text not null,
    updated_at text not null,
    stale_at text,
    unique(representation_id, embedding_space_id),
    unique(embedding_space_id, lance_row_id),
    check(status != 'ready' or (
        index_generation_id is not null
        and lance_dataset_version is not null
        and vector_norm is not null
    )),
    check(status not in ('stale', 'tombstoned') or stale_at is not null)
);

create table embedding_outbox_jobs (
    id text primary key check(length(id) between 1 and 160),
    embedding_record_id text not null references embedding_records(id) on delete restrict,
    attempt integer not null check(attempt >= 1),
    status text not null check(status in (
        'queued', 'running', 'succeeded', 'failed', 'canceled', 'interrupted'
    )),
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 240),
    lease_owner text,
    lease_expires_at text,
    error_code text check(error_code is null or length(error_code) <= 120),
    created_at text not null,
    started_at text,
    finished_at text,
    unique(embedding_record_id, attempt),
    check(
        (status = 'running' and lease_owner is not null and lease_expires_at is not null)
        or status != 'running'
    )
);

create table embedding_reconciliation_runs (
    id text primary key check(length(id) between 1 and 160),
    index_generation_id text references embedding_index_generations(id) on delete restrict,
    status text not null check(status in ('running', 'succeeded', 'failed', 'canceled', 'interrupted')),
    checked_count integer not null default 0 check(checked_count >= 0),
    missing_count integer not null default 0 check(missing_count >= 0),
    orphan_count integer not null default 0 check(orphan_count >= 0),
    repaired_count integer not null default 0 check(repaired_count >= 0),
    error_code text check(error_code is null or length(error_code) <= 120),
    started_at text not null,
    finished_at text
);

create table embedding_generation_leases (
    id text primary key check(length(id) between 1 and 160),
    index_generation_id text not null references embedding_index_generations(id) on delete cascade,
    owner_kind text not null check(owner_kind in ('query', 'worker', 'algorithm', 'maintenance')),
    owner_id text not null check(length(owner_id) between 1 and 200),
    expires_at text not null,
    created_at text not null,
    unique(index_generation_id, owner_kind, owner_id)
);

create index semantic_representations_subject_idx
    on semantic_representations(subject_kind, subject_id, subject_revision desc);
create index embedding_records_status_idx
    on embedding_records(status, embedding_space_id, updated_at);
create index embedding_outbox_queue_idx
    on embedding_outbox_jobs(status, created_at);
create index embedding_reconciliation_status_idx
    on embedding_reconciliation_runs(status, started_at desc);
create index embedding_generation_leases_expiry_idx
    on embedding_generation_leases(index_generation_id, expires_at);
