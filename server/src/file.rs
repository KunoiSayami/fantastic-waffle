mod files {
    use super::FileEventHelper;
    use crate::database::current::{delete, insert, mark, query_path, reset_all_mark, update};
    use crate::file::types::FileEvent;
    use anyhow::anyhow;
    use async_walkdir::WalkDir;
    use futures::StreamExt;
    use log::{error, info};
    use publib::file::get_hash;
    use publib::types::{AsyncExitExt, FileEntry};
    use publib::PATH_UTF8_ERROR;
    use sqlx::SqliteConnection;
    use std::path::{Path, PathBuf};
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;

    pub async fn init_files(conn: &mut SqliteConnection, path: String) -> anyhow::Result<()> {
        reset_all_mark(conn).await?;
        let mut entries = WalkDir::new(path);
        while let Some(Ok(entry)) = entries.next().await {
            process_file(conn, entry).await?;
        }
        Ok(())
    }

    pub async fn process_file(
        conn: &mut SqliteConnection,
        entry: async_walkdir::DirEntry,
    ) -> anyhow::Result<()> {
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
        async fn event_handler(
            conn: &mut SqliteConnection,
            event: FileEvent,
        ) -> anyhow::Result<()> {
            match event {
                FileEvent::New(ref paths) | FileEvent::Update(ref paths) => {
                    let event_type = if let FileEvent::New(_) = event {
                        "new"
                    } else {
                        "update"
                    };
                    for path in paths {
                        let path: &Path = path.as_ref();
                        let hash = get_hash(path)
                            .await
                            .map_err(|e| anyhow!("Get file hash error({}): {:?}", event_type, e))?;

                        insert(
                            conn,
                            FileEntry::try_from_path(path, hash).map_err(|e| {
                                anyhow!("Unable read metadata({}): {:?}", event_type, e)
                            })?,
                        )
                        .await
                        .map_err(|e| anyhow!("Unable insert file({}): {:?}", event_type, e))?;
                    }
                }

                FileEvent::Remove(paths) => {
                    for path in paths {
                        let path: &Path = path.as_ref();
                        delete(conn, path.to_str().expect(PATH_UTF8_ERROR).to_string())
                            .await
                            .map_err(|e| anyhow!("Unable delete path {:?}: {:?}", path, e))?;
                    }
                }
                _ => unreachable!(),
            }
            Ok(())
        }

        async fn handler(
            mut conn: SqliteConnection,
            mut receiver: mpsc::Receiver<FileEvent>,
        ) -> anyhow::Result<()> {
            while let Some(event) = receiver.recv().await {
                match event {
                    FileEvent::New(_) | FileEvent::Update(_) | FileEvent::Remove(_) => {
                        Self::event_handler(&mut conn, event)
                            .await
                            .inspect_err(|e| error!("{}", e))
                            .ok();
                    }
                    FileEvent::Terminate => break,
                    FileEvent::Unknown => {
                        unreachable!()
                    }
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
        Request(Vec<String>, oneshot::Sender<Vec<FileEntry>>),
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
        pub(super) fn new() -> (Self, mpsc::Receiver<FileEvent>) {
            let (sender, receiver) = mpsc::channel(2048);
            (Self { upstream: sender }, receiver)
        }

        pub async fn send(&self, event: Event) -> Option<()> {
            self.upstream.send(event.into()).await.ok()
        }

        pub async fn send_request(
            &self,
            paths: Vec<String>,
        ) -> Option<oneshot::Receiver<Vec<FileEntry>>> {
            let (sender, receiver) = oneshot::channel();
            self.upstream
                .send(FileEvent::Request(paths, sender))
                .await
                .ok()?;
            Some(receiver)
        }
    }
}

mod watcher {
    use crate::file::types::FileEventHelper;
    use log::{error, warn};
    use notify::{Event, EventKind, RecursiveMode, Watcher};
    use publib::types::{AsyncExitExt, ExitExt};
    use std::path::Path;
    use std::thread::JoinHandle;
    use tap::TapOptional;

    #[derive(Debug)]
    pub struct FileWatcher {
        handler: JoinHandle<Result<(), notify::Error>>,
        exit_shot: oneshot::Sender<bool>,
    }

    impl FileWatcher {
        pub fn watcher<P: AsRef<Path>>(
            path: P,
            exit_signal: oneshot::Receiver<bool>,
            upstream: FileEventHelper,
        ) -> Result<(), notify::Error> {
            let mut watcher = notify::recommended_watcher(move |res| match res {
                Ok(event) => {}
                Err(e) => {}
            })?;
            watcher
                .watch(path.as_ref(), RecursiveMode::Recursive)
                .inspect_err(|e| error!("[file watcher]Unable to watch directory: {:?}", e))?;
            exit_signal
                .recv()
                .inspect_err(|e| {
                    error!("[file watcher]Got error while poll oneshot event: {:?}", e)
                })
                .ok();
            watcher
                .unwatch(path.as_ref())
                .inspect_err(|e| error!("[file watcher]Unable to unwatch directory: {:?}", e))?;
            Ok(())
        }

        pub fn event_handler(event: Event, upstream: FileEventHelper) {
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                    tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(upstream.send(event))
                        .tap_none(|| warn!("Unable send event to file daemon"));
                }
                _ => {}
            }
        }

        pub fn start<P: AsRef<Path> + Send + 'static>(
            path: P,
            event_helper: FileEventHelper,
        ) -> Self {
            let (sender, receiver) = oneshot::channel();
            let handler = std::thread::spawn(move || Self::watcher(path, receiver, event_helper));
            Self::new(handler, sender)
        }

        fn new(
            handler: JoinHandle<Result<(), notify::Error>>,
            exit_shot: oneshot::Sender<bool>,
        ) -> Self {
            Self { handler, exit_shot }
        }
    }

    impl ExitExt for FileWatcher {
        fn _send_terminate(self) -> Option<()> {
            self.exit_shot.send(true).ok()
        }

        fn is_finished(&self) -> bool {
            self.handler.is_finished()
        }
    }
}

pub use files::FileDaemon;
pub use types::FileEventHelper;
pub use watcher::FileWatcher;
