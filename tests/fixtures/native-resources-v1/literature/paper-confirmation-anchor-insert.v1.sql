insert into source_anchors
                (id,item_id,attachment_id,content_sha256,selector_version,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,created_at)
                values(?1,?2,?3,?4,1,?5,?6,'[]',?7,?8,?9,strftime('%Y-%m-%dT%H:%M:%fZ','now'))
