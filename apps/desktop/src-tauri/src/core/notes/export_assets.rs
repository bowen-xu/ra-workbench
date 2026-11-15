//! Asset access for note export. The reader retains the Core lock and database
//! ownership; consumers receive only the existing bytes and recorded MIME type.
use crate::storage::AppStore;

pub(crate) struct NoteExportAsset {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

pub(crate) trait NoteExportAssets {
    fn read_export_asset(&self, id: &str) -> Result<NoteExportAsset, String>;
}

/// Application-managed consumer handle over the existing Core store. Cloning
/// this handle retains that same owner without exposing its database or paths.
#[derive(Clone)]
pub(crate) struct NoteExportAssetReader(AppStore);

impl NoteExportAssetReader {
    pub(crate) fn for_store(store: &AppStore) -> Self {
        Self(store.clone())
    }
}

impl NoteExportAssets for NoteExportAssetReader {
    fn read_export_asset(&self, id: &str) -> Result<NoteExportAsset, String> {
        self.0.read_export_asset(id)
    }
}

impl NoteExportAssets for AppStore {
    fn read_export_asset(&self, id: &str) -> Result<NoteExportAsset, String> {
        let _read = self
            .content_read()
            .map_err(|error| format!("NOTE_EXPORT_ASSET_READ：{error}"))?;
        let connection = self
            .connection()
            .map_err(|error| format!("NOTE_EXPORT_ASSET_READ：{error}"))?;
        let bytes = super::assets::read_note_asset_at(&connection, self.content_directory(), id)
            .map_err(|error| format!("NOTE_EXPORT_ASSET_READ：{error}"))?;
        let mime_type = connection
            .query_row(
                "select mime_type from content_objects where sha256=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|error| format!("NOTE_EXPORT_ASSET_READ：{error}"))?;
        Ok(NoteExportAsset { bytes, mime_type })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, AppStore, String) {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("objects");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("image.png"), b"existing asset bytes").unwrap();
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "create table content_objects (sha256 text primary key, relative_path text not null, mime_type text not null);",
        ).unwrap();
        let id = "a".repeat(64);
        connection.execute(
            "insert into content_objects (sha256, relative_path, mime_type) values (?1, 'image.png', 'image/png')",
            [&id],
        ).unwrap();
        (root, AppStore::for_test(connection, directory), id)
    }

    #[test]
    fn reads_the_existing_object_and_mime_without_changing_metadata() {
        let (_root, store, id) = fixture();
        let before = store.connection().unwrap().total_changes();
        let asset = store.read_export_asset(&id).unwrap();
        assert_eq!(asset.bytes, b"existing asset bytes");
        assert_eq!(asset.mime_type, "image/png");
        assert_eq!(store.connection().unwrap().total_changes(), before);
    }

    #[test]
    fn invalid_and_missing_assets_preserve_export_diagnostics() {
        let (_root, store, _id) = fixture();
        let invalid = store.read_export_asset("../image.png").err().unwrap();
        assert_eq!(
            invalid,
            "NOTE_EXPORT_ASSET_READ：笔记输入无效：附件 ID 无效"
        );
        let missing = store.read_export_asset(&"b".repeat(64)).err().unwrap();
        assert_eq!(missing, "NOTE_EXPORT_ASSET_READ：找不到笔记对象");
    }

    #[test]
    fn rejects_objects_outside_the_content_store() {
        let (root, store, id) = fixture();
        std::fs::write(root.path().join("outside.png"), b"private").unwrap();
        store
            .connection()
            .unwrap()
            .execute(
                "update content_objects set relative_path='../outside.png' where sha256=?1",
                [&id],
            )
            .unwrap();
        assert_eq!(
            store.read_export_asset(&id).err().unwrap(),
            "NOTE_EXPORT_ASSET_READ：笔记输入无效：附件路径不属于对象存储"
        );
    }

    #[test]
    fn consumer_clones_share_the_existing_store_and_retain_it_for_background_reads() {
        let (_root, store, id) = fixture();
        let reader = NoteExportAssetReader::for_store(&store);
        let background = reader.clone();
        store
            .connection()
            .unwrap()
            .execute(
                "update content_objects set mime_type='image/jpeg' where sha256=?1",
                [&id],
            )
            .unwrap();
        drop(store);
        drop(reader);
        let asset = background.read_export_asset(&id).unwrap();
        assert_eq!(asset.mime_type, "image/jpeg");
        assert_eq!(asset.bytes, b"existing asset bytes");
    }
}
