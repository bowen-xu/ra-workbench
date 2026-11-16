select id, item_type, title, abstract_text, issued_year, container_title,
                     conference_name, proceedings_title, place, volume, issue, pages,
                     url, language, bibtex_key, raw_bibtex,
                     reading_status, archived_at, trashed_at, created_at, updated_at
              from bibliographic_items order by id
