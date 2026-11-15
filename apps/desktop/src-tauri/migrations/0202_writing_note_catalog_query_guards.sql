-- Version only the derived projection, independently of file versions and HBS.
alter table writing_note_catalog_state add column projection_version text not null default 'writing-note-text/v1';
-- Invalid timestamps remain intact and produce an explicit scoped error. This
-- guard must not scan the entire ready catalog before every metadata page.
create index writing_note_catalog_invalid_time on writing_note_catalog(status,id)
 where state='ready' and updated_ms is null;

-- Readiness is independent of page size: read at most six counters, rather than
-- counting every legacy/pending file again before each page query.
create table writing_note_catalog_counts (
 status text not null check(status in ('active','archived')),
 state text not null check(state in ('pending','ready','error')),
 count integer not null check(count>=0),primary key(status,state)
);
insert into writing_note_catalog_counts(status,state,count) values
 ('active','pending',0),('active','ready',0),('active','error',0),
 ('archived','pending',0),('archived','ready',0),('archived','error',0);
update writing_note_catalog_counts set count=(select count(*) from writing_note_catalog c
 where c.status=writing_note_catalog_counts.status and c.state=writing_note_catalog_counts.state);
create trigger writing_note_catalog_counts_insert after insert on writing_note_catalog begin
 update writing_note_catalog_counts set count=count+1 where status=new.status and state=new.state;
end;
create trigger writing_note_catalog_counts_delete after delete on writing_note_catalog begin
 update writing_note_catalog_counts set count=count-1 where status=old.status and state=old.state;
end;
create trigger writing_note_catalog_counts_update after update of status,state on writing_note_catalog
 when old.status is not new.status or old.state is not new.state
begin
 update writing_note_catalog_counts set count=count-1 where status=old.status and state=old.state;
 update writing_note_catalog_counts set count=count+1 where status=new.status and state=new.state;
end;
