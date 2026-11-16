select distinct b.id from bibliographic_items b join identifiers i on i.item_id=b.id
            where i.kind='doi' and i.normalized_value=?1 and b.archived_at is null and b.trashed_at is null
