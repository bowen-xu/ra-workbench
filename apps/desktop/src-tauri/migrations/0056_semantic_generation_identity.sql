drop trigger semantic_representations_no_update;
drop trigger semantic_representations_no_delete;
drop index semantic_representations_subject_idx;
drop index embedding_records_status_idx;
drop index embedding_outbox_queue_idx;

alter table embedding_outbox_jobs rename to embedding_outbox_jobs_v55;
alter table embedding_records rename to embedding_records_v55;
alter table semantic_representations rename to semantic_representations_v55;

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
    unique(subject_kind, subject_id, subject_revision, representation_kind, language, render_version)
);

insert into semantic_representations
select id, subject_kind, subject_id, subject_revision, representation_kind, namespace,
       language, render_version, rendered_text, input_sha256, created_at
from semantic_representations_v55;

create trigger semantic_representations_no_update
before update on semantic_representations begin
    select raise(abort, 'semantic representations are immutable');
end;

create trigger semantic_representations_no_delete
before delete on semantic_representations begin
    select raise(abort, 'semantic representations are immutable');
end;

create table embedding_records (
    id text primary key check(length(id) between 1 and 160),
    representation_id text not null references semantic_representations(id) on delete restrict,
    embedding_space_id text not null references embedding_spaces(id) on delete restrict,
    index_generation_id text not null references embedding_index_generations(id) on delete restrict,
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
    unique(representation_id, embedding_space_id, index_generation_id),
    unique(index_generation_id, lance_row_id),
    check(status != 'ready' or (
        lance_dataset_version is not null and vector_norm is not null
    )),
    check(status not in ('stale', 'tombstoned') or stale_at is not null)
);

insert into embedding_records
select id, representation_id, embedding_space_id, index_generation_id, lance_table,
       lance_row_id, lance_dataset_version, vector_encoding, vector_norm, generation_run_id,
       status, error_code, created_at, updated_at, stale_at
from embedding_records_v55;

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

insert into embedding_outbox_jobs
select id, embedding_record_id, attempt, status, idempotency_key, lease_owner,
       lease_expires_at, error_code, created_at, started_at, finished_at
from embedding_outbox_jobs_v55;

drop table embedding_outbox_jobs_v55;
drop table embedding_records_v55;
drop table semantic_representations_v55;

create index semantic_representations_subject_idx
    on semantic_representations(subject_kind, subject_id, subject_revision desc);
create index semantic_representations_hash_idx
    on semantic_representations(input_sha256, representation_kind, render_version);
create index embedding_records_status_idx
    on embedding_records(status, embedding_space_id, updated_at);
create index embedding_records_generation_idx
    on embedding_records(index_generation_id, status, lance_row_id);
create index embedding_outbox_queue_idx
    on embedding_outbox_jobs(status, created_at);
