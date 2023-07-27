mod indexer {
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use tokio::fs::DirEntry;

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
pub use indexer::iter_directory;
