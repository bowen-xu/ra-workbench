create table ai_candidate_revisions_next (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    revision integer not null check(revision >= 1),
    payload_json text not null check(json_valid(payload_json)),
    payload_hash text not null check(length(payload_hash) = 64),
    editor_kind text not null check(editor_kind in ('ai', 'human', 'ai_retry')),
    source_run_id text unique references ai_runs(id) on delete restrict,
    parent_revision integer check(parent_revision is null or parent_revision >= 1),
    change_reason text not null default '',
    created_at text not null,
    unique(candidate_id, revision),
    check(
        (revision = 1 and editor_kind = 'ai' and source_run_id is not null
         and parent_revision is null)
        or (revision > 1 and editor_kind = 'human' and source_run_id is null
            and parent_revision = revision - 1)
        or (revision > 1 and editor_kind = 'ai_retry' and source_run_id is not null
            and parent_revision = revision - 1)
    )
);

create table ai_review_decisions_next (
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
        references ai_candidate_revisions_next(candidate_id, revision) on delete restrict
);

create table ai_evidence_inspections_next (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    evidence_item_id text not null references ai_evidence_items(id) on delete restrict,
    viewed_at text not null,
    unique(candidate_id, candidate_revision, evidence_item_id),
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions_next(candidate_id, revision) on delete restrict
);

insert into ai_candidate_revisions_next
    (id, candidate_id, revision, payload_json, payload_hash, editor_kind, source_run_id,
     parent_revision, change_reason, created_at)
select r.id, r.candidate_id, r.revision, r.payload_json, r.payload_hash, r.editor_kind,
       case when r.editor_kind = 'ai' then c.run_id else null end,
       r.parent_revision, r.change_reason, r.created_at
from ai_candidate_revisions r join ai_candidates c on c.id = r.candidate_id;

insert into ai_review_decisions_next select * from ai_review_decisions;
insert into ai_evidence_inspections_next select * from ai_evidence_inspections;

drop table ai_evidence_inspections;
drop table ai_review_decisions;
drop table ai_candidate_revisions;

alter table ai_candidate_revisions_next rename to ai_candidate_revisions;
alter table ai_review_decisions_next rename to ai_review_decisions;
alter table ai_evidence_inspections_next rename to ai_evidence_inspections;

create table ai_overview_retry_runs (
    run_id text primary key references ai_runs(id) on delete restrict,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    base_revision integer not null check(base_revision >= 1),
    target_fields_json text not null check(
        json_valid(target_fields_json)
        and json_type(target_fields_json) = 'array'
        and json_array_length(target_fields_json) between 1 and 6
    ),
    created_at text not null,
    foreign key(candidate_id, base_revision)
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

create trigger ai_overview_retry_runs_no_update
before update on ai_overview_retry_runs
begin
    select raise(abort, 'ai overview retry contexts are immutable');
end;

create trigger ai_overview_retry_runs_no_delete
before delete on ai_overview_retry_runs
begin
    select raise(abort, 'ai overview retry contexts are immutable');
end;

create index ai_review_decisions_candidate_idx
    on ai_review_decisions(candidate_id, created_at desc);
create index ai_evidence_inspections_candidate_idx
    on ai_evidence_inspections(candidate_id, candidate_revision, viewed_at desc);
create index ai_overview_retry_candidate_idx
    on ai_overview_retry_runs(candidate_id, base_revision);
