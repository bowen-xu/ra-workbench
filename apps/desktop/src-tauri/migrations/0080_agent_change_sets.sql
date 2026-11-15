-- Frozen proposal metadata and object ownership; publication occurs in one transaction.
create table agent_change_sets (
    id text primary key,
    conversation_id text not null,
    request_id text not null,
    turn_id text not null,
    context_snapshot_id text not null,
    manuscript_id text not null references manuscripts(id) on delete restrict,
    lease_id text not null,
    manifest_hash text not null check(length(manifest_hash) = 64),
    inventory_hash text not null check(length(inventory_hash) = 64),
    schema_version integer not null check(schema_version = 1),
    status text not null default 'pending' check(status in
        ('pending','rejected','applying','applied','conflict','recovery_required','undone')),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    unique(conversation_id, request_id, inventory_hash),
    foreign key(conversation_id, request_id)
        references agent_submissions(conversation_id, request_id) on delete cascade
);
create index agent_change_sets_history_idx
    on agent_change_sets(conversation_id, manuscript_id, created_at desc, id desc);

create table agent_change_set_files (
    change_set_id text not null references agent_change_sets(id) on delete cascade,
    relative_path text not null check(length(relative_path) between 1 and 1024),
    before_sha256 text references content_objects(sha256) on delete restrict,
    after_sha256 text references content_objects(sha256) on delete restrict,
    before_executable integer check(before_executable in (0,1)),
    after_executable integer check(after_executable in (0,1)),
    source_json text check(source_json is null or json_valid(source_json)),
    primary key(change_set_id, relative_path),
    check(before_sha256 is not null or after_sha256 is not null),
    check((before_sha256 is null) = (before_executable is null)),
    check((after_sha256 is null) = (after_executable is null)),
    check((before_sha256 is null) = (source_json is null))
);
create index agent_change_set_files_before_idx on agent_change_set_files(before_sha256);
create index agent_change_set_files_after_idx on agent_change_set_files(after_sha256);

-- Preserve frozen facts; lifecycle decisions update status only.
create trigger agent_change_sets_immutable
before update of id, conversation_id, request_id, turn_id, context_snapshot_id,
    manuscript_id, lease_id, manifest_hash, inventory_hash, schema_version, created_at
on agent_change_sets
begin
    select raise(abort, 'immutable change set');
end;
create trigger agent_change_set_files_immutable
before update on agent_change_set_files
begin
    select raise(abort, 'immutable change set file');
end;

drop view content_object_references;
create view content_object_references as
select content_sha256, 'attachment' as owner_kind, id as owner_id, 'content' as role
from attachments
union all
select content_sha256, 'source_anchor' as owner_kind, id as owner_id, 'source' as role
from source_anchors
union all
select content_sha256, 'note_block' as owner_kind, block_id as owner_id, 'live_note_asset' as role
from note_block_assets
union all
select content_sha256, 'note_revision' as owner_kind, revision_id as owner_id, 'history_asset' as role
from note_revision_assets
union all
select content_sha256, 'manuscript_compile_artifact' as owner_kind, id as owner_id, 'compile_artifact' as role
from manuscript_compile_artifacts
where content_sha256 is not null
union all
select content_sha256, 'docling_parse_run' as owner_kind, id as owner_id, 'structured_pdf_parse' as role
from docling_parse_runs
union all
select image_content_sha256, 'paper_ai_reading_turn_context' as owner_kind, id as owner_id, 'paper_image' as role
from paper_ai_reading_turn_context_items
where image_content_sha256 is not null
union all
select image_content_sha256, 'ai_docling_evidence_context' as owner_kind, evidence_item_id as owner_id, 'paper_image' as role
from ai_docling_evidence_context_items
where image_content_sha256 is not null
union all
select image_content_sha256, 'paper_context_snapshot' as owner_kind,
       snapshot_id || ':' || context_item_id as owner_id, 'paper_image' as role
from paper_context_items
where image_content_sha256 is not null
union all
select before_sha256, 'agent_change_set' as owner_kind, change_set_id as owner_id, 'before' as role
from agent_change_set_files where before_sha256 is not null
union all
select after_sha256, 'agent_change_set' as owner_kind, change_set_id as owner_id, 'after' as role
from agent_change_set_files where after_sha256 is not null;

create trigger content_object_gc_agent_change_insert
after insert on agent_change_set_files
begin
    delete from content_object_gc_candidates
    where content_sha256 in (new.before_sha256, new.after_sha256);
end;
create trigger content_object_gc_agent_change_delete
after delete on agent_change_set_files
begin
    insert into content_object_gc_candidates
        (content_sha256, unreferenced_since, not_before, reason)
    select c.sha256, strftime('%Y-%m-%dT%H:%M:%fZ','now'),
        strftime('%Y-%m-%dT%H:%M:%fZ','now','+30 days'), 'live_reference_removed'
    from content_objects c
    where c.sha256 in (old.before_sha256, old.after_sha256)
      and not exists(select 1 from content_object_references r where r.content_sha256=c.sha256)
    on conflict(content_sha256) do nothing;
end;
