-- Keep evidence of a pending legacy import if its paper is deleted. The
-- conversation owns this guard; deleting the paper must not erase it.
create table agent_legacy_import_recovery_guards (
    conversation_id text primary key not null references agent_conversations(id) on delete cascade
);

insert into agent_legacy_import_recovery_guards(conversation_id)
select conversation_id from agent_paper_reading_history_migrations where status='pending';

create trigger agent_legacy_import_guard_insert
after insert on agent_paper_reading_history_migrations
when new.status='pending'
begin
    insert into agent_legacy_import_recovery_guards(conversation_id) values(new.conversation_id)
    on conflict(conversation_id) do nothing;
end;

create trigger agent_legacy_import_guard_pending_update
after update of status, conversation_id on agent_paper_reading_history_migrations
when new.status='pending'
begin
    insert into agent_legacy_import_recovery_guards(conversation_id) values(new.conversation_id)
    on conflict(conversation_id) do nothing;
end;

-- A successful completion for the same conversation releases the guard in
-- the same transaction. Deleting a pending mapping deliberately does not.
create trigger agent_legacy_import_guard_complete
after update of status, conversation_id on agent_paper_reading_history_migrations
when old.status='pending' and new.status='complete' and old.conversation_id=new.conversation_id
begin
    delete from agent_legacy_import_recovery_guards where conversation_id=old.conversation_id;
end;
