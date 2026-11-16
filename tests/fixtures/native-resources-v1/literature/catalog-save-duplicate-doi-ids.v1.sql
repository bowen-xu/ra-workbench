select other.item_id from identifiers current
             join identifiers other on other.kind = current.kind
                and other.normalized_value = current.normalized_value
                and other.item_id <> current.item_id
             where current.item_id = ?1 and current.kind = 'doi'