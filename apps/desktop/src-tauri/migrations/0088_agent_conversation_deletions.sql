-- Minimal permanent deletion receipts: no title, transcript, credentials or source text.
-- A stale writer/create retry must not resurrect a deleted conversation identity.
create table agent_conversation_deletions (
    request_id text primary key not null,
    conversation_id text not null unique,
    request_hash text not null check(length(request_hash)=64 and request_hash not glob '*[^0-9a-f]*'),
    deleted_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
create trigger agent_conversation_no_resurrection
before insert on agent_conversations
when exists(select 1 from agent_conversation_deletions where conversation_id=new.id)
begin select raise(abort,'conversation was permanently deleted'); end;
create trigger agent_conversation_deletion_immutable_update
before update on agent_conversation_deletions
begin select raise(abort,'immutable conversation deletion receipt'); end;
create trigger agent_conversation_deletion_immutable_delete
before delete on agent_conversation_deletions
begin select raise(abort,'immutable conversation deletion receipt'); end;
