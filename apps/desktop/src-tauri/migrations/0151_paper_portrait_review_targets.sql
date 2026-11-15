-- A reviewed portrait targets its immutable candidate revision, never a reading unit.
create table ai_review_decisions_next (
    id text primary key,
    candidate_id text not null references ai_candidates(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    action text not null check(action in (
        'accept_unchanged', 'accept_after_edit', 'reject', 'defer', 'request_more_evidence'
    )),
    rationale text not null default '',
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    target_object_kind text check(target_object_kind is null or target_object_kind in (
        'reading_unit', 'reference_record', 'derived_research_view', 'paper_portrait'
    )),
    target_object_id text,
    target_object_revision integer check(target_object_revision is null or target_object_revision >= 1),
    created_at text not null,
    check(
        (action in ('accept_unchanged', 'accept_after_edit')
         and target_object_kind is not null and target_object_id is not null
         and target_object_revision is not null)
        or (action not in ('accept_unchanged', 'accept_after_edit')
            and target_object_kind is null and target_object_id is null
            and target_object_revision is null)
    ),
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions(candidate_id, revision) on delete restrict
);
insert into ai_review_decisions_next select * from ai_review_decisions;
drop table ai_review_decisions;
alter table ai_review_decisions_next rename to ai_review_decisions;
create index ai_review_decisions_candidate_idx on ai_review_decisions(candidate_id, created_at desc);
create trigger ai_review_decisions_no_update before update on ai_review_decisions begin
    select raise(abort, 'ai review decisions are immutable');
end;
create trigger ai_review_decisions_no_delete before delete on ai_review_decisions begin
    select raise(abort, 'ai review decisions are immutable');
end;
create trigger ai_review_decisions_target_exists before insert on ai_review_decisions begin
    select case
        when new.target_object_kind = 'reading_unit' and not exists(
            select 1 from structured_reading_units
            where id = new.target_object_id and revision = new.target_object_revision
        ) then raise(abort, 'AI review target reading unit does not exist')
        when new.target_object_kind = 'reference_record' and not exists(
            select 1 from reference_record_revisions
            where reference_record_id = new.target_object_id and revision = new.target_object_revision
        ) then raise(abort, 'AI review target reference record does not exist')
        when new.target_object_kind = 'derived_research_view' and not exists(
            select 1 from derived_research_view_revisions
            where view_id = new.target_object_id and revision = new.target_object_revision
        ) then raise(abort, 'AI review target derived view does not exist')
        when new.target_object_kind = 'paper_portrait' and (
            new.target_object_id != new.candidate_id or new.target_object_revision != new.candidate_revision
            or not exists(select 1 from ai_candidates c join ai_candidate_revisions r on r.candidate_id=c.id
                where c.id = new.target_object_id and c.candidate_kind = 'paper_portrait_v1'
                  and r.revision = new.target_object_revision)
        ) then raise(abort, 'AI review target portrait revision does not exist')
    end;
end;
