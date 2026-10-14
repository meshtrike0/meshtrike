//! Storage (section 10, D30/D34): each of the two databases is a key-value
//! store of typed records. On device it is a SQLCipher 4 database
//! (`sqlcipher` feature); in the browser harness and simulator it is memory.
//! Both detect a wrong database key on open.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StorageError {
    #[error("wrong key")]
    WrongKey,
    #[error("io: {0}")]
    Io(String),
    #[error("corrupt record")]
    Corrupt,
}

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Put(&'static str, Vec<u8>, Vec<u8>),
    Delete(&'static str, Vec<u8>),
    ClearTable(&'static str),
}

pub trait Kv: Send {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn scan(&self, table: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
    /// Records whose key starts with `prefix`, ordered by key.
    fn scan_prefix(&self, table: &str, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .scan(table)?
            .into_iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .collect())
    }
    /// Apply all operations atomically.
    fn apply(&mut self, ops: Vec<Op>) -> Result<()>;
    fn count(&self, table: &str) -> Result<usize> {
        Ok(self.scan(table)?.len())
    }
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<()> {
        self.apply(vec![Op::Put(table, key.to_vec(), value.to_vec())])
    }
    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<()> {
        self.apply(vec![Op::Delete(table, key.to_vec())])
    }
}

// ===== Memory backend =====

/// In-memory store. The key check mirrors SQLCipher: opening an existing
/// store with a different key fails with `WrongKey`.
#[derive(Default)]
pub struct MemoryKv {
    key_check: [u8; 32],
    data: BTreeMap<(String, Vec<u8>), Vec<u8>>,
}

impl MemoryKv {
    pub fn create(key_check: [u8; 32]) -> Self {
        Self {
            key_check,
            data: BTreeMap::new(),
        }
    }

    /// Re-open with `key_check` (a one-way function of the database key).
    pub fn verify(&self, key_check: &[u8; 32]) -> Result<()> {
        if &self.key_check == key_check {
            Ok(())
        } else {
            Err(StorageError::WrongKey)
        }
    }
}

impl Kv for MemoryKv {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.data.get(&(table.to_string(), key.to_vec())).cloned())
    }

    fn scan(&self, table: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let lo = (table.to_string(), Vec::new());
        Ok(self
            .data
            .range(lo..)
            .take_while(|((t, _), _)| t == table)
            .map(|((_, k), v)| (k.clone(), v.clone()))
            .collect())
    }

    fn scan_prefix(&self, table: &str, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let lo = (table.to_string(), prefix.to_vec());
        Ok(self
            .data
            .range(lo..)
            .take_while(|((t, k), _)| t == table && k.starts_with(prefix))
            .map(|((_, k), v)| (k.clone(), v.clone()))
            .collect())
    }

    fn apply(&mut self, ops: Vec<Op>) -> Result<()> {
        for op in ops {
            match op {
                Op::Put(t, k, v) => {
                    self.data.insert((t.to_string(), k), v);
                }
                Op::Delete(t, k) => {
                    self.data.remove(&(t.to_string(), k));
                }
                Op::ClearTable(t) => self.data.retain(|(tt, _), _| tt != t),
            }
        }
        Ok(())
    }

    fn count(&self, table: &str) -> Result<usize> {
        let lo = (table.to_string(), Vec::new());
        Ok(self
            .data
            .range(lo..)
            .take_while(|((t, _), _)| t == table)
            .count())
    }
}

// ===== SQLCipher backend =====

#[cfg(feature = "sqlcipher")]
pub mod sqlcipher {
    use super::*;
    use rusqlite::{params, Connection};

    pub const SCHEMA_VERSION: i64 = 1;

    fn io(e: rusqlite::Error) -> StorageError {
        StorageError::Io(e.to_string())
    }

    pub struct SqlCipherKv {
        conn: Connection,
    }

    impl SqlCipherKv {
        /// Open or create `path` keyed by a raw 256-bit key (section 14:
        /// cipher_page_size 4096, raw key so no KDF, WAL, secure_delete).
        pub fn open(path: &str, key: &[u8; 32]) -> Result<Self> {
            let conn = Connection::open(path).map_err(io)?;
            conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex::encode(key)))
                .map_err(io)?;
            conn.execute_batch("PRAGMA cipher_page_size = 4096; PRAGMA kdf_iter = 1;")
                .map_err(io)?;
            // A wrong key surfaces as "file is not a database" on first read.
            if conn
                .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                    r.get::<_, i64>(0)
                })
                .is_err()
            {
                return Err(StorageError::WrongKey);
            }
            conn.execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA secure_delete = ON;
                 CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS kv (t TEXT NOT NULL, k BLOB NOT NULL, v BLOB NOT NULL,
                   PRIMARY KEY (t, k)) WITHOUT ROWID;",
            )
            .map_err(io)?;
            let mut s = Self { conn };
            s.migrate()?;
            Ok(s)
        }

        fn migrate(&mut self) -> Result<()> {
            let v: Option<i64> = self
                .conn
                .query_row("SELECT max(version) FROM schema_version", [], |r| r.get(0))
                .map_err(io)?;
            if v.is_none() {
                self.conn
                    .execute(
                        "INSERT INTO schema_version VALUES (?1)",
                        params![SCHEMA_VERSION],
                    )
                    .map_err(io)?;
            }
            Ok(())
        }
    }

    impl Kv for SqlCipherKv {
        fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
            let mut st = self
                .conn
                .prepare_cached("SELECT v FROM kv WHERE t = ?1 AND k = ?2")
                .map_err(io)?;
            let mut rows = st.query(params![table, key]).map_err(io)?;
            match rows.next().map_err(io)? {
                Some(r) => Ok(Some(r.get(0).map_err(io)?)),
                None => Ok(None),
            }
        }

        fn scan(&self, table: &str) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
            let mut st = self
                .conn
                .prepare_cached("SELECT k, v FROM kv WHERE t = ?1 ORDER BY k")
                .map_err(io)?;
            let rows = st
                .query_map(params![table], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(io)?;
            rows.collect::<std::result::Result<_, _>>().map_err(io)
        }

        fn scan_prefix(&self, table: &str, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
            let mut st = self
                .conn
                .prepare_cached(
                    "SELECT k, v FROM kv WHERE t = ?1 AND substr(k, 1, ?2) = ?3 ORDER BY k",
                )
                .map_err(io)?;
            let rows = st
                .query_map(params![table, prefix.len() as i64, prefix], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .map_err(io)?;
            rows.collect::<std::result::Result<_, _>>().map_err(io)
        }

        fn count(&self, table: &str) -> Result<usize> {
            self.conn
                .query_row(
                    "SELECT count(*) FROM kv WHERE t = ?1",
                    params![table],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n as usize)
                .map_err(io)
        }

        fn apply(&mut self, ops: Vec<Op>) -> Result<()> {
            let tx = self.conn.transaction().map_err(io)?;
            for op in ops {
                match op {
                    Op::Put(t, k, v) => tx.execute(
                        "INSERT OR REPLACE INTO kv (t, k, v) VALUES (?1, ?2, ?3)",
                        params![t, k, v],
                    ),
                    Op::Delete(t, k) => {
                        tx.execute("DELETE FROM kv WHERE t = ?1 AND k = ?2", params![t, k])
                    }
                    Op::ClearTable(t) => tx.execute("DELETE FROM kv WHERE t = ?1", params![t]),
                }
                .map_err(io)?;
            }
            tx.commit().map_err(io)
        }
    }

    /// Delete a database and its WAL/SHM side files (panic wipe).
    pub fn wipe(path: &str) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(kv: &mut dyn Kv) {
        kv.put("a", b"1", b"x").unwrap();
        kv.put("a", b"2", b"y").unwrap();
        kv.put("b", b"1", b"z").unwrap();
        assert_eq!(kv.get("a", b"1").unwrap(), Some(b"x".to_vec()));
        kv.put("p", b"ab1", b"1").unwrap();
        kv.put("p", b"ab2", b"2").unwrap();
        kv.put("p", b"ac1", b"3").unwrap();
        assert_eq!(kv.scan_prefix("p", b"ab").unwrap().len(), 2);
        assert_eq!(kv.scan("a").unwrap().len(), 2);
        assert_eq!(kv.count("b").unwrap(), 1);
        kv.apply(vec![Op::Delete("a", b"1".to_vec()), Op::ClearTable("b")])
            .unwrap();
        assert_eq!(kv.get("a", b"1").unwrap(), None);
        assert_eq!(kv.count("b").unwrap(), 0);
        assert_eq!(kv.count("a").unwrap(), 1);
    }

    #[test]
    fn memory_backend() {
        let mut m = MemoryKv::create([1; 32]);
        exercise(&mut m);
        assert!(m.verify(&[1; 32]).is_ok());
        assert_eq!(m.verify(&[2; 32]), Err(StorageError::WrongKey));
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn sqlcipher_backend_and_wrong_key() {
        let dir = std::env::temp_dir().join(format!("mesh-kv-{}", std::process::id()));
        let path = dir.to_str().unwrap().to_string();
        sqlcipher::wipe(&path);
        {
            let mut db = sqlcipher::SqlCipherKv::open(&path, &[7; 32]).unwrap();
            exercise(&mut db);
        }
        assert!(matches!(
            sqlcipher::SqlCipherKv::open(&path, &[8; 32]),
            Err(StorageError::WrongKey)
        ));
        let db = sqlcipher::SqlCipherKv::open(&path, &[7; 32]).unwrap();
        assert_eq!(db.count("a").unwrap(), 1);
        drop(db);
        // The file must not contain plaintext.
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"SQLite"));
        sqlcipher::wipe(&path);
    }
}
