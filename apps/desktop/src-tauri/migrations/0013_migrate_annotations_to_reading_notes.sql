insert into note_blocks (
    id, scope_kind, page_id, parent_block_id, position, block_type,
    props_json, content_json, plain_text, schema_version, revision,
    status, is_favorite, created_at, updated_at, trashed_at
)
select
    a.id,
    'reading',
    null,
    null,
    0,
    case
        when a.annotation_type = 'highlight' then 'reading_highlight'
        when length(trim(s.quote_exact)) > 0 then 'reading_annotation'
        else 'reading_page_note'
    end,
    json_object('color', a.color),
    json_object('kind', 'inline', 'items', json_array(json_object('type', 'text', 'text', a.body, 'styles', json_array()))),
    a.body,
    1,
    0,
    'active',
    0,
    a.created_at,
    a.updated_at,
    null
from annotations a
join source_anchors s on s.id = a.anchor_id;

insert into reading_note_sources (
    block_id, current_anchor_id, paper_id_snapshot, paper_title_snapshot,
    attachment_id_snapshot, attachment_name_snapshot, content_sha256,
    page_index, page_label, rects_json, quote_exact, quote_prefix, quote_suffix,
    locator_version, created_at, updated_at
)
select
    a.id,
    s.id,
    s.item_id,
    coalesce(b.title, ''),
    s.attachment_id,
    coalesce(att.display_name, ''),
    s.content_sha256,
    s.page_index,
    s.page_label,
    s.rects_json,
    s.quote_exact,
    s.quote_prefix,
    s.quote_suffix,
    s.selector_version,
    a.created_at,
    a.updated_at
from annotations a
join source_anchors s on s.id = a.anchor_id
left join bibliographic_items b on b.id = s.item_id
left join attachments att on att.id = s.attachment_id;

create temporary table note_annotation_migration_assert (
    passed integer not null check(passed = 1)
);

insert into note_annotation_migration_assert(passed)
select case when
    (select count(*) from annotations) =
    (select count(*) from note_blocks where scope_kind = 'reading')
    and (select count(*) from annotations) =
    (select count(*) from reading_note_sources)
    and not exists(
        select 1 from annotations a
        left join note_blocks b on b.id = a.id
        where b.id is null
    )
then 1 else 0 end;

drop table note_annotation_migration_assert;
