update literature_search set full_text = coalesce((
             select group_concat(page_text, ' ') from attachment_page_texts p
             join attachments a on a.id = p.attachment_id where a.item_id = ?1
         ), '') where item_id = ?1