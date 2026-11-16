with writing_keys as materialized (
             select cast(json_extract(value, '$.literature_item_id') as text) as literature_item_id,
                    json_extract(value, '$.bound_key') as bound_key,
                    json_extract(value, '$.first_imported_key') as first_imported_key,
                    json_extract(value, '$.imported_keys_search_text') as imported_keys_search_text
             from json_each(?1)
         )
         select b.id, b.title, b.issued_year, b.container_title, b.bibtex_key,
                coalesce((
                    select nullif(c.literal_name, '')
                    from item_creators ic join creators c on c.id = ic.creator_id
                    where ic.item_id = b.id and ic.role = 'author'
                    order by ic.position limit 1
                ), (
                    select c.family_name
                    from item_creators ic join creators c on c.id = ic.creator_id
                    where ic.item_id = b.id and ic.role = 'author'
                    order by ic.position limit 1
                ), ''),
                exists(select 1 from attachments a where a.item_id = b.id),
                exists(
                    select 1 from evidence_links el
                    join source_anchors sa on sa.id = el.anchor_id
                    where sa.item_id = b.id
                ),
                coalesce(keys.bound_key, keys.first_imported_key)
         from bibliographic_items b
         left join writing_keys keys on keys.literature_item_id = b.id
         where b.archived_at is null and b.trashed_at is null
           and (?2 = 0 or exists(
               select 1 from project_items pi where pi.project_id = ?3 and pi.item_id = b.id
           ))
           and (?4 = '%%' or lower(
               b.title || ' ' || coalesce(b.issued_year, '') || ' ' || b.container_title || ' '
               || b.conference_name || ' ' || b.proceedings_title || ' ' || b.place || ' '
               || b.bibtex_key || ' ' || coalesce(keys.bound_key, '') || ' '
               || coalesce(keys.imported_keys_search_text, '') || ' ' || coalesce((
                   select group_concat(c.family_name || ' ' || c.given_name || ' ' || c.literal_name, ' ')
                   from item_creators ic join creators c on c.id = ic.creator_id
                   where ic.item_id = b.id
               ), '')
           ) like ?4)
         order by case when keys.bound_key is null then 1 else 0 end,
                  b.updated_at desc, b.id
         limit 60
