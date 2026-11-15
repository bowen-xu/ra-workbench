-- A read from another turn must never authorize an older model's patch.
-- Legacy unscoped reads cannot prove their owner; re-read before new edits.
create table agent_text_read_scopes (
 conversation_id text not null,
 scope_request_id text not null,
 path text not null,
 version text not null,
 snapshot_json text not null check(json_valid(snapshot_json)),
 primary key(conversation_id,scope_request_id,path),
 foreign key(conversation_id,scope_request_id) references agent_submissions(conversation_id,request_id) on delete cascade
);
