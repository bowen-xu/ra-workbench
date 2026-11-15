create table ai_citation_assessments (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    statement_id text not null check(length(statement_id) between 1 and 160),
    evidence_item_id text not null references ai_evidence_items(id) on delete restrict,
    assessment_revision integer not null check(assessment_revision >= 1),
    semantic_relation text not null check(semantic_relation in (
        'supports', 'partially_supports', 'context_only', 'opposes',
        'unsupported', 'uncertain'
    )),
    rationale text not null default '' check(length(rationale) <= 2000),
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    request_hash text not null check(length(request_hash) = 64),
    assessed_at text not null,
    unique(
        candidate_id, candidate_revision, statement_id,
        evidence_item_id, assessment_revision
    ),
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict
);

create trigger ai_citation_assessments_no_update
before update on ai_citation_assessments
begin
    select raise(abort, 'ai citation assessments are immutable');
end;

create trigger ai_citation_assessments_no_delete
before delete on ai_citation_assessments
begin
    select raise(abort, 'ai citation assessments are immutable');
end;

create index ai_citation_assessments_candidate_idx
    on ai_citation_assessments(
        candidate_id, candidate_revision, statement_id,
        evidence_item_id, assessment_revision desc
    );
