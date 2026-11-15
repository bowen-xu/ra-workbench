create table note_favorite_order (
    page_id text primary key references note_pages(id) on delete cascade,
    position integer not null check(position >= 0)
);

insert into note_favorite_order(page_id, position)
select p.id, row_number() over (order by p.updated_at desc, p.id) - 1
from note_pages p where p.is_favorite = 1
and not exists(select 1 from reading_note_sources s where s.page_id = p.id
    and (s.note_type = 'highlight' or (s.note_type = 'annotation' and trim(s.quote_exact) <> '')));

create trigger note_favorite_added after update of is_favorite on note_pages
when new.is_favorite = 1 and old.is_favorite = 0
begin
    update note_favorite_order set position = position + 1;
    insert into note_favorite_order(page_id, position) values(new.id, 0);
end;

create trigger note_favorite_removed after update of is_favorite on note_pages
when new.is_favorite = 0
begin
    delete from note_favorite_order where page_id = new.id;
end;

create trigger note_favorite_created after insert on note_pages
when new.is_favorite = 1
begin
    update note_favorite_order set position = position + 1;
    insert into note_favorite_order(page_id, position) values(new.id, 0);
end;
