-- Conversation cards read frozen tool generations, never the live review baseline.
create table agent_note_patch_snapshots(
 conversation_id text not null references agent_conversations(id) on delete cascade,
 proposal_id text not null,
 generation integer not null check(generation>=1),
 notes_json text not null check(json_valid(notes_json)),
 primary key(conversation_id,proposal_id,generation)
);
-- Older retained proposals have only their latest candidate; preserve that exact
-- snapshot without inferring content for generations whose bytes no longer exist.
insert into agent_note_patch_snapshots(conversation_id,proposal_id,generation,notes_json)
select conversation_id,proposal_id,generation,json_group_array(json(candidate_json))
from agent_note_patch_candidates group by conversation_id,proposal_id,generation;
