select
                (select count(*) from attachments where item_id = ?1),
                (select count(*) from (
                    select block_id as note_id from reading_note_sources where paper_id_snapshot = ?1
                    union
                    select legacy.id from annotations legacy
                    join source_anchors source on source.id = legacy.anchor_id
                    where source.item_id = ?1 and not exists(
                        select 1 from reading_note_sources current
                        where current.current_anchor_id = source.id
                    )
                )),
                (select count(*) from evidence_links e
                    join source_anchors s on s.id = e.anchor_id where s.item_id = ?1),
                (select count(*) from project_items where item_id = ?1),
                (select count(*) from merge_operations
                    where survivor_item_id = ?1 or merged_item_id = ?1),
                (select count(distinct a.content_sha256) from attachments a
                    where a.item_id = ?1 and (
                        exists(select 1 from attachments other
                            where other.content_sha256 = a.content_sha256
                              and other.item_id is not ?1)
                        or exists(select 1 from source_anchors other_anchor
                            where other_anchor.content_sha256 = a.content_sha256
                              and other_anchor.item_id <> ?1)
                        or exists(select 1 from note_block_assets note_asset
                            where note_asset.content_sha256 = a.content_sha256)
                    ))