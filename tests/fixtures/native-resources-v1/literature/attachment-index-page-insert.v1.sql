insert into attachment_page_texts (attachment_id, page_index, page_text, indexed_at)
             values (?1, max(?2, 0), ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))