create table ai_overview_template_runs (
    run_id text primary key references ai_runs(id) on delete restrict,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    portrait_candidate_id text not null,
    portrait_revision integer not null check(portrait_revision >= 1),
    template_json text not null check(json_valid(template_json)),
    created_at text not null,
    foreign key(portrait_candidate_id, portrait_revision) references ai_candidate_revisions(candidate_id, revision) on delete restrict
);
create trigger ai_overview_template_runs_no_update before update on ai_overview_template_runs begin
    select raise(abort, 'overview template snapshots are immutable');
end;
create trigger ai_overview_template_runs_no_delete before delete on ai_overview_template_runs begin
    select raise(abort, 'overview template snapshots are immutable');
end;
create table reading_unit_template_fields (
    reading_unit_id text primary key references structured_reading_units(id) on delete restrict,
    portrait_candidate_id text not null references ai_candidates(id) on delete restrict,
    field_id text not null check(length(field_id) between 1 and 64),
    created_at text not null
);
create index reading_unit_template_fields_identity_idx on reading_unit_template_fields(portrait_candidate_id, field_id);
create trigger reading_unit_template_fields_no_update before update on reading_unit_template_fields begin
    select raise(abort, 'reading template field identity is immutable');
end;
create trigger reading_unit_template_fields_no_delete before delete on reading_unit_template_fields begin
    select raise(abort, 'reading template field identity is immutable');
end;
create table ai_overview_field_confirmations_next (
    id text primary key,
    batch_id text not null references ai_overview_confirmation_batches(id) on delete restrict,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    field_kind text not null check(length(field_kind) between 1 and 64),
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

insert into ai_overview_field_confirmations_next select * from ai_overview_field_confirmations;
drop table ai_overview_field_confirmations;
alter table ai_overview_field_confirmations_next rename to ai_overview_field_confirmations;
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

create index ai_overview_field_confirmations_candidate_idx on ai_overview_field_confirmations(candidate_id, field_kind, created_at desc);
create trigger ai_overview_template_field_guard before insert on ai_overview_field_confirmations
when not exists (
    select 1 from ai_candidate_revisions r, json_each(r.payload_json, '$.fields') f
    where r.candidate_id = new.candidate_id and r.revision = new.candidate_revision
      and json_extract(r.payload_json, '$.kind') = 'paper_overview_unit_v1'
      and json_extract(f.value, '$.field') = new.field_kind
)
begin select raise(abort, 'confirmation field outside candidate'); end;
