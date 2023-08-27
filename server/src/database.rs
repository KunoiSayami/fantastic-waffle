pub mod v1 {
    use format_sql_query::QuotedData;
    use publib::types::FileEntry;
    use publib::PATH_UTF8_ERROR;
    use sqlx::{Result, SqliteConnection};
    use std::path::Path;

    pub const VERSION: &str = "1";

    pub(super) const CREATE_TABLE: &str = r#"
        CREATE TABLE "files" (
            "path"	TEXT NOT NULL,
            "hash"	TEXT,
            "mtime"	INTEGER NOT NULL DEFAULT 0,
            "size"	INTEGER NOT NULL DEFAULT 0,
            "is_dir"	INTEGER NOT NULL DEFAULT 0,
            "marked"    INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY("path")
        );
                
        CREATE TABLE "meta" (
            "key" TEXT NOT NULL,
            "value" TEXT
        );
        "#;

    pub async fn query_path<P: AsRef<Path>>(
        conn: &mut SqliteConnection,
        path: P,
    ) -> Result<Option<FileEntry>> {
        query(conn, path.as_ref().to_str().expect(PATH_UTF8_ERROR)).await
    }

    pub async fn query_by_path(
        conn: &mut SqliteConnection,
        path: String,
    ) -> Result<Vec<FileEntry>> {
        let quoted = insert_percent(path);
        sqlx::query_as::<_, FileEntry>(&format!(
            r#"SELECT * FROM "files" WHERE "path" LIKE {}"#,
            quoted
        ))
        .fetch_all(conn)
        .await
    }

    pub async fn query(conn: &mut SqliteConnection, path: &str) -> Result<Option<FileEntry>> {
        sqlx::query_as::<_, FileEntry>(r#"SELECT * FROM "files" WHERE "path" = ?"#)
            .bind(path)
            .fetch_optional(conn)
            .await
    }

    pub async fn update(conn: &mut SqliteConnection, entry: FileEntry) -> Result<()> {
        sqlx::query(r#"UPDATE "files" SET "hash" = ?, "mtime" = ?, "size" = ?, "marked" = 1" WHERE "path" = ?"#)
            .bind(entry.hash())
            .bind(entry.mtime())
            .bind(entry.size())
            .bind(entry.path())
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn mark_path<P: AsRef<Path>>(conn: &mut SqliteConnection, path: P) -> Result<()> {
        mark_path_str(conn, path.as_ref().to_str().expect(PATH_UTF8_ERROR)).await
    }

    pub async fn mark_path_str(conn: &mut SqliteConnection, path: &str) -> Result<()> {
        sqlx::query(r#"UPDATE "files" SET "marked" = 1 WHERE "path" = ?"#)
            .bind(path)
            .execute(conn)
            .await
            .map(|_| ())
    }

    pub async fn mark(conn: &mut SqliteConnection, entry: FileEntry) -> Result<()> {
        mark_path_str(conn, entry.path()).await
    }

    pub async fn reset_all_mark(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(r#"UPDATE "files" SET "marked" = 0"#)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn delete(conn: &mut SqliteConnection, path: String) -> Result<()> {
        let p: &Path = path.as_ref();
        if p.is_dir() {
            let quoted = insert_percent(path);
            sqlx::query(&format!(
                r#"DELETE FROM "files" WHERE "path" LIKE {}"#,
                quoted
            ))
            .execute(conn)
            .await?;
        } else {
            sqlx::query(r#"DELETE FROM "files" WHERE "path" = ?"#)
                .bind(path)
                .execute(conn)
                .await?;
        }
        Ok(())
    }

    pub async fn delete_all_unmarked(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(r#"DELETE FROM "files" WHERE "marked" = 0"#)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn insert(conn: &mut SqliteConnection, entry: FileEntry) -> Result<()> {
        if entry.is_dir() {
            sqlx::query(r#"INSERT INTO "files" ("path", "is_dir") VALUES (?, ?)"#)
                .bind(entry.path())
                .bind(1)
                .execute(conn)
                .await?;
        } else {
            sqlx::query(r#"INSERT INTO "files" VALUES (?, ?, ?, ?, ?, ?)"#)
                .bind(entry.path())
                .bind(entry.hash())
                .bind(entry.mtime())
                .bind(entry.size())
                .bind(0)
                .bind(1)
                .execute(conn)
                .await?;
        }
        Ok(())
    }

    pub fn insert_percent(s: String) -> String {
        let mut quoted = QuotedData(&if s.ends_with('/') {
            s
        } else {
            format!("{}/", s)
        })
        .to_string();
        debug_assert!(quoted.ends_with("'"));
        quoted.insert(quoted.len() - 1, '%');
        quoted
    }
}

pub async fn load_database(path: &str) -> sqlx::Result<sqlx::SqliteConnection> {
    let mut conn = SqliteConnectOptions::new()
        .create_if_missing(true)
        .filename(path)
        .connect()
        .await?;
    if !check_database(&mut conn, "meta").await? {
        sqlx::query(current::CREATE_TABLE)
            .execute(&mut conn)
            .await?;
        insert_database_version(&mut conn, "meta", VERSION).await?;
    }
    Ok(conn)
}

use kstool::sqlx::{check_database, insert_database_version};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::ConnectOptions;
pub use v1 as current;
pub use v1::VERSION;
