alter table structured_reading_units add column reverted_at text;

create table ai_overview_confirmation_batches (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    scope_kind text not null check(scope_kind in ('field', 'block', 'paper')),
    paper_id text not null references bibliographic_items(id) on delete restrict,
    project_id text references research_projects(id) on delete restrict,
    previous_candidate_review_state text not null check(previous_candidate_review_state in (
        'unreviewed', 'edited', 'deferred', 'needs_more_evidence',
        'accepted_unchanged', 'accepted_after_edit', 'rejected'
    )),
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    created_at text not null,
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict
);

create table ai_overview_field_confirmations (
    id text primary key,
    batch_id text not null references ai_overview_confirmation_batches(id) on delete restrict,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    field_kind text not null check(field_kind in (
        'research_question', 'approach', 'key_result', 'conclusion', 'limitation',
        'open_question', 'argument', 'objection', 'response'
    )),
    field_payload_hash text not null check(length(field_payload_hash) = 64),
    decision_kind text not null check(decision_kind in ('ai_unchanged', 'human_edited')),
    target_reading_unit_id text not null references structured_reading_units(id) on delete restrict,
    target_applied_revision integer not null check(target_applied_revision >= 1),
    previous_snapshot_json text check(previous_snapshot_json is null or json_valid(previous_snapshot_json)),
    created_at text not null,
    unique(batch_id, field_kind),
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict
);

create table ai_overview_confirmation_undos (
    id text primary key,
    batch_id text not null unique references ai_overview_confirmation_batches(id) on delete restrict,
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    created_at text not null
);

create trigger ai_overview_confirmation_batches_no_update
before update on ai_overview_confirmation_batches
begin
    select raise(abort, 'AI overview confirmation batches are immutable');
end;

create trigger ai_overview_confirmation_batches_no_delete
before delete on ai_overview_confirmation_batches
begin
    select raise(abort, 'AI overview confirmation batches are immutable');
end;

create trigger ai_overview_field_confirmations_no_update
before update on ai_overview_field_confirmations
begin
    select raise(abort, 'AI overview field confirmations are immutable');
end;

create trigger ai_overview_field_confirmations_no_delete
before delete on ai_overview_field_confirmations
begin
    select raise(abort, 'AI overview field confirmations are immutable');
end;

create trigger ai_overview_confirmation_undos_no_update
before update on ai_overview_confirmation_undos
begin
    select raise(abort, 'AI overview confirmation undos are immutable');
end;

create trigger ai_overview_confirmation_undos_no_delete
before delete on ai_overview_confirmation_undos
begin
    select raise(abort, 'AI overview confirmation undos are immutable');
end;

create index ai_overview_confirmation_batches_candidate_idx
    on ai_overview_confirmation_batches(candidate_id, candidate_revision, created_at desc);
create index ai_overview_field_confirmations_candidate_idx
    on ai_overview_field_confirmations(candidate_id, field_kind, created_at desc);
create index structured_reading_units_active_scope_idx
    on structured_reading_units(project_id, paper_id, unit_kind, updated_at desc)
    where reverted_at is null;
