insert into literature_search (item_id, title, creators, identifiers, tags, full_text)
         select b.id, b.title,
                coalesce((select group_concat(c.family_name || ' ' || c.given_name, ' ') from item_creators ic join creators c on c.id = ic.creator_id where ic.item_id = b.id), ''),
                coalesce((select group_concat(display_value, ' ') from identifiers where item_id = b.id), ''),
                coalesce((select group_concat(t.name, ' ') from item_tags it join tags t on t.id = it.tag_id where it.item_id = b.id), ''),
                coalesce((select group_concat(p.page_text, ' ') from attachment_page_texts p
                    join attachments a on a.id = p.attachment_id where a.item_id = b.id), '')
         from bibliographic_items b where b.id = ?1