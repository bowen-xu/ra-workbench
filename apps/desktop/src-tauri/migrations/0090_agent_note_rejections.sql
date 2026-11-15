-- An explicit user rejection is distinct from the immutable model tool result.
-- It changes no note/source bytes and is removed with an explicitly deleted chat.
create table agent_note_rejections (
    decision_id text primary key,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    page_id text not null,
    turn_id text not null,
    item_id text not null,
    proposal_id text not null check(length(proposal_id)=77 and substr(proposal_id,1,13)='note-rewrite:'),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    unique(conversation_id, item_id)
);
create trigger agent_note_rejection_immutable
before update on agent_note_rejections
begin
    select raise(abort, 'immutable note rejection');
end;
