-- Conversation-owned proposals/read caches must not survive conversation deletion.
delete from agent_note_patch_candidates where conversation_id not in(select id from agent_conversations);
delete from agent_note_patch_decisions where conversation_id not in(select id from agent_conversations);
delete from agent_text_patch_calls where conversation_id not in(select id from agent_conversations);
delete from agent_text_reads where conversation_id not in(select id from agent_conversations);
create trigger agent_text_patch_conversation_cleanup after delete on agent_conversations
begin
 delete from agent_note_patch_candidates where conversation_id=old.id;
 delete from agent_note_patch_decisions where conversation_id=old.id;
 delete from agent_text_patch_calls where conversation_id=old.id;
 delete from agent_text_reads where conversation_id=old.id;
 delete from agent_text_read_scopes where conversation_id=old.id;
end;
