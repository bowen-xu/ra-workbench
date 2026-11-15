-- Instruction loading only; this does not authorize shell, network or disclosure.
create table agent_manuscript_instruction_trust (
    manuscript_id text primary key references manuscripts(id) on delete cascade,
    revision integer not null check (revision between 1 and 9007199254740991),
    granted integer not null check (granted in (0, 1)),
    canonical_path text not null,
    device text not null,
    inode text not null,
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
