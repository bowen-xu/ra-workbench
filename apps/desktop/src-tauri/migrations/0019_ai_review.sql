create table ai_evidence_bundles (
    id text primary key,
    run_id text not null unique references ai_runs(id) on delete restrict,
    scope_hash text not null check(length(scope_hash) = 64),
    retrieval_config_version text not null,
    validation_status text not null check(validation_status in ('valid', 'partial', 'invalid')),
    created_at text not null
);

create table ai_evidence_items (
    id text primary key,
    bundle_id text not null references ai_evidence_bundles(id) on delete restrict,
    citation_key text not null check(length(citation_key) between 1 and 80),
    ordinal integer not null check(ordinal >= 0),
    anchor_id text not null references source_anchors(id) on delete restrict,
    paper_id text not null,
    paper_title text not null,
    attachment_id text not null,
    content_sha256 text not null check(length(content_sha256) = 64),
    page_index integer not null check(page_index >= 0),
    page_label text,
    quote_exact text not null check(length(trim(quote_exact)) > 0),
    quote_prefix text not null default '',
    quote_suffix text not null default '',
    context_text text not null,
    relation_kind text not null check(relation_kind in ('direct_quote', 'context')),
    validation_status text not null check(validation_status in ('valid', 'invalid')),
    created_at text not null,
    unique(bundle_id, citation_key),
    unique(bundle_id, ordinal)
);

create table ai_candidates (
    id text primary key,
    run_id text not null unique references ai_runs(id) on delete restrict,
    evidence_bundle_id text not null references ai_evidence_bundles(id) on delete restrict,
    candidate_kind text not null check(candidate_kind = 'selection_explanation_v1'),
    schema_version integer not null check(schema_version = 1),
    current_revision integer not null default 1 check(current_revision >= 1),
    review_state text not null default 'unreviewed' check(review_state in (
        'unreviewed', 'edited', 'deferred', 'needs_more_evidence',
        'accepted_unchanged', 'accepted_after_edit', 'rejected'
    )),
    validation_status text not null check(validation_status = 'valid'),
    original_payload_hash text not null check(length(original_payload_hash) = 64),
    created_at text not null,
    updated_at text not null
);

create table ai_candidate_revisions (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    revision integer not null check(revision >= 1),
    payload_json text not null check(json_valid(payload_json)),
    payload_hash text not null check(length(payload_hash) = 64),
    editor_kind text not null check(editor_kind in ('ai', 'human')),
    parent_revision integer check(parent_revision is null or parent_revision >= 1),
    change_reason text not null default '',
    created_at text not null,
    unique(candidate_id, revision),
    check(
        (revision = 1 and editor_kind = 'ai' and parent_revision is null)
        or (revision > 1 and editor_kind = 'human' and parent_revision = revision - 1)
    )
);

create table ai_review_decisions (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    action text not null check(action in (
        'accept_unchanged', 'accept_after_edit', 'reject', 'defer', 'request_more_evidence'
    )),
    rationale text not null default '',
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    target_object_kind text check(target_object_kind is null or target_object_kind = 'reading_unit'),
    target_object_id text references structured_reading_units(id) on delete restrict,
    target_object_revision integer check(target_object_revision is null or target_object_revision >= 1),
    created_at text not null,
    check(
        (action in ('accept_unchanged', 'accept_after_edit')
         and target_object_kind = 'reading_unit'
         and target_object_id is not null
         and target_object_revision is not null)
        or
        (action not in ('accept_unchanged', 'accept_after_edit')
         and target_object_kind is null
         and target_object_id is null
         and target_object_revision is null)
    ),
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict
);

create table ai_evidence_inspections (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    evidence_item_id text not null references ai_evidence_items(id) on delete restrict,
    viewed_at text not null,
    unique(candidate_id, candidate_revision, evidence_item_id),
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict
);

create trigger ai_candidate_revisions_no_update
before update on ai_candidate_revisions
begin
    select raise(abort, 'ai candidate revisions are immutable');
end;

create trigger ai_candidate_revisions_no_delete
before delete on ai_candidate_revisions
begin
    select raise(abort, 'ai candidate revisions are immutable');
end;

create trigger ai_review_decisions_no_update
before update on ai_review_decisions
begin
    select raise(abort, 'ai review decisions are immutable');
end;

create trigger ai_review_decisions_no_delete
before delete on ai_review_decisions
begin
    select raise(abort, 'ai review decisions are immutable');
end;

create trigger ai_evidence_inspections_no_update
before update on ai_evidence_inspections
begin
    select raise(abort, 'ai evidence inspections are immutable');
end;

create trigger ai_evidence_inspections_no_delete
before delete on ai_evidence_inspections
begin
    select raise(abort, 'ai evidence inspections are immutable');
end;

create index ai_evidence_items_anchor_idx on ai_evidence_items(anchor_id);
create index ai_candidates_review_idx on ai_candidates(review_state, updated_at desc);
create index ai_review_decisions_candidate_idx
    on ai_review_decisions(candidate_id, created_at desc);
create index ai_evidence_inspections_candidate_idx
    on ai_evidence_inspections(candidate_id, candidate_revision, viewed_at desc);
