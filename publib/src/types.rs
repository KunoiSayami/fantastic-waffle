mod file_entry {
    use crate::PATH_UTF8_ERROR;
    use sqlx::sqlite::SqliteRow;
    use sqlx::{Error, FromRow, Row};
    use std::fmt::Display;
    #[cfg(target_os = "linux")]
    use std::os::unix::prelude::MetadataExt;
    use tokio::fs::DirEntry;

    #[derive(Clone, Debug)]
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

        pub async fn try_from_entry<D: Display + std::default::Default>(
            entry: DirEntry,
            hash: Option<D>,
        ) -> Result<Self, std::io::Error> {
            let meta = entry.metadata().await?;
            Ok(Self::new(
                entry.path().to_str().expect(PATH_UTF8_ERROR).to_string(),
                hash.unwrap_or_default(),
                meta.mtime(),
                meta.size() as i64,
                meta.is_dir(),
            ))
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
    }

    impl PartialEq<FileEntry> for FileEntry {
        fn eq(&self, other: &FileEntry) -> bool {
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
}

pub use file_entry::FileEntry;