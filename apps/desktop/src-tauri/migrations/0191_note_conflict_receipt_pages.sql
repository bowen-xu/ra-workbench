-- v184 ran both before and after page_id was added in place. Rebuild from
-- immutable request JSON, which is common to both deployed definitions.
-- Keep unresolvable historical receipts verbatim with a null page association;
-- new receipts must have a live page. Never discard replay evidence to upgrade.
create table note_conflict_resolutions_next (
 request_id text primary key,
 page_id text references note_pages(id) on delete cascade,
 request_json text not null check(json_valid(request_json)),
 result_json text not null check(json_valid(result_json))
);
insert into note_conflict_resolutions_next(request_id,page_id,request_json,result_json)
select r.request_id,p.id,r.request_json,r.result_json
from note_conflict_resolutions r
left join note_pages p on json_type(r.request_json,'$.document.pageId')='text'
 and p.id=json_extract(r.request_json,'$.document.pageId');
drop table note_conflict_resolutions;
alter table note_conflict_resolutions_next rename to note_conflict_resolutions;
create index note_conflict_resolutions_page_idx on note_conflict_resolutions(page_id);
create trigger note_conflict_resolution_requires_page_insert
before insert on note_conflict_resolutions when new.page_id is null
begin select raise(abort,'conflict receipt requires a live page'); end;
create trigger note_conflict_resolution_requires_page_update
before update of page_id on note_conflict_resolutions when new.page_id is null
begin select raise(abort,'conflict receipt requires a live page'); end;
