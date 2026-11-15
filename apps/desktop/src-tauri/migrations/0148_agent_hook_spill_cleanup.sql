-- Incremental repair for both exact deployed v147 definitions.
-- Preserve all receipts, previews, grants and recorded migration hashes.
drop trigger if exists agent_hook_spill_origin_delete;
drop trigger if exists agent_hook_spill_grant_delete;
-- A fork keeps its own grant after origin deletion. Drop orphan evidence only
-- when both the origin and every granted descendant have been removed.
create trigger agent_hook_spill_origin_delete after delete on agent_conversations
begin
 delete from agent_hook_spill_artifacts where source_conversation_id=old.id
  and not exists(select 1 from agent_hook_spill_grants g where g.artifact_id=agent_hook_spill_artifacts.id);
end;
create trigger agent_hook_spill_grant_delete after delete on agent_hook_spill_grants
begin
 delete from agent_hook_spill_artifacts where id=old.artifact_id
  and not exists(select 1 from agent_conversations c where c.id=source_conversation_id)
  and not exists(select 1 from agent_hook_spill_grants g where g.artifact_id=old.artifact_id);
end;
