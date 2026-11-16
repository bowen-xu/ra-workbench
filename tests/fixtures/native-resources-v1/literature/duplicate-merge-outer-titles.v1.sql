select survivor.title, merged.title from bibliographic_items survivor
         join bibliographic_items merged on merged.id = ?2 where survivor.id = ?1