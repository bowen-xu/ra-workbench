create table ai_suggestion_item_decisions (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    suggestion_id text not null check(length(suggestion_id) between 1 and 80),
    action text not null check(action in (
        'accept_unchanged', 'accept_after_edit', 'reject', 'defer', 'request_more_evidence'
    )),
    resolution_kind text not null check(resolution_kind in (
        'none', 'create_concept', 'link_concept', 'unresolved',
        'attach_definition', 'assert_relation'
    )),
    rationale text not null default '' check(length(rationale) <= 2000),
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    request_hash text not null check(length(request_hash) = 64),
    target_object_kind text check(target_object_kind is null or target_object_kind in (
        'concept', 'term_usage', 'definition_claim', 'relation_assertion'
    )),
    target_object_id text,
    target_object_revision integer check(
        target_object_revision is null or target_object_revision >= 1
    ),
    created_at text not null,
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict,
    check(
        (action in ('accept_unchanged', 'accept_after_edit')
         and resolution_kind != 'none'
         and target_object_kind is not null
         and target_object_id is not null
         and target_object_revision is not null)
        or
        (action not in ('accept_unchanged', 'accept_after_edit')
         and resolution_kind = 'none'
         and target_object_kind is null
         and target_object_id is null
         and target_object_revision is null)
    ),
    check(
        resolution_kind not in ('create_concept', 'link_concept')
        or target_object_kind = 'concept'
    ),
    check(resolution_kind != 'unresolved' or target_object_kind = 'term_usage'),
    check(
        resolution_kind != 'attach_definition'
        or target_object_kind = 'definition_claim'
    ),
    check(
        resolution_kind != 'assert_relation'
        or target_object_kind = 'relation_assertion'
    )
);

create index ai_suggestion_item_decisions_candidate_idx
    on ai_suggestion_item_decisions(
        candidate_id, candidate_revision, suggestion_id, created_at
    );

create trigger ai_suggestion_item_decisions_no_update
before update on ai_suggestion_item_decisions begin
    select raise(abort, 'AI suggestion item decisions are immutable');
end;

create trigger ai_suggestion_item_decisions_no_delete
before delete on ai_suggestion_item_decisions begin
    select raise(abort, 'AI suggestion item decisions are immutable');
end;
