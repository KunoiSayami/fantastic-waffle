#![feature(async_closure)]
#![feature(generators)]

pub mod file;
pub mod types;

pub const PATH_UTF8_ERROR: &str = "Unexpect non UTF-8 path";

pub fn check_penetration(path: &str) -> bool {
    let current_dir = std::env::current_dir().unwrap();

    let mut new_path = current_dir.clone();
    new_path.push(path);

    match std::fs::canonicalize(new_path) {
        Ok(path) => path.starts_with(current_dir),
        Err(_) => false,
    }
}

pub fn append_current_path(path: &str) -> std::path::PathBuf {
    let mut current_dir = std::env::current_dir().unwrap();
    current_dir.push(path);
    current_dir
}

#[cfg(test)]
mod test {
    use crate::check_penetration;

    #[test]
    fn test_path_check() {
        assert_eq!(check_penetration("../"), false);
        assert_eq!(check_penetration("../publib/Cargo.toml"), true);
        assert_eq!(check_penetration("Cargo.toml"), true);
        assert_eq!(check_penetration("../publib/src/lib.rs"), true);
    }
}
