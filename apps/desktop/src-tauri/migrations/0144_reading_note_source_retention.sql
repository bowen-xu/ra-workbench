-- Keep retained source attachments reachable independently of an active anchor.
drop trigger if exists content_object_gc_reading_source_insert;
drop trigger if exists content_object_gc_reading_source_delete;
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
from agent_change_set_files where after_sha256 is not null
union all
select content_sha256, 'reading_note_source', page_id, 'source' from reading_note_sources;
create trigger content_object_gc_reading_source_insert after insert on reading_note_sources
begin
 delete from content_object_gc_candidates where content_sha256=new.content_sha256;
end;
create trigger content_object_gc_reading_source_delete after delete on reading_note_sources
when exists(select 1 from content_objects where sha256=old.content_sha256)
 and not exists(select 1 from content_object_references where content_sha256=old.content_sha256)
begin
 insert into content_object_gc_candidates(content_sha256,unreferenced_since,not_before,reason)
 values(old.content_sha256,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now','+30 days'),'live_reference_removed')
 on conflict(content_sha256) do nothing;
end;
delete from content_object_gc_candidates where content_sha256 in(select content_sha256 from reading_note_sources);
