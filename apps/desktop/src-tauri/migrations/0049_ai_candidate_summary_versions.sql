create table ai_candidate_summary_versions (
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

create unique index ai_candidate_summary_versions_selected_idx
    on ai_candidate_summary_versions(candidate_id, candidate_revision)
    where is_selected = 1 and discarded_at is null;

create index ai_candidate_summary_versions_candidate_idx
    on ai_candidate_summary_versions(candidate_id, candidate_revision, version);
