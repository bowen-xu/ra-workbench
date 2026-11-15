-- New execution copies carry file-delivery ownership independently of chat views.
-- Deliberately do not backfill old copies or frozen historical proposals.
create table writing_agent_file_deliveries (
    id integer primary key autoincrement,
    conversation_id text not null,
    request_id text not null,
    manuscript_id text not null references manuscripts(id) on delete cascade,
    settled integer not null default 0 check(settled in (0,1)),
    unique(conversation_id, request_id),
    foreign key(conversation_id, request_id)
        references agent_execution_workspaces(conversation_id, request_id) on delete cascade
);
create index writing_agent_file_deliveries_pending
    on writing_agent_file_deliveries(manuscript_id, settled, id);
create trigger writing_agent_file_delivery_prepared
after insert on agent_execution_workspaces
begin
    insert into writing_agent_file_deliveries(conversation_id, request_id, manuscript_id)
    values(new.conversation_id, new.request_id, new.manuscript_id);
end;
