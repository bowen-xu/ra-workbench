create table if not exists ai_candidate_summary_versions (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    version integer not null check(version >= 1),
    summary_text text not null check(length(trim(summary_text)) between 1 and 1200),
    source_kind text not null check(source_kind in ('local_extractive', 'ai_generated', 'human')),
    is_selected integer not null default 0 check(is_selected in (0, 1)),
    idempotency_key text not null check(length(trim(idempotency_key)) between 1 and 200),
    created_at text not null,
    discarded_at text,
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict,
    unique(candidate_id, candidate_revision, version),
    unique(candidate_id, idempotency_key),
    check(discarded_at is null or is_selected = 0)
);

create unique index if not exists ai_candidate_summary_versions_selected_idx
    on ai_candidate_summary_versions(candidate_id, candidate_revision)
    where is_selected = 1 and discarded_at is null;

create index if not exists ai_candidate_summary_versions_candidate_idx
    on ai_candidate_summary_versions(candidate_id, candidate_revision, version);

create table migration_checksums (
    version integer primary key references schema_migrations(version) on delete restrict,
    sha256 text not null check(length(sha256) = 64),
    recorded_at text not null
);

create table ai_candidate_recovery_audits (
    id text primary key,
    candidate_id text not null unique references ai_candidates(id) on delete restrict,
    run_id text not null references ai_runs(id) on delete restrict,
    workflow_id text references paper_analysis_workflows(id) on delete restrict,
    recovery_kind text not null check(recovery_kind in (
        'run_only', 'paper_overview', 'paper_concept_map'
    )),
    recovery_reason text not null,
    recovery_version integer not null check(recovery_version >= 1),
    original_run_status text not null,
    original_run_error_code text,
    original_run_error_message text,
    original_attempt_status text,
    original_attempt_error_code text,
    original_attempt_error_message text,
    recovered_at text not null
);

create index ai_candidate_recovery_audits_run_idx
    on ai_candidate_recovery_audits(run_id, recovered_at);
