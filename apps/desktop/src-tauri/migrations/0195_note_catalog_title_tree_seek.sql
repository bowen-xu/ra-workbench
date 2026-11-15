-- Keep the recursive row as the outer loop so each child lookup seeks the
-- existing (page_id,parent_block_id,position) index. Reordering the join to scan
-- every block for each parent made a single title refresh quadratic.
drop view note_page_catalog_values;
create view note_page_catalog_values as
with values_with_title as (
 select p.*, s.block_id as source_id, s.note_type, s.quote_exact, s.paper_title_snapshot,
    coalesce((
      with recursive blocks(id, path, text) as (
        select b.id, printf('%010d',b.position) || ':' || b.id, trim(b.plain_text,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))
        from note_blocks b where b.page_id=p.id and b.parent_block_id is null
        union all
        select b.id, t.path || '/' || printf('%010d',b.position) || ':' || b.id, trim(b.plain_text,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))
        from blocks t cross join note_blocks b on b.parent_block_id=t.id where b.page_id=p.id
      ) select trim(substr(text,1,instr(text || char(10),char(10))-1),' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))
        from blocks where text<>'' order by path limit 1
    ),'') as body_title
 from note_pages p left join reading_note_sources s on s.page_id=p.id
)
select p.id,
 case when source_id is null then title else substr(coalesce(nullif(body_title,''),
    nullif(trim(substr(ltrim(quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)),1,instr(ltrim(quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)) || char(10),char(10))-1),' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)),''),title),1,100) end as title,
 case when note_type='highlight' or (note_type='annotation' and trim(quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))<>'') then 'dependent' else 'independent' end as category,
 source_id,
 case when source_id is not null then paper_title_snapshot else coalesce((
   select group_concat(label,' · ') from (
     select json_extract(l.target_snapshot_json,'$.labelSnapshot') as label from note_links l
     where l.source_page_id=p.id and l.target_kind in ('paper','manuscript')
       and coalesce(json_extract(l.target_snapshot_json,'$.labelSnapshot'),'')<>'' order by l.created_at,l.id
   )
 ),'') end as source_label,
 updated_at,status,
 case when note_type='highlight' or (note_type='annotation' and trim(quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))<>'') then 0 else is_favorite end as is_favorite,
 coalesce((select position from note_favorite_order f where f.page_id=p.id),9223372036854775807) as favorite_position,revision
from values_with_title p;
