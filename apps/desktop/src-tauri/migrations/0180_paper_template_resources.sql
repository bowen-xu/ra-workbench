-- Paper-specific templates are independent of reusable global templates.
create table structured_paper_template_owners (
    template_id text primary key references structured_resources(id) on delete cascade,
    paper_id text not null unique references bibliographic_items(id) on delete cascade
);
create temporary table paper_template_resource_seed (paper_id text primary key,id text not null);
insert into paper_template_resource_seed(paper_id,id)
select id,lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(6))) from bibliographic_items;
insert into structured_resources(id,kind,title) select id,'readingTemplate','本文阅读模板' from paper_template_resource_seed;
insert into structured_paper_template_owners(template_id,paper_id) select id,paper_id from paper_template_resource_seed;
drop table paper_template_resource_seed;
create trigger bibliographic_paper_template_created after insert on bibliographic_items begin
    insert into structured_resources(id,kind,title) values(lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(6))),'readingTemplate','本文阅读模板');
    insert into structured_paper_template_owners(template_id,paper_id) select id,new.id from structured_resources where rowid=last_insert_rowid();
end;
create table structured_template_draft_adoptions (
    draft_id text primary key references structured_resource_drafts(id) on delete cascade,
    reading_resource_id text not null references structured_resources(id) on delete cascade,
    expected_reading_revision integer not null check(expected_reading_revision>=0)
);
