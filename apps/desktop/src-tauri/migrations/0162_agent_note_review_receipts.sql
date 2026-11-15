-- Rich review D/B/history is user-only; model tools continue to expose text views.
-- Each receipt commits with its candidate mutations and any final formal save.
create table agent_note_review_receipts(
 request_id text primary key,
 conversation_id text not null references agent_conversations(id) on delete cascade,
 proposal_id text not null,
 input_sha256 text not null,
 result_json text not null check(json_valid(result_json)),
 before_json text not null check(json_valid(before_json)),
 after_json text not null check(json_valid(after_json)),
 formal_json text not null check(json_valid(formal_json))
);
create index agent_note_review_receipts_proposal on agent_note_review_receipts(conversation_id,proposal_id);
