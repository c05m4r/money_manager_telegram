// Copyright (C) 2026 Marcos Gabriel Miller
use std::{fmt, path::Path, str::FromStr};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use uuid::Uuid;

#[derive(Clone, PartialEq)]
pub struct SessionRow {
    pub telegram_user_id: i64,
    pub chat_id: i64,
    pub user_uuid: Uuid,
    pub username: String,
    pub role: String,
    pub jwt: Option<String>,
    pub jwt_expires_at: Option<i64>,
    pub credentials: Option<Vec<u8>>,
}

impl fmt::Debug for SessionRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionRow")
            .field("telegram_user_id", &self.telegram_user_id)
            .field("user_uuid", &self.user_uuid)
            .field("username", &self.username)
            .field("role", &self.role)
            .field("jwt", &self.jwt.as_ref().map(|_| "***"))
            .field("jwt_expires_at", &self.jwt_expires_at)
            .field("credentials", &self.credentials.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Bot-owned SQLite tables (`migrations/`). All writes are keyed by Telegram user id.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Store {
    /// Opens (creating if needed) the database file with `0600` permissions and runs migrations.
    pub async fn open(path: &Path) -> Result<Self, sqlx::Error> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        restrict_permissions(path)?;
        Self::from_pool(pool).await
    }

    pub async fn from_pool(pool: SqlitePool) -> Result<Self, sqlx::Error> {
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    // ---------- sessions ----------

    pub async fn get_session(
        &self,
        telegram_user_id: i64,
    ) -> Result<Option<SessionRow>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT telegram_user_id, chat_id, user_uuid, username, role, jwt, jwt_expires_at, credentials
             FROM sessions WHERE telegram_user_id = ?",
        )
        .bind(telegram_user_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            let user_uuid: String = row.try_get("user_uuid")?;
            Ok(SessionRow {
                telegram_user_id: row.try_get("telegram_user_id")?,
                chat_id: row.try_get("chat_id")?,
                user_uuid: Uuid::parse_str(&user_uuid)
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
                username: row.try_get("username")?,
                role: row.try_get("role")?,
                jwt: row.try_get("jwt")?,
                jwt_expires_at: row.try_get("jwt_expires_at")?,
                credentials: row.try_get("credentials")?,
            })
        })
        .transpose()
    }

    pub async fn upsert_session(&self, session: &SessionRow) -> Result<(), sqlx::Error> {
        let now = now();
        sqlx::query(
            "INSERT INTO sessions
                 (telegram_user_id, chat_id, user_uuid, username, role, jwt, jwt_expires_at, credentials, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(telegram_user_id) DO UPDATE SET
                 chat_id = excluded.chat_id, user_uuid = excluded.user_uuid, username = excluded.username,
                 role = excluded.role, jwt = excluded.jwt, jwt_expires_at = excluded.jwt_expires_at,
                 credentials = excluded.credentials, updated_at = excluded.updated_at",
        )
        .bind(session.telegram_user_id)
        .bind(session.chat_id)
        .bind(session.user_uuid.to_string())
        .bind(&session.username)
        .bind(&session.role)
        .bind(&session.jwt)
        .bind(session.jwt_expires_at)
        .bind(&session.credentials)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_token(
        &self,
        telegram_user_id: i64,
        jwt: &str,
        jwt_expires_at: i64,
        role: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE sessions SET jwt = ?, jwt_expires_at = ?, role = ?, updated_at = ? WHERE telegram_user_id = ?")
            .bind(jwt)
            .bind(jwt_expires_at)
            .bind(role)
            .bind(now())
            .bind(telegram_user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Forces the next call to fetch a new token (e.g. after a 401).
    pub async fn expire_token(&self, telegram_user_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE sessions SET jwt_expires_at = 0, updated_at = ? WHERE telegram_user_id = ?",
        )
        .bind(now())
        .bind(telegram_user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Removes the session and everything the bot keeps for that user.
    pub async fn delete_session(&self, telegram_user_id: i64) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        for table in ["sessions", "short_refs", "list_state"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE telegram_user_id = ?"))
                .bind(telegram_user_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }

    // ---------- short refs ----------

    pub async fn set_refs(
        &self,
        telegram_user_id: i64,
        kind: &str,
        targets: &[String],
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM short_refs WHERE telegram_user_id = ? AND kind = ?")
            .bind(telegram_user_id)
            .bind(kind)
            .execute(&mut *tx)
            .await?;
        for (index, target) in targets.iter().enumerate() {
            sqlx::query("INSERT INTO short_refs (telegram_user_id, kind, position, target) VALUES (?, ?, ?, ?)")
                .bind(telegram_user_id)
                .bind(kind)
                .bind(index as i64 + 1)
                .bind(target)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }

    pub async fn get_ref(
        &self,
        telegram_user_id: i64,
        kind: &str,
        position: i64,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT target FROM short_refs WHERE telegram_user_id = ? AND kind = ? AND position = ?")
            .bind(telegram_user_id)
            .bind(kind)
            .bind(position)
            .fetch_optional(&self.pool)
            .await
    }

    // ---------- list state ----------

    pub async fn set_list_state(
        &self,
        telegram_user_id: i64,
        kind: &str,
        filters_json: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO list_state (telegram_user_id, kind, filters_json) VALUES (?, ?, ?)
             ON CONFLICT(telegram_user_id, kind) DO UPDATE SET filters_json = excluded.filters_json",
        )
        .bind(telegram_user_id)
        .bind(kind)
        .bind(filters_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_list_state(
        &self,
        telegram_user_id: i64,
        kind: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT filters_json FROM list_state WHERE telegram_user_id = ? AND kind = ?",
        )
        .bind(telegram_user_id)
        .bind(kind)
        .fetch_optional(&self.pool)
        .await
    }

    // ---------- login failures ----------

    pub async fn record_login_failure(&self, telegram_user_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO login_failures (telegram_user_id, failed_at) VALUES (?, ?)")
            .bind(telegram_user_id)
            .bind(now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Failures since `since` (unix seconds), and the oldest one, after pruning older rows.
    pub async fn login_failures_since(
        &self,
        telegram_user_id: i64,
        since: i64,
    ) -> Result<(i64, Option<i64>), sqlx::Error> {
        sqlx::query("DELETE FROM login_failures WHERE failed_at < ?")
            .bind(since)
            .execute(&self.pool)
            .await?;
        let row = sqlx::query("SELECT COUNT(*) AS n, MIN(failed_at) AS oldest FROM login_failures WHERE telegram_user_id = ?")
            .bind(telegram_user_id)
            .fetch_one(&self.pool)
            .await?;
        Ok((row.try_get("n")?, row.try_get("oldest")?))
    }

    pub async fn clear_login_failures(&self, telegram_user_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM login_failures WHERE telegram_user_id = ?")
            .bind(telegram_user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
pub(crate) async fn memory_store() -> Store {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    Store::from_pool(pool).await.unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(jwt: Option<&str>) -> SessionRow {
        SessionRow {
            telegram_user_id: 42,
            chat_id: 42,
            user_uuid: Uuid::nil(),
            username: "default".into(),
            role: "user".into(),
            jwt: jwt.map(str::to_string),
            jwt_expires_at: Some(100),
            credentials: Some(vec![1, 2, 3]),
        }
    }

    #[tokio::test]
    async fn session_lifecycle() {
        let store = memory_store().await;
        assert!(store.get_session(42).await.unwrap().is_none());

        store.upsert_session(&session(Some("a"))).await.unwrap();
        assert_eq!(
            store.get_session(42).await.unwrap(),
            Some(session(Some("a")))
        );

        store.update_token(42, "b", 200, "manager").await.unwrap();
        let updated = store.get_session(42).await.unwrap().unwrap();
        assert_eq!(
            (
                updated.jwt.as_deref(),
                updated.jwt_expires_at,
                updated.role.as_str()
            ),
            (Some("b"), Some(200), "manager")
        );

        store.expire_token(42).await.unwrap();
        assert_eq!(
            store.get_session(42).await.unwrap().unwrap().jwt_expires_at,
            Some(0)
        );

        store.set_refs(42, "tx", &["x".into()]).await.unwrap();
        store.delete_session(42).await.unwrap();
        assert!(store.get_session(42).await.unwrap().is_none());
        assert!(store.get_ref(42, "tx", 1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn refs_are_replaced_per_kind() {
        let store = memory_store().await;
        store
            .set_refs(42, "tx", &["a".into(), "b".into()])
            .await
            .unwrap();
        store.set_refs(42, "account", &["z".into()]).await.unwrap();
        assert_eq!(
            store.get_ref(42, "tx", 2).await.unwrap().as_deref(),
            Some("b")
        );

        store.set_refs(42, "tx", &["c".into()]).await.unwrap();
        assert_eq!(
            store.get_ref(42, "tx", 1).await.unwrap().as_deref(),
            Some("c")
        );
        assert!(store.get_ref(42, "tx", 2).await.unwrap().is_none());
        assert_eq!(
            store.get_ref(42, "account", 1).await.unwrap().as_deref(),
            Some("z")
        );
    }

    #[tokio::test]
    async fn list_state_and_login_failures() {
        let store = memory_store().await;
        store.set_list_state(42, "tx", "{}").await.unwrap();
        store.set_list_state(42, "tx", "{\"a\":1}").await.unwrap();
        assert_eq!(
            store.get_list_state(42, "tx").await.unwrap().as_deref(),
            Some("{\"a\":1}")
        );

        store.record_login_failure(42).await.unwrap();
        store.record_login_failure(42).await.unwrap();
        store.record_login_failure(7).await.unwrap();
        let (count, oldest) = store.login_failures_since(42, now() - 60).await.unwrap();
        assert_eq!(count, 2);
        assert!(oldest.is_some());
        assert_eq!(
            store.login_failures_since(42, now() + 60).await.unwrap().0,
            0
        );

        store.clear_login_failures(7).await.unwrap();
        assert_eq!(store.login_failures_since(7, 0).await.unwrap().0, 0);
    }
}
