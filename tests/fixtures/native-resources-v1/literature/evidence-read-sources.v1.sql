select s.id, coalesce(note.id, legacy.id), s.item_id, b.title, s.attachment_id, attachment.display_name,
            s.content_sha256, s.page_index, s.page_label, s.rects_json, s.quote_exact,
            s.quote_prefix, s.quote_suffix,
            coalesce(case note.block_type when 'reading_highlight' then 'highlight' else 'note' end, legacy.annotation_type),
            coalesce(note.plain_text, legacy.body, ''), s.created_at
     from source_anchors s
     join bibliographic_items b on b.id = s.item_id
     join attachments attachment on attachment.id = s.attachment_id
     left join reading_note_sources note_source on note_source.current_anchor_id = s.id
     left join note_blocks note on note.id = note_source.block_id and note.status = 'active'
     left join annotations legacy on legacy.anchor_id = s.id
         where b.archived_at is null and (?1 is null or exists(
             select 1 from project_items pi where pi.item_id = s.item_id and pi.project_id = ?1
         )) order by s.created_at desc, s.id desc
