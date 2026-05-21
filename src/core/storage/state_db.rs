use std::path::Path;

use rusqlite::{Connection, params};

use crate::types::Error;

pub struct StateDb {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub formula_name: String,
    pub version: String,
    pub store_key: String,
    pub kind: InstalledPackageKind,
    pub installed_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstalledPackageKind {
    Formula,
    App,
}

impl InstalledPackageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Formula => "formula",
            Self::App => "app",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "app" => Self::App,
            _ => Self::Formula,
        }
    }
}

impl StateDb {
    pub fn open(path: &Path) -> Result<Self, Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::StoreCorruption {
                message: format!("failed to create state db directory: {e}"),
            })?;
        }

        let conn = Connection::open(path).map_err(Self::map_sql_error)?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self, Error> {
        let conn = Connection::open_in_memory().map_err(Self::map_sql_error)?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<(), Error> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS installed_packages (
                install_name TEXT PRIMARY KEY,
                formula_name TEXT NOT NULL,
                version TEXT NOT NULL,
                store_key TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('formula', 'app')),
                installed_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS installed_packages_kind_idx
                ON installed_packages(kind);

            CREATE TABLE IF NOT EXISTS package_index (
                name TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('formula', 'app')),
                version TEXT,
                description TEXT,
                source TEXT,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (name, kind)
            );

            CREATE INDEX IF NOT EXISTS package_index_name_idx
                ON package_index(name);",
        )
        .map_err(Self::map_sql_error)
    }

    pub fn record_installed(&self, package: &InstalledPackage) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO installed_packages
                    (install_name, formula_name, version, store_key, kind, installed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    package.name,
                    package.formula_name,
                    package.version,
                    package.store_key,
                    package.kind.as_str(),
                    package.installed_at
                ],
            )
            .map(|_| ())
            .map_err(Self::map_sql_error)
    }

    pub fn remove_installed(&self, name: &str) -> Result<(), Error> {
        self.conn
            .execute(
                "DELETE FROM installed_packages WHERE install_name = ?1",
                params![name],
            )
            .map(|_| ())
            .map_err(Self::map_sql_error)
    }

    pub fn has_installed_packages(&self) -> Result<bool, Error> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM installed_packages)",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(Self::map_sql_error)
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledPackage>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT install_name, formula_name, version, store_key, kind, installed_at
                 FROM installed_packages
                 ORDER BY install_name, version",
            )
            .map_err(Self::map_sql_error)?;

        let rows = stmt
            .query_map([], |row| {
                let kind: String = row.get(4)?;
                Ok(InstalledPackage {
                    name: row.get(0)?,
                    formula_name: row.get(1)?,
                    version: row.get(2)?,
                    store_key: row.get(3)?,
                    kind: InstalledPackageKind::from_str(&kind),
                    installed_at: row.get(5)?,
                })
            })
            .map_err(Self::map_sql_error)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(Self::map_sql_error)
    }

    fn map_sql_error(err: rusqlite::Error) -> Error {
        Error::StoreCorruption {
            message: format!("state database error: {err}"),
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn records_lists_and_removes_installed_packages() {
        let db = StateDb::in_memory().unwrap();
        db.record_installed(&InstalledPackage {
            name: "cask:ghostty".to_string(),
            formula_name: "cask:ghostty".to_string(),
            version: "1.3.1".to_string(),
            store_key: "abc".to_string(),
            kind: InstalledPackageKind::App,
            installed_at: 1,
        })
        .unwrap();

        let installed = db.list_installed().unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "cask:ghostty");
        assert_eq!(installed[0].kind, InstalledPackageKind::App);

        db.remove_installed("cask:ghostty").unwrap();
        assert!(db.list_installed().unwrap().is_empty());
    }
}
