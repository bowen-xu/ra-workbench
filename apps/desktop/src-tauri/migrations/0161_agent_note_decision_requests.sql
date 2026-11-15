-- Serial ownership and confirmed failures for user decisions, distinct from tool calls.
create table agent_note_decision_requests(
 request_id text primary key,
 conversation_id text not null references agent_conversations(id) on delete cascade,
 input_sha256 text not null,
 owner_boot text not null,
 owner_pid integer not null,
 state text not null check(state in('running','failed','complete')),
 error text
);
