-- Rebuildable writing-note projections. No canonical file, note/HBS save
-- revision or audit record is rewritten. Existing files start pending; this
-- metadata migration performs no file/body I/O.
create table writing_note_catalog (
 manuscript_id text not null,relative_path text not null,
 note_id text,
 id text generated always as (case when note_id is not null then 'writing:' || json_array(manuscript_id,note_id) end) stored,
 source_hash text not null,title text not null,source_label text not null,
 status text not null check(status in ('active','archived')),
 updated_at text not null,updated_ms integer,
 state text not null default 'pending' check(state in ('pending','ready','error')),
 error_cause text,
 title_key blob not null,title_down blob not null,
 source_key blob not null,source_down blob not null,
 metadata_text text not null,
 primary key(manuscript_id,relative_path),unique(manuscript_id,note_id),
 foreign key(manuscript_id,relative_path) references manuscript_files(manuscript_id,relative_path) on update cascade on delete cascade
);
create table writing_note_catalog_state (id integer primary key check(id=1),generation integer not null);
insert into writing_note_catalog_state(id,generation) values(1,0);
create view writing_note_catalog_file_values as
 select f.manuscript_id,f.relative_path,f.sha256 as source_hash,
  ra_writing_note_title_v1(f.relative_path) as title,m.title as source_label,m.status,
  f.modified_at as updated_at,ra_note_utc_millis_v1(f.modified_at) as updated_ms,
  ra_note_sort_key_v1(ra_writing_note_title_v1(f.relative_path)) as title_key,
  ra_note_sort_key_v1(m.title) as source_key,
  ra_note_fold_v1(ra_writing_note_title_v1(f.relative_path) || ' ' || m.title) as metadata_text
 from manuscript_files f join manuscripts m on m.id=f.manuscript_id
 where lower(substr(f.relative_path,-7))='.ranote';
insert into writing_note_catalog(manuscript_id,relative_path,source_hash,title,source_label,status,updated_at,updated_ms,title_key,title_down,source_key,source_down,metadata_text)
 select manuscript_id,relative_path,source_hash,title,source_label,status,updated_at,updated_ms,title_key,ra_note_reverse_key_v1(title_key),source_key,ra_note_reverse_key_v1(source_key),metadata_text from writing_note_catalog_file_values;
create table writing_note_catalog_body (
 manuscript_id text not null,relative_path text not null,search_text text not null,folded_text text not null,
 primary key(manuscript_id,relative_path),
 foreign key(manuscript_id,relative_path) references writing_note_catalog(manuscript_id,relative_path) on update cascade on delete cascade
);
create index writing_note_catalog_pending on writing_note_catalog(state,status,manuscript_id,relative_path);
create index writing_note_catalog_id on writing_note_catalog(id) where id is not null;
create index writing_note_catalog_title_up on writing_note_catalog(status,title_key,id) where state='ready';
create index writing_note_catalog_title_down on writing_note_catalog(status,title_down,id) where state='ready';
create index writing_note_catalog_source_up on writing_note_catalog(status,source_key,id) where state='ready';
create index writing_note_catalog_source_down on writing_note_catalog(status,source_down,id) where state='ready';
create index writing_note_catalog_category on writing_note_catalog(status,id) where state='ready';
create index writing_note_catalog_time_up on writing_note_catalog(status,updated_ms,id) where state='ready';
create index writing_note_catalog_time_down on writing_note_catalog(status,updated_ms desc,id) where state='ready';
create virtual table writing_note_catalog_metadata_search using fts5(metadata_text,content='writing_note_catalog',content_rowid='rowid',tokenize='trigram');
insert into writing_note_catalog_metadata_search(writing_note_catalog_metadata_search)values('rebuild');
create virtual table writing_note_catalog_body_search using fts5(folded_text,content='writing_note_catalog_body',content_rowid='rowid',tokenize='trigram');
create trigger writing_note_catalog_search_insert after insert on writing_note_catalog begin
 insert into writing_note_catalog_metadata_search(rowid,metadata_text)values(new.rowid,new.metadata_text); end;
create trigger writing_note_catalog_search_delete after delete on writing_note_catalog begin
 insert into writing_note_catalog_metadata_search(writing_note_catalog_metadata_search,rowid,metadata_text)values('delete',old.rowid,old.metadata_text); end;
create trigger writing_note_catalog_search_update after update of metadata_text on writing_note_catalog when old.metadata_text is not new.metadata_text begin
 insert into writing_note_catalog_metadata_search(writing_note_catalog_metadata_search,rowid,metadata_text)values('delete',old.rowid,old.metadata_text);
 insert into writing_note_catalog_metadata_search(rowid,metadata_text)values(new.rowid,new.metadata_text); end;
create trigger writing_note_catalog_generation_insert after insert on writing_note_catalog begin
 update writing_note_catalog_state set generation=generation+1 where id=1; end;
create trigger writing_note_catalog_generation_update after update on writing_note_catalog begin
 update writing_note_catalog_state set generation=generation+1 where id=1; end;
create trigger writing_note_catalog_generation_delete after delete on writing_note_catalog begin
 update writing_note_catalog_state set generation=generation+1 where id=1; end;
create trigger writing_note_catalog_body_search_insert after insert on writing_note_catalog_body begin
 insert into writing_note_catalog_body_search(rowid,folded_text)values(new.rowid,new.folded_text); end;
create trigger writing_note_catalog_body_search_delete after delete on writing_note_catalog_body begin
 insert into writing_note_catalog_body_search(writing_note_catalog_body_search,rowid,folded_text)values('delete',old.rowid,old.folded_text); end;
create trigger writing_note_catalog_body_search_update after update of folded_text on writing_note_catalog_body when old.folded_text is not new.folded_text begin
 insert into writing_note_catalog_body_search(writing_note_catalog_body_search,rowid,folded_text)values('delete',old.rowid,old.folded_text);
 insert into writing_note_catalog_body_search(rowid,folded_text)values(new.rowid,new.folded_text); end;
create trigger writing_note_catalog_body_generation_insert after insert on writing_note_catalog_body begin
 update writing_note_catalog_state set generation=generation+1 where id=1; end;
create trigger writing_note_catalog_body_generation_update after update on writing_note_catalog_body begin
 update writing_note_catalog_state set generation=generation+1 where id=1; end;
create trigger writing_note_catalog_body_generation_delete after delete on writing_note_catalog_body begin
 update writing_note_catalog_state set generation=generation+1 where id=1; end;
create trigger writing_note_catalog_file_insert after insert on manuscript_files
 when lower(substr(new.relative_path,-7))='.ranote'
begin
 insert into writing_note_catalog(manuscript_id,relative_path,source_hash,title,source_label,status,updated_at,updated_ms,title_key,title_down,source_key,source_down,metadata_text)
 select manuscript_id,relative_path,source_hash,title,source_label,status,updated_at,updated_ms,title_key,ra_note_reverse_key_v1(title_key),source_key,ra_note_reverse_key_v1(source_key),metadata_text from writing_note_catalog_file_values where manuscript_id=new.manuscript_id and relative_path=new.relative_path;
end;
-- Hash changes retire old search text in the original file-index transaction.
create trigger writing_note_catalog_file_update after update of sha256,modified_at,relative_path on manuscript_files
begin
 delete from writing_note_catalog where manuscript_id=new.manuscript_id and relative_path=new.relative_path and lower(substr(new.relative_path,-7))<>'.ranote';
 insert into writing_note_catalog(manuscript_id,relative_path,source_hash,title,source_label,status,updated_at,updated_ms,title_key,title_down,source_key,source_down,metadata_text)
 select manuscript_id,relative_path,source_hash,title,source_label,status,updated_at,updated_ms,title_key,ra_note_reverse_key_v1(title_key),source_key,ra_note_reverse_key_v1(source_key),metadata_text from writing_note_catalog_file_values where manuscript_id=new.manuscript_id and relative_path=new.relative_path and 1
 on conflict(manuscript_id,relative_path) do update set
  note_id=case when writing_note_catalog.source_hash<>excluded.source_hash then null else writing_note_catalog.note_id end,
  state=case when writing_note_catalog.source_hash<>excluded.source_hash then 'pending' else writing_note_catalog.state end,
  error_cause=case when writing_note_catalog.source_hash<>excluded.source_hash then null else writing_note_catalog.error_cause end,
  source_hash=excluded.source_hash,title=excluded.title,title_key=excluded.title_key,title_down=excluded.title_down,
  updated_at=excluded.updated_at,updated_ms=excluded.updated_ms,metadata_text=excluded.metadata_text
 where writing_note_catalog.source_hash is not excluded.source_hash or writing_note_catalog.title is not excluded.title or writing_note_catalog.updated_at is not excluded.updated_at;
 delete from writing_note_catalog_body where manuscript_id=new.manuscript_id and relative_path=new.relative_path
  and exists(select 1 from writing_note_catalog c where c.manuscript_id=new.manuscript_id and c.relative_path=new.relative_path and c.state='pending');
end;
create trigger writing_note_catalog_manuscript_update after update of title,status on manuscripts
 when old.title is not new.title or old.status is not new.status
begin
 update writing_note_catalog set source_label=new.title,status=new.status,
  source_key=ra_note_sort_key_v1(new.title),source_down=ra_note_reverse_key_v1(ra_note_sort_key_v1(new.title)),
  metadata_text=ra_note_fold_v1(title || ' ' || new.title) where manuscript_id=new.id;
end;
