select b.id, b.item_type, b.title, b.abstract_text, b.issued_year,
                    b.container_title, b.conference_name, b.proceedings_title, b.place,
                    b.volume, b.issue, b.pages,
                    coalesce((select display_value from identifiers where item_id = b.id and kind = 'doi' limit 1), ''),
                    b.url, b.language, b.bibtex_key, b.raw_bibtex, b.reading_status,
                    b.created_at, b.updated_at, b.trashed_at
             from bibliographic_items b
             where b.id = ?1
               and (?2 or (b.archived_at is null and b.trashed_at is null))