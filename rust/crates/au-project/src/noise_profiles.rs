//! Storage for Audacity noise profiles in project SQLite databases.

use rusqlite::{params, Connection, OpenFlags};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// SQLite table used to store noise profiles.
pub const TABLE: &str = "aurs_noise_profiles";

/// Replaces all stored profiles in an existing project database.
pub fn save(path: &Path, profiles: &[(String, Vec<u8>)]) -> Result<(), String> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|error| error.to_string())?;

    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;

    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS aurs_noise_profiles (
                device TEXT PRIMARY KEY,
                captured_at INTEGER,
                data BLOB
            );",
        )
        .map_err(|error| error.to_string())?;

    transaction
        .execute("DELETE FROM aurs_noise_profiles", [])
        .map_err(|error| error.to_string())?;

    let captured_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64;

    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO aurs_noise_profiles (device, captured_at, data)
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(|error| error.to_string())?;

        for (device, data) in profiles {
            statement
                .execute(params![device, captured_at, data])
                .map_err(|error| error.to_string())?;
        }
    }

    transaction.commit().map_err(|error| error.to_string())
}

/// Loads profiles ordered by device, returning an empty vector if the table is absent.
pub fn load(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;

    let table_exists: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type = 'table' AND name = ?1
            )",
            [TABLE],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    if !table_exists {
        return Ok(Vec::new());
    }

    let mut statement = connection
        .prepare("SELECT device, data FROM aurs_noise_profiles ORDER BY device")
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE_ID: AtomicU64 = AtomicU64::new(0);

    fn fresh_database_path() -> std::path::PathBuf {
        let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "au-project-noise-profiles-{}-{}-{id}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        drop(Connection::open(&path).expect("create test SQLite database"));
        path
    }

    #[test]
    fn save_load_round_trip() {
        let path = fresh_database_path();
        let profiles = vec![
            ("device-b".to_string(), vec![1, 2, 3]),
            ("device-a".to_string(), vec![9, 8]),
        ];

        save(&path, &profiles).expect("save profiles");
        assert_eq!(
            load(&path).expect("load profiles"),
            vec![
                ("device-a".to_string(), vec![9, 8]),
                ("device-b".to_string(), vec![1, 2, 3]),
            ]
        );

        std::fs::remove_file(path).expect("remove test database");
    }

    #[test]
    fn save_replaces_all_rows() {
        let path = fresh_database_path();

        save(
            &path,
            &[
                ("old-a".to_string(), vec![1]),
                ("old-b".to_string(), vec![2]),
            ],
        )
        .expect("save initial profiles");

        let replacement = vec![("new".to_string(), vec![4, 5, 6])];
        save(&path, &replacement).expect("replace profiles");
        assert_eq!(load(&path).expect("load replacement"), replacement);

        save(&path, &[]).expect("clear profiles");
        assert!(load(&path).expect("load cleared profiles").is_empty());

        std::fs::remove_file(path).expect("remove test database");
    }

    #[test]
    fn missing_table_returns_empty() {
        let path = fresh_database_path();

        assert!(load(&path).expect("load without profile table").is_empty());

        std::fs::remove_file(path).expect("remove test database");
    }
}
