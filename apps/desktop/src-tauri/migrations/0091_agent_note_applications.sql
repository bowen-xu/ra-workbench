-- Atomic note acceptance receipts contain identities, revisions and hashes only.
-- Original edits remain in the immutable tool journal; no duplicate asset owner.
create table agent_note_applications (
    application_id text primary key,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    page_id text not null references note_pages(id) on delete cascade,
    turn_id text not null,
    item_id text not null,
    proposal_id text not null check(length(proposal_id)=77 and substr(proposal_id,1,13)='note-rewrite:'),
    before_revision integer not null check(before_revision>=0 and before_revision<9007199254740991),
    applied_revision integer not null check(applied_revision=before_revision+1),
    before_hash text not null check(length(before_hash)=64),
    after_hash text not null check(length(after_hash)=64),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    unique(conversation_id,item_id)
);
create trigger agent_note_application_immutable before update on agent_note_applications
begin select raise(abort,'immutable note application'); end;
create trigger agent_note_application_not_rejected before insert on agent_note_applications
when exists(select 1 from agent_note_rejections where conversation_id=new.conversation_id and item_id=new.item_id)
begin select raise(abort,'note proposal already rejected'); end;
create trigger agent_note_rejection_not_applied before insert on agent_note_rejections
when exists(select 1 from agent_note_applications where conversation_id=new.conversation_id and item_id=new.item_id)
begin select raise(abort,'note proposal already applied'); end;
