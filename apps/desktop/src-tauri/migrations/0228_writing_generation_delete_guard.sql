-- Retain live preparation identities while allowing manuscript deletion to
-- cascade through the original delivery proof and its derived generations.
drop trigger writing_agent_workspace_generation_no_delete;
create trigger writing_agent_workspace_generation_no_delete
before delete on writing_agent_workspace_generations
when exists(select 1 from agent_submissions where conversation_id=old.conversation_id and request_id=old.request_id)
 and exists(select 1 from manuscripts where id=old.manuscript_id)
begin select raise(abort,'workspace generation cannot be deleted'); end;
