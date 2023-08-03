mod file_entry {
    use crate::types::{FileMeta, OptionFile};
    use crate::PATH_UTF8_ERROR;
    use async_walkdir::DirEntry;
    use serde_derive::{Deserialize, Serialize};
    use sqlx::sqlite::SqliteRow;
    use sqlx::{Error, FromRow, Row};
    use std::fmt::Display;
    #[cfg(target_os = "linux")]
    use std::os::unix::prelude::MetadataExt;
    use std::path::Path;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct FileEntry {
        path: String,
        hash: String,
        mtime: i64,
        size: i64,
        is_dir: bool,
    }

    impl FileEntry {
        pub fn path(&self) -> &str {
            &self.path
        }
        pub fn hash(&self) -> &str {
            &self.hash
        }

        pub fn mtime(&self) -> i64 {
            if self.is_dir {
                return 0;
            }
            self.mtime
        }
        pub fn size(&self) -> i64 {
            if self.is_dir {
                return 0;
            }
            self.size
        }
        pub fn is_dir(&self) -> bool {
            self.is_dir
        }
        pub fn new<D: Display>(path: String, hash: D, mtime: i64, size: i64, is_dir: bool) -> Self {
            Self {
                path,
                hash: hash.to_string(),
                mtime,
                size,
                is_dir,
            }
        }

        pub fn check_hash_only(&self, other: &Self) -> bool {
            if self.is_dir {
                return self.is_dir == other.is_dir;
            }
            self.hash == other.hash
        }

        #[allow(unused)]
        #[deprecated]
        pub async fn check_fmeta_only(&self, other: &DirEntry) -> Result<bool, std::io::Error> {
            let meta = other.metadata().await?;
            if meta.is_dir() {
                return Ok(self.is_dir);
            }
            Ok(self.mtime == meta.mtime() && self.size as u64 == meta.size())
        }

        pub fn override_hash<D: Display + std::default::Default>(
            mut self,
            hash: Option<D>,
        ) -> Self {
            self.hash = hash.unwrap_or_default().to_string();
            self
        }

        pub fn try_from_path<P: AsRef<Path> + Send + Sync, D: Display + Default>(
            path: P,
            hash: Option<D>,
        ) -> Result<Self, std::io::Error> {
            let meta = path.as_ref().metadata()?;
            Ok(Self::from_metadata(path, meta, hash))
        }

        pub fn from_metadata<P: AsRef<Path>, D: Display + Default>(
            path: P,
            metadata: std::fs::Metadata,
            hash: Option<D>,
        ) -> Self {
            Self::new(
                path.as_ref().to_str().expect(PATH_UTF8_ERROR).to_string(),
                hash.unwrap_or_default(),
                metadata.mtime(),
                metadata.size() as i64,
                metadata.is_dir(),
            )
        }

        pub async fn try_from_entry<D: Display + Default>(
            entry: DirEntry,
            hash: Option<D>,
        ) -> Result<Self, std::io::Error> {
            let meta = entry.metadata().await?;
            Ok(Self::from_metadata(entry.path(), meta, hash))
        }

        pub fn to_tb_row(&self) -> String {
            format!(
                "<tb><tr>{}</tr><tr>{}</tr><tr>{}</tr><tr>{}</tr><tr>{}</tr></tb>",
                self.path, self.hash, self.mtime, self.size, self.is_dir
            )
        }

        pub const fn get_tb_title() -> &'static str {
            "<tb><tr>Path</tr><tr>Hash</tr><tr>mtime</tr><tr>size</tr><tr>is_dir</tr></tb>"
        }
    }

    impl PartialEq<Self> for FileEntry {
        fn eq(&self, other: &Self) -> bool {
            if self.is_dir {
                return self.is_dir == other.is_dir;
            }
            self.mtime == other.mtime && self.size == other.size
        }
    }

    impl FromRow<'_, SqliteRow> for FileEntry {
        fn from_row(row: &'_ SqliteRow) -> Result<Self, Error> {
            Ok(Self::new(
                row.try_get(0)?,
                row.try_get::<Option<String>, _>(1)?.unwrap_or_default(),
                row.try_get(2)?,
                row.try_get(3)?,
                row.try_get::<i32, _>(4)? != 0,
            ))
        }
    }

    impl From<FileEntry> for OptionFile {
        fn from(value: FileEntry) -> Self {
            Self::new(
                value.path,
                Some(FileMeta::new(
                    value.hash,
                    value.mtime,
                    value.size,
                    value.is_dir,
                )),
            )
        }
    }

    impl From<FileEntry> for FileMeta {
        fn from(value: FileEntry) -> Self {
            Self::new(value.hash, value.mtime, value.size, value.is_dir)
        }
    }
}

mod option_file_entry {
    use crate::types::FileEntry;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct FileMeta {
        hash: String,
        mtime: i64,
        size: i64,
        is_dir: bool,
    }

    impl FileMeta {
        pub fn new(hash: String, mtime: i64, size: i64, is_dir: bool) -> Self {
            Self {
                hash,
                mtime,
                size,
                is_dir,
            }
        }

        pub fn into_file_entry(self, path: String) -> FileEntry {
            FileEntry::new(path, self.hash, self.mtime, self.size, self.is_dir)
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct OptionFile {
        path: String,
        meta: Option<FileMeta>,
    }

    impl OptionFile {
        pub fn is_exist(&self) -> bool {
            return self.meta.is_some();
        }
        pub fn new(path: String, meta: Option<FileMeta>) -> Self {
            Self { path, meta }
        }

        pub fn new_empty(path: String) -> Self {
            Self::new(path, None)
        }

        pub fn into_file_entry(self) -> Option<FileEntry> {
            Some(self.meta?.into_file_entry(self.path))
        }

        pub fn from_option_entry(path: String, entry: Option<FileEntry>) -> Self {
            match entry {
                None => Self::new_empty(path),
                Some(entry) => entry.into(),
            }
        }
    }
}

mod thread_controller {

    #[async_trait::async_trait]
    pub trait AsyncExitExt {
        async fn _send_terminate(&self) -> Option<()>;

        fn is_finished(&self) -> bool;

        async fn stop(&self, not_finished: fn() -> ()) -> Option<()> {
            if !self.is_finished() {
                self._send_terminate().await?;
                for _ in 0..5 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if self.is_finished() {
                        break;
                    }
                }
                if !self.is_finished() {
                    not_finished();
                    return None;
                }
            }
            Some(())
        }
    }

    pub trait ExitExt {
        fn _send_terminate(&self) -> Option<()>;

        fn is_finished(&self) -> bool;

        fn stop(&self, not_finished: fn() -> ()) -> Option<()> {
            if !self.is_finished() {
                self._send_terminate()?;
                for _ in 0..5 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if self.is_finished() {
                        break;
                    }
                }
                if !self.is_finished() {
                    not_finished();
                    return None;
                }
            }
            Some(())
        }
    }
}

pub use file_entry::FileEntry;
pub use option_file_entry::{FileMeta, OptionFile};
pub use thread_controller::{AsyncExitExt, ExitExt};
