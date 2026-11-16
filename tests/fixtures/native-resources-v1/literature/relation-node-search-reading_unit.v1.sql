select id, coalesce(nullif(title, ''),
                case when length(body) > 240
                     then substr(body, 1, 239) || '…' else body end), unit_kind
         from structured_reading_units
         where ?1 = '' or instr(lower(title || ' ' || body), lower(?1)) > 0
         order by updated_at desc limit 15
