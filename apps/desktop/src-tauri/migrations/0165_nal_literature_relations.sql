-- NAL statements replace the active collection UI. Old collection tables are
-- retained as migration/merge history; import is transactional and one-time.
create table nal_state (
    id integer primary key check(id = 1),
    revision integer not null default 0,
    legacy_imported integer not null default 0 check(legacy_imported in (0,1))
);
insert into nal_state (id) values (1);
create table nal_statements (
    id text primary key,
    subject text not null check(json_valid(subject)),
    predicate text not null check(json_valid(predicate)),
    origin text not null check(origin in ('human','legacy')),
    unique(subject,predicate)
);
create table nal_annotations (
    id text primary key,
    concept_id text references concepts(id) on delete cascade,
    statement_id text references nal_statements(id) on delete cascade,
    note_page_id text not null references note_pages(id) on delete cascade,
    check((concept_id is null) <> (statement_id is null))
);
create unique index nal_annotation_concept on nal_annotations(concept_id,note_page_id) where concept_id is not null;
create unique index nal_annotation_statement on nal_annotations(statement_id,note_page_id) where statement_id is not null;
create trigger nal_concept_created after insert on concepts begin
    update nal_state set revision = revision + 1 where id = 1;
end;
create trigger nal_concept_updated after update on concepts begin
    update nal_state set revision = revision + 1 where id = 1;
end;
create trigger nal_concept_deleted before delete on concepts begin
    delete from nal_statements where exists (
        select 1 from json_tree(subject) j where j.key = 'id' and j.value = old.id
        and json_extract(subject, substr(j.fullkey,1,length(j.fullkey)-3) || '.kind') = 'concept'
    ) or exists (
        select 1 from json_tree(predicate) j where j.key = 'id' and j.value = old.id
        and json_extract(predicate, substr(j.fullkey,1,length(j.fullkey)-3) || '.kind') = 'concept'
    );
    update nal_state set revision = revision + 1 where id = 1;
end;
create trigger nal_paper_deleted before delete on bibliographic_items begin
    delete from nal_statements where exists (
        select 1 from json_tree(subject) j where j.key = 'id' and j.value = old.id
        and json_extract(subject, substr(j.fullkey,1,length(j.fullkey)-3) || '.kind') = 'paper'
    ) or exists (
        select 1 from json_tree(predicate) j where j.key = 'id' and j.value = old.id
        and json_extract(predicate, substr(j.fullkey,1,length(j.fullkey)-3) || '.kind') = 'paper'
    );
    update nal_state set revision = revision + 1 where id = 1;
end;
create trigger nal_note_deleted after delete on note_pages begin
    update nal_state set revision = revision + 1 where id = 1;
end;
