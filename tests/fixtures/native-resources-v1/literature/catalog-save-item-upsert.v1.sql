insert into bibliographic_items (
             id, item_type, title, abstract_text, issued_year, container_title,
             conference_name, proceedings_title, place, volume, issue,
             pages, url, language, bibtex_key, raw_bibtex, created_at, updated_at
         ) values (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
             strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         ) on conflict(id) do update set
             item_type = excluded.item_type, title = excluded.title,
             abstract_text = excluded.abstract_text, issued_year = excluded.issued_year,
             container_title = excluded.container_title,
             conference_name = excluded.conference_name,
             proceedings_title = excluded.proceedings_title,
             place = excluded.place, volume = excluded.volume,
             issue = excluded.issue, pages = excluded.pages, url = excluded.url,
             language = excluded.language, bibtex_key = excluded.bibtex_key,
             raw_bibtex = coalesce(excluded.raw_bibtex, bibliographic_items.raw_bibtex),
             updated_at = excluded.updated_at