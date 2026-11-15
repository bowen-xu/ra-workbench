-- Captured evidence references use the existing content-addressed object store.
create table agent_evidence_images (
 id text primary key,
 content_sha256 text not null references content_objects(sha256) on delete cascade,
 media_type text not null check(media_type in ('image/png','image/jpeg','image/webp')),
 byte_size integer not null check(byte_size between 1 and 524288),
 draft_held integer not null check(draft_held in (0,1))
);
create index agent_evidence_images_content on agent_evidence_images(content_sha256);
-- Preserve all deployed reference sources from migration 0147.
drop view content_object_references;
create view content_object_references as
select content_sha256, 'attachment' as owner_kind, id as owner_id, 'content' as role from attachments
union all select content_sha256,'source_anchor',id,'source' from source_anchors
union all select content_sha256,'note_block',block_id,'live_note_asset' from note_block_assets
union all select content_sha256,'note_revision',revision_id,'history_asset' from note_revision_assets
union all select content_sha256,'manuscript_compile_artifact',id,'compile_artifact' from manuscript_compile_artifacts where content_sha256 is not null
union all select content_sha256,'docling_parse_run',id,'structured_pdf_parse' from docling_parse_runs
union all select image_content_sha256,'paper_ai_reading_turn_context',id,'paper_image' from paper_ai_reading_turn_context_items where image_content_sha256 is not null
union all select image_content_sha256,'ai_docling_evidence_context',evidence_item_id,'paper_image' from ai_docling_evidence_context_items where image_content_sha256 is not null
union all select image_content_sha256,'paper_context_snapshot',snapshot_id||':'||context_item_id,'paper_image' from paper_context_items where image_content_sha256 is not null
union all select before_sha256,'agent_change_set',change_set_id,'before' from agent_change_set_files where before_sha256 is not null
union all select after_sha256,'agent_change_set',change_set_id,'after' from agent_change_set_files where after_sha256 is not null
union all select content_sha256,'reading_note_source',page_id,'source' from reading_note_sources
union all select content_sha256,'agent_hook_spill_artifact',id,'history_asset' from agent_hook_spill_artifacts
union all select e.content_sha256,'agent_evidence_image',e.id,case when e.draft_held=1 then 'agent_draft' else 'history_asset' end
from agent_evidence_images e where e.draft_held=1
 or exists(select 1 from agent_submissions s,json_each(s.input_json,'$.userInput') i where json_extract(i.value,'$.image.captureId')=e.id)
 or exists(select 1 from agent_steering_resources s,json_each(s.input_json) i where json_extract(i.value,'$.image.captureId')=e.id)
 or exists(select 1 from agent_input_queues q,json_each(q.snapshot_json,'$.inputs') i,json_each(i.value,'$.userInput') r where json_extract(r.value,'$.image.captureId')=e.id)
 or exists(select 1 from agent_input_queue_receipts q,json_tree(q.command_json) i where i.key='captureId' and i.value=e.id)
 or exists(select 1 from agent_input_queue_receipts q,json_tree(q.result_json) i where i.key='captureId' and i.value=e.id);
create trigger agent_evidence_pin_clears_gc after insert on agent_evidence_images
begin delete from content_object_gc_candidates where content_sha256=new.content_sha256; end;
