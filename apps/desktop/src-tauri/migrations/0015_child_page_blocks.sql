with missing_children as (
    select
        child.id as child_id,
        child.title as child_title,
        child.parent_page_id as parent_id,
        child.status as child_status,
        row_number() over (
            partition by child.parent_page_id
            order by child.position, child.id
        ) as child_rank,
        coalesce((
            select max(existing.position)
            from note_blocks existing
            where existing.page_id = child.parent_page_id
              and existing.parent_block_id is null
        ), -1) as base_position
    from note_pages child
    where child.parent_page_id is not null
      and not exists (
          select 1
          from note_blocks existing
          where existing.block_type = 'childPage'
            and json_extract(existing.content_json, '$.targetId') = child.id
      )
)
insert into note_blocks (
    id, scope_kind, page_id, parent_block_id, position, block_type,
    props_json, content_json, plain_text, schema_version, revision,
    status, is_favorite, created_at, updated_at, trashed_at
)
select
    lower(hex(randomblob(16))),
    'page',
    parent_id,
    null,
    base_position + child_rank,
    'childPage',
    '{}',
    json_object(
        'kind', 'reference',
        'targetKind', 'note_page',
        'targetId', child_id,
        'labelSnapshot', child_title,
        'quoteSnapshot', ''
    ),
    child_title,
    1,
    0,
    child_status,
    0,
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    case when child_status = 'trashed' then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else null end
from missing_children;

create unique index note_child_page_active_target_idx
on note_blocks(json_extract(content_json, '$.targetId'))
where scope_kind = 'page' and block_type = 'childPage' and status = 'active';
