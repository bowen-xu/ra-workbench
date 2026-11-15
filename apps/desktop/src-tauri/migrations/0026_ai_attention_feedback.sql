create table ai_attention_feedback (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    suggestion_id text not null check(length(suggestion_id) between 1 and 80),
    feedback_kind text not null check(feedback_kind in ('useful', 'not_useful', 'dismissed')),
    note text not null default '' check(length(note) <= 1000),
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    created_at text not null,
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict
);

create trigger ai_attention_feedback_no_update
before update on ai_attention_feedback begin
    select raise(abort, 'AI attention feedback is immutable');
end;

create trigger ai_attention_feedback_no_delete
before delete on ai_attention_feedback begin
    select raise(abort, 'AI attention feedback is immutable');
end;

create index ai_attention_feedback_candidate_idx
    on ai_attention_feedback(candidate_id, candidate_revision, suggestion_id, created_at);
