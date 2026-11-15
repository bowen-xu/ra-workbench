-- Source provenance is a bidirectional reference, never lifecycle ownership.
insert into note_links
    (id, source_page_id, source_block_id, target_kind, target_id,
     relation_type, target_snapshot_json, created_at)
select 'reading-origin:' || s.block_id, s.page_id, null, 'paper', s.paper_id_snapshot,
       'derived_from', json_object('labelSnapshot', s.paper_title_snapshot,
                                   'quoteSnapshot', s.quote_exact), s.created_at
from reading_note_sources s
where not exists (
    select 1 from note_links l where l.source_page_id = s.page_id
    and l.source_block_id is null and l.target_kind = 'paper'
    and l.target_id = s.paper_id_snapshot and l.relation_type = 'derived_from'
);

create trigger reading_note_origin_link_insert after insert on reading_note_sources
begin
    insert into note_links
        (id, source_page_id, source_block_id, target_kind, target_id,
         relation_type, target_snapshot_json, created_at)
    select 'reading-origin:' || new.block_id, new.page_id, null, 'paper', new.paper_id_snapshot,
           'derived_from', json_object('labelSnapshot', new.paper_title_snapshot,
                                       'quoteSnapshot', new.quote_exact), new.created_at
    where not exists (
        select 1 from note_links l where l.source_page_id = new.page_id
        and l.source_block_id is null and l.target_kind = 'paper'
        and l.target_id = new.paper_id_snapshot and l.relation_type = 'derived_from'
    );
end;
