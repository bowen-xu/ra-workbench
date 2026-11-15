-- Undo is a new atomic note save. Original application receipts remain immutable.
create table agent_note_undos (
    undo_id text primary key,
    application_id text not null unique references agent_note_applications(application_id) on delete cascade,
    before_revision integer not null check(before_revision>=1 and before_revision<9007199254740991),
    restored_revision integer not null check(restored_revision=before_revision+1),
    before_hash text not null check(length(before_hash)=64),
    restored_hash text not null check(length(restored_hash)=64),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
create trigger agent_note_undo_immutable before update on agent_note_undos
begin select raise(abort,'immutable note undo'); end;
create trigger agent_note_undo_matches_application before insert on agent_note_undos
when not exists(select 1 from agent_note_applications a where a.application_id=new.application_id
    and a.applied_revision=new.before_revision and a.after_hash=new.before_hash
    and a.before_hash=new.restored_hash)
begin select raise(abort,'note undo baseline mismatch'); end;
