use nous_shared::{NousError, Result};
use rusqlite::params;

use crate::db::MemoryDb;
use crate::types::Model;

impl MemoryDb {
    pub fn register_model(
        &self,
        name: &str,
        variant: Option<&str>,
        dimensions: i64,
        max_tokens: i64,
        chunk_size: i64,
        chunk_overlap: i64,
    ) -> Result<i64> {
        if dimensions <= 0 {
            return Err(NousError::Validation("dimensions must be positive".into()));
        }
        if max_tokens <= 0 {
            return Err(NousError::Validation("max_tokens must be positive".into()));
        }
        if chunk_size <= 0 {
            return Err(NousError::Validation("chunk_size must be positive".into()));
        }
        if chunk_overlap >= chunk_size {
            return Err(NousError::Validation(
                "chunk_overlap must be less than chunk_size".into(),
            ));
        }

        self.connection().execute(
            "INSERT INTO models (name, dimensions, max_tokens, variant, chunk_size, chunk_overlap, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![name, dimensions, max_tokens, variant, chunk_size, chunk_overlap],
        )?;
        Ok(self.connection().last_insert_rowid())
    }

    pub fn activate_model(&self, id: i64) -> Result<()> {
        let tx = self.connection().unchecked_transaction()?;
        tx.execute("UPDATE models SET active = 0", [])?;
        let changed = tx.execute("UPDATE models SET active = 1 WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(NousError::Validation(format!("model id {id} not found")));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn active_model(&self) -> Result<Option<Model>> {
        match self.connection().query_row(
            "SELECT id, name, dimensions, max_tokens, variant, chunk_size, chunk_overlap, active, created_at
             FROM models WHERE active = 1",
            [],
            Self::row_to_model,
        ) {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn deactivate_model(&self, id: i64) -> Result<()> {
        self.connection()
            .execute("UPDATE models SET active = 0 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_models(&self) -> Result<Vec<Model>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, name, dimensions, max_tokens, variant, chunk_size, chunk_overlap, active, created_at
             FROM models ORDER BY id",
        )?;
        let models = stmt
            .query_map([], Self::row_to_model)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(models)
    }

    pub fn register_and_activate_model(
        &self,
        name: &str,
        variant: Option<&str>,
        dimensions: i64,
        max_tokens: i64,
        chunk_size: i64,
        chunk_overlap: i64,
    ) -> Result<i64> {
        let existing: Option<i64> = match self.connection().query_row(
            "SELECT id FROM models WHERE name = ?1",
            params![name],
            |row| row.get(0),
        ) {
            Ok(id) => Some(id),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.into()),
        };

        let id = match existing {
            Some(id) => id,
            None => self.register_model(
                name,
                variant,
                dimensions,
                max_tokens,
                chunk_size,
                chunk_overlap,
            )?,
        };
        self.activate_model(id)?;
        Ok(id)
    }

    pub(crate) fn row_to_model(row: &rusqlite::Row<'_>) -> rusqlite::Result<Model> {
        Ok(Model {
            id: row.get(0)?,
            name: row.get(1)?,
            dimensions: row.get(2)?,
            max_tokens: row.get(3)?,
            variant: row.get(4)?,
            chunk_size: row.get(5)?,
            chunk_overlap: row.get(6)?,
            active: row.get::<_, i64>(7)? != 0,
            created_at: row.get(8)?,
        })
    }
}
