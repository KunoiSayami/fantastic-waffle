#[deprecated]
#[cfg(feature = "deprecated")]
mod indexer {
    use futures_core::Stream;
    use std::fmt::Display;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::fs::{DirEntry, ReadDir};

    pub fn iter_directory<P: AsRef<Path> + 'static, Fut: Future<Output = anyhow::Result<()>>>(
        dir: P,
        f: impl Fn(DirEntry) -> Fut + Copy + 'static,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>>>> {
        Box::pin(async move {
            let mut entries = tokio::fs::read_dir(dir).await?;
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.path().is_dir() {
                    iter_directory(entry.path(), f).await?;
                }
                //println!("{}", entry.path().display());
                f(entry).await?;
            }
            Ok(())
        })
    }

    #[derive(Clone, Debug)]
    enum EntryState {
        Dir(DirEntry),
        File(DirEntry),
    }

    #[derive(Clone, Debug)]
    pub struct WalkDir {
        stack: Vec<DirEntry>,
        current_dir: Poll<Option<ReadDir>>,
    }

    impl WalkDir {
        pub async fn new<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
            Ok(Self {
                stack: vec![],
                current_dir: Poll::Ready(Some(tokio::fs::read_dir(path)?)),
            })
        }
    }

    impl Stream for WalkDir {
        type Item = Result<DirEntry, std::io::Error>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if let Poll::Ready(ref current_dir) = self.current_dir {
                if self.current_dir.is_none() {
                    return Poll::Ready(None);
                }
                if let Some(ref mut current_dir) = current_dir {
                    return match current_dir.next_entry().poll_next(cx) {
                        Poll::Pending => Poll::Pending,
                        Poll::Ready(None) => {
                            if let Some(entry) = self.stack.pop() {
                                match tokio::fs::read_dir(entry.path()).poll(cx) {
                                    Poll::Ready(Ok(x)) => self.current_dir = Poll::Ready(Some(x)),
                                    Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e))),
                                    Poll::Pending => {}
                                }
                            }
                        }
                    };
                }
            }

            if let Some(ref mut current_dir) = self.current_dir {
                return match current_dir.next_entry().poll_next(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(None) => {
                        //self.current_dir = None;
                        if let Some(entry) = self.stack.pop() {
                            self.current_dir = tokio::fs::read_dir(entry.path()).poll_next()
                        }
                    }
                };
            }
        }
    }

    pub fn iter_directory_ng<P: AsRef<Path> + 'static>(
        dir: P,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>>>> {
        Box::pin(async move {
            let mut stack: Vec<EntryState> = Vec::new();
            let mut current_dir = tokio::fs::read_dir(dir).await?;

            while let Ok(Some(entry)) = current_dir.next_entry().await? {
                match entry {
                    EntryState::Dir(dir_entry) => {
                        let mut entries = tokio::fs::read_dir(dir_entry.path()).await?;
                        stack.push(EntryState::File(dir_entry));

                        while let Some(entry) = entries.next_entry().await? {
                            if entry.path().is_dir() {
                                stack.push(EntryState::Dir(entry));
                            } else {
                                // Process files here if needed.
                                //println!("{}", entry.path().display());
                            }
                        }
                    }
                    EntryState::File(dir_entry) => {
                        // Process files here if needed (after processing the directory).
                        //println!("Processing directory: {}", dir_entry.path().display());
                    }
                }
            }

            while let Some(entry) = stack.pop() {
                match entry {
                    EntryState::Dir(dir_entry) => {
                        let mut entries = tokio::fs::read_dir(dir_entry.path()).await?;
                        stack.push(EntryState::File(dir_entry));

                        while let Some(entry) = entries.next_entry().await? {
                            if entry.path().is_dir() {
                                stack.push(EntryState::Dir(entry));
                            } else {
                                // Process files here if needed.
                                //println!("{}", entry.path().display());
                            }
                        }
                    }
                    EntryState::File(dir_entry) => {
                        // Process files here if needed (after processing the directory).
                        //println!("Processing directory: {}", dir_entry.path().display());
                    }
                }
            }

            Ok(())
        })
    }

    pub fn iter_dir_ng<P: AsRef<Path>>(dir: P) -> impl Stream<Item = std::fs::DirEntry> {
        let mut entries = std::fs::read_dir(dir)?;
        for entry in entries {
            let entry = entry?;
            || {
                yield entry;
            };
            if entry.path().is_dir() {
                || yield iter_dir_ng(entry.path());
            }
            //println!("{}", entry.path().display());
        }
        Ok(())
    }

    #[cfg(test)]
    mod test {
        use super::iter_directory;

        #[test]
        fn test_iter_files() {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(iter_directory(".", async move |entry| {
                    println!("{}", entry.path().display());
                    Ok(())
                }))
                .unwrap()
        }
    }
}

mod hash {
    use std::path::Path;
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;
    use xxhash_rust::xxh3::Xxh3;

    const BUFFER_SIZE: usize = 1024;

    pub async fn get_file_hash<P: AsRef<Path>>(path: P) -> Result<u64, std::io::Error> {
        if path.as_ref().is_dir() {
            return Ok(0);
        }
        let mut buffer = [0u8; BUFFER_SIZE];
        let mut xxhash = Xxh3::new();
        let mut file = File::open(path).await?;
        while let Ok(read_size) = file.read(&mut buffer).await {
            xxhash.update(&buffer);
            if read_size < BUFFER_SIZE {
                break;
            }
        }
        Ok(xxhash.digest())
    }

    pub async fn get_hash<P: AsRef<Path>>(path: P) -> Result<Option<u64>, std::io::Error> {
        if path.as_ref().is_dir() {
            return Ok(None);
        }
        get_file_hash(path).await.map(|hash| Some(hash))
    }
}

pub use hash::{get_file_hash, get_hash};
