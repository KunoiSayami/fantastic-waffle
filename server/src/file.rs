mod files {
    use super::FileEventHelper;
    use crate::database::current::{insert, mark, query_path, reset_all_mark, update};
    use crate::file::types::FileEvent;
    use log::info;
    use publib::file::{get_hash, iter_directory};
    use publib::types::FileEntry;
    use sqlx::SqliteConnection;
    use tokio::fs::DirEntry;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;

    pub async fn init_files(conn: &mut SqliteConnection, path: String) -> anyhow::Result<()> {
        reset_all_mark(conn).await?;
        iter_directory(&path, async move |entry| {
            process_file(conn, entry).await?;
            Ok(())
        })
        .await?;
        Ok(())
    }

    pub async fn process_file(conn: &mut SqliteConnection, entry: DirEntry) -> anyhow::Result<()> {
        match query_path(conn, entry.path()).await? {
            None => {
                let hash = get_hash(entry.path()).await?.map(|x| format!("{}", x));
                insert(conn, FileEntry::try_from_entry(entry, hash).await?).await?;
            }
            Some(sql_entry) => {
                let entry = FileEntry::try_from_entry::<String>(entry, None).await?;
                if sql_entry == entry {
                    mark(conn, entry).await?;
                    return Ok(());
                }
                // mtime || size not match
                let hash = get_hash(entry.path()).await?;
                let entry = entry.override_hash(hash);
                // maybe mtime change but hash same
                if sql_entry.check_hash_only(&entry) {
                    info!("{} changed but hash is same", entry.path());
                } else {
                    info!("{} updated", entry.path());
                }
                update(conn, entry).await?;
            }
        }
        Ok(())
    }

    #[derive(Debug)]
    pub struct FileDaemon {
        handler: JoinHandle<anyhow::Result<()>>,
    }

    impl FileDaemon {
        pub async fn handler(
            mut conn: SqliteConnection,
            mut receiver: mpsc::Receiver<FileEvent>,
        ) -> anyhow::Result<()> {
            while let Some(event) = receiver.recv().await {
                match event {
                    FileEvent::New(_) => {}
                    FileEvent::Update(_) => {}
                    FileEvent::Remove(_) => {}
                    FileEvent::Terminate => break,
                    FileEvent::Unknown => {}
                    FileEvent::Request(path, sender) => {}
                }
            }
            Ok(())
        }

        pub fn start(conn: SqliteConnection) -> (Self, FileEventHelper) {
            let (helper, receiver) = FileEventHelper::new();
            let handler = tokio::spawn(Self::handler(conn, receiver));
            (Self { handler }, helper)
        }
    }
}

mod types {
    use notify::{Event, EventKind};
    use publib::types::FileEntry;
    use publib::PATH_UTF8_ERROR;
    use std::path::PathBuf;
    use tokio::sync::{mpsc, oneshot};

    pub(super) enum FileEvent {
        New(Vec<String>),
        Update(Vec<String>),
        Remove(Vec<String>),
        /// Request files (from https)
        Request(String, oneshot::Sender<Vec<FileEntry>>),
        Terminate,
        Unknown,
    }

    fn convert(paths: Vec<PathBuf>) -> Option<Vec<String>> {
        paths
            .iter()
            .map(|path| path.to_str().map(|s| s.to_string()))
            .collect::<Option<Vec<_>>>()
    }

    impl From<Event> for FileEvent {
        fn from(value: Event) -> Self {
            let paths = convert(value.paths).expect(PATH_UTF8_ERROR);
            match value.kind {
                EventKind::Create(_) => Self::New(paths),
                EventKind::Modify(_) => Self::Update(paths),
                EventKind::Remove(_) => Self::Remove(paths),
                _ => Self::Unknown,
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct FileEventHelper {
        upstream: mpsc::Sender<FileEvent>,
    }

    impl FileEventHelper {
        pub fn new() -> (Self, mpsc::Receiver<FileEvent>) {
            let (sender, receiver) = mpsc::channel(2048);
            (Self { upstream: sender }, receiver)
        }
    }
}

mod watcher {
    use crate::file::types::FileEventHelper;
    use notify::Event;
    use std::path::Path;
    use std::thread::JoinHandle;

    #[derive(Debug)]
    pub struct FileWatcher {
        handler: JoinHandle<Result<(), std::io::Error>>,
        upstream: FileEventHelper,
    }

    impl FileWatcher {
        pub fn watcher<P: AsRef<Path>>(path: P) -> Result<(), std::io::Error> {
            let mut watcher = notify::recommended_watcher(move |res| match res {
                Ok(event) => {}
                Err(e) => {}
            });
            Ok(())
        }

        pub fn event_handler(event: Event) {
            if event.kind.is_access() {
                return;
            }
        }
    }
}

pub use types::FileEventHelper;
