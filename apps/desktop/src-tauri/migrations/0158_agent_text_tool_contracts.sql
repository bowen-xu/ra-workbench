-- Preserve the original tool contract of pre-upgrade submissions on recovery.
create table agent_text_tool_contracts (
 conversation_id text not null,
 request_id text not null,
 version integer not null check(version in (1,2)),
 primary key(conversation_id,request_id),
 foreign key(conversation_id,request_id) references agent_submissions(conversation_id,request_id) on delete cascade
);
insert into agent_text_tool_contracts(conversation_id,request_id,version)
select conversation_id,request_id,1 from agent_submissions;
