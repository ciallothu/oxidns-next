// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Persistent authentication state for the management API.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::infra::error::{DnsError, Result};

const AUTH_SCHEMA_VERSION: i64 = 2;

#[derive(Clone)]
pub(crate) struct UserRecord {
    pub id: String,
    pub webauthn_user_id: String,
    pub username: String,
    pub password_hash: String,
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
}

#[derive(Clone)]
pub(crate) struct SessionRecord {
    pub id: String,
    pub user: UserRecord,
    pub csrf_token: String,
    pub auth_method: String,
}

#[derive(Clone)]
pub(crate) struct PasskeyRecord {
    pub id: String,
    pub name: String,
    pub credential_json: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Clone)]
pub(crate) struct OidcIdentityRecord {
    pub issuer: String,
    pub subject: String,
    pub display_name: Option<String>,
}

pub(crate) struct AuthStore {
    connection: Connection,
}

impl AuthStore {
    pub(crate) fn open(path: &str) -> Result<Self> {
        if let Some(parent) = Path::new(path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        restrict_auth_database_permissions(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        ensure_supported_auth_schema_version(&connection)?;
        connection.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS auth_users (
                id TEXT PRIMARY KEY,
                webauthn_user_id TEXT NOT NULL UNIQUE,
                username TEXT NOT NULL COLLATE NOCASE UNIQUE,
                password_hash TEXT NOT NULL,
                totp_secret TEXT,
                totp_enabled INTEGER NOT NULL DEFAULT 0,
                totp_last_counter INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_login_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS auth_sessions (
                id TEXT PRIMARY KEY,
                token_hash BLOB NOT NULL UNIQUE,
                user_id TEXT NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
                csrf_token TEXT NOT NULL,
                auth_method TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_seen_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_auth_sessions_user ON auth_sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_auth_sessions_expiry ON auth_sessions(expires_at);
            CREATE TABLE IF NOT EXISTS auth_passkeys (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                credential_id BLOB NOT NULL UNIQUE,
                credential_json TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_used_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_auth_passkeys_user ON auth_passkeys(user_id);
            CREATE TABLE IF NOT EXISTS auth_oidc_identities (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
                issuer TEXT NOT NULL,
                subject TEXT NOT NULL,
                display_name TEXT,
                created_at INTEGER NOT NULL,
                last_login_at INTEGER,
                UNIQUE(issuer, subject)
            );
            CREATE INDEX IF NOT EXISTS idx_auth_oidc_user ON auth_oidc_identities(user_id);
            CREATE TABLE IF NOT EXISTS auth_recovery_codes (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
                code_hash BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                used_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_auth_recovery_user ON auth_recovery_codes(user_id);
            "#,
        )?;
        migrate_auth_schema(&connection)?;
        // Re-apply after WAL/schema initialization to cover sidecar files that
        // SQLite may have created while opening the store.
        restrict_auth_database_permissions(path)?;
        Ok(Self { connection })
    }

    pub(crate) fn user_count(&self) -> Result<u64> {
        let count = self
            .connection
            .query_row("SELECT COUNT(*) FROM auth_users", [], |row| {
                row.get::<_, i64>(0)
            })?;
        Ok(count.max(0) as u64)
    }

    pub(crate) fn create_user(
        &self,
        id: &str,
        webauthn_user_id: &str,
        username: &str,
        password_hash: &str,
        now: i64,
    ) -> Result<UserRecord> {
        self.connection.execute(
            "INSERT INTO auth_users (id, webauthn_user_id, username, password_hash, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, webauthn_user_id, username, password_hash, now],
        )?;
        self.user_by_id(id)?
            .ok_or_else(|| DnsError::runtime("new authentication user disappeared"))
    }

    pub(crate) fn user_by_username(&self, username: &str) -> Result<Option<UserRecord>> {
        self.connection
            .query_row(
                "SELECT id, webauthn_user_id, username, password_hash, totp_secret, totp_enabled FROM auth_users WHERE username = ?1 COLLATE NOCASE",
                [username],
                map_user,
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn user_by_id(&self, id: &str) -> Result<Option<UserRecord>> {
        self.connection
            .query_row(
                "SELECT id, webauthn_user_id, username, password_hash, totp_secret, totp_enabled FROM auth_users WHERE id = ?1",
                [id],
                map_user,
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn set_password_hash(&self, user_id: &str, hash: &str, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE auth_users SET password_hash = ?2, updated_at = ?3 WHERE id = ?1",
            params![user_id, hash, now],
        )?;
        Ok(())
    }

    pub(crate) fn set_totp(
        &mut self,
        user_id: &str,
        secret: &str,
        accepted_counter: i64,
        recovery_codes: &[(String, Vec<u8>)],
        now: i64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE auth_users SET totp_secret = ?2, totp_enabled = 1, totp_last_counter = ?3, updated_at = ?4 WHERE id = ?1",
            params![user_id, secret, accepted_counter, now],
        )?;
        tx.execute(
            "DELETE FROM auth_recovery_codes WHERE user_id = ?1",
            [user_id],
        )?;
        for (id, hash) in recovery_codes {
            tx.execute(
                "INSERT INTO auth_recovery_codes (id, user_id, code_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, user_id, hash, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn clear_totp(&mut self, user_id: &str, now: i64) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE auth_users SET totp_secret = NULL, totp_enabled = 0, totp_last_counter = NULL, updated_at = ?2 WHERE id = ?1",
            params![user_id, now],
        )?;
        tx.execute(
            "DELETE FROM auth_recovery_codes WHERE user_id = ?1",
            [user_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Atomically accepts only a TOTP counter newer than the last one used by
    /// this account. This prevents replay across concurrent requests and
    /// process restarts while still allowing the configured clock-skew window.
    pub(crate) fn consume_totp_counter(&self, user_id: &str, counter: i64) -> Result<bool> {
        Ok(self.connection.execute(
            r#"UPDATE auth_users
               SET totp_last_counter = ?2
               WHERE id = ?1
                 AND totp_enabled = 1
                 AND (totp_last_counter IS NULL OR totp_last_counter < ?2)"#,
            params![user_id, counter],
        )? != 0)
    }

    pub(crate) fn consume_recovery_code(
        &mut self,
        user_id: &str,
        code_hash: &[u8],
        now: i64,
    ) -> Result<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = tx
            .query_row(
                "SELECT id FROM auth_recovery_codes WHERE user_id = ?1 AND code_hash = ?2 AND used_at IS NULL LIMIT 1",
                params![user_id, code_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(id) = id else {
            tx.commit()?;
            return Ok(false);
        };
        tx.execute(
            "UPDATE auth_recovery_codes SET used_at = ?2 WHERE id = ?1 AND used_at IS NULL",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn create_session(
        &self,
        id: &str,
        token_hash: &[u8],
        user_id: &str,
        csrf_token: &str,
        auth_method: &str,
        now: i64,
        expires_at: i64,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO auth_sessions (id, token_hash, user_id, csrf_token, auth_method, created_at, last_seen_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
            params![id, token_hash, user_id, csrf_token, auth_method, now, expires_at],
        )?;
        self.connection.execute(
            "UPDATE auth_users SET last_login_at = ?2 WHERE id = ?1",
            params![user_id, now],
        )?;
        Ok(())
    }

    pub(crate) fn session_by_token_hash(
        &self,
        token_hash: &[u8],
        now: i64,
    ) -> Result<Option<SessionRecord>> {
        let record = self
            .connection
            .query_row(
                r#"SELECT s.id, s.csrf_token, s.auth_method,
                          u.id, u.webauthn_user_id, u.username, u.password_hash,
                          u.totp_secret, u.totp_enabled
                   FROM auth_sessions s
                   JOIN auth_users u ON u.id = s.user_id
                   WHERE s.token_hash = ?1 AND s.revoked_at IS NULL AND s.expires_at > ?2"#,
                params![token_hash, now],
                |row| {
                    Ok(SessionRecord {
                        id: row.get(0)?,
                        csrf_token: row.get(1)?,
                        auth_method: row.get(2)?,
                        user: UserRecord {
                            id: row.get(3)?,
                            webauthn_user_id: row.get(4)?,
                            username: row.get(5)?,
                            password_hash: row.get(6)?,
                            totp_secret: row.get(7)?,
                            totp_enabled: row.get::<_, i64>(8)? != 0,
                        },
                    })
                },
            )
            .optional()?;
        if let Some(record) = &record {
            self.connection.execute(
                "UPDATE auth_sessions SET last_seen_at = ?2 WHERE id = ?1",
                params![record.id, now],
            )?;
        }
        Ok(record)
    }

    pub(crate) fn revoke_session(&self, session_id: &str, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE auth_sessions SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            params![session_id, now],
        )?;
        Ok(())
    }

    pub(crate) fn revoke_other_sessions(
        &self,
        user_id: &str,
        current_session_id: &str,
        now: i64,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE auth_sessions SET revoked_at = ?3 WHERE user_id = ?1 AND id <> ?2 AND revoked_at IS NULL",
            params![user_id, current_session_id, now],
        )?;
        Ok(())
    }

    pub(crate) fn purge_expired_sessions(&self, now: i64) -> Result<()> {
        self.connection.execute(
            "DELETE FROM auth_sessions WHERE expires_at <= ?1 OR revoked_at IS NOT NULL",
            [now],
        )?;
        Ok(())
    }

    pub(crate) fn passkeys_for_user(&self, user_id: &str) -> Result<Vec<PasskeyRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, credential_json, created_at, last_used_at FROM auth_passkeys WHERE user_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map([user_id], |row| {
            Ok(PasskeyRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                credential_json: row.get(2)?,
                created_at: row.get(3)?,
                last_used_at: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub(crate) fn insert_passkey(
        &self,
        id: &str,
        user_id: &str,
        name: &str,
        credential_id: &[u8],
        credential_json: &str,
        now: i64,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO auth_passkeys (id, user_id, name, credential_id, credential_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, user_id, name, credential_id, credential_json, now],
        )?;
        Ok(())
    }

    pub(crate) fn update_passkey_credential(
        &self,
        user_id: &str,
        credential_id: &[u8],
        expected_credential_json: &str,
        credential_json: &str,
        now: i64,
    ) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE auth_passkeys SET credential_json = ?4, last_used_at = ?5 WHERE user_id = ?1 AND credential_id = ?2 AND credential_json = ?3",
            params![user_id, credential_id, expected_credential_json, credential_json, now],
        )? != 0)
    }

    pub(crate) fn rename_passkey(&self, user_id: &str, id: &str, name: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE auth_passkeys SET name = ?3 WHERE id = ?2 AND user_id = ?1",
            params![user_id, id, name],
        )? != 0)
    }

    pub(crate) fn delete_passkey(&self, user_id: &str, id: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM auth_passkeys WHERE id = ?2 AND user_id = ?1",
            params![user_id, id],
        )? != 0)
    }

    pub(crate) fn user_for_oidc_identity(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<UserRecord>> {
        self.connection
            .query_row(
                r#"SELECT u.id, u.webauthn_user_id, u.username, u.password_hash, u.totp_secret, u.totp_enabled
                   FROM auth_oidc_identities i JOIN auth_users u ON u.id = i.user_id
                   WHERE i.issuer = ?1 AND i.subject = ?2"#,
                params![issuer, subject],
                map_user,
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn link_oidc_identity(
        &self,
        id: &str,
        user_id: &str,
        issuer: &str,
        subject: &str,
        display_name: Option<&str>,
        now: i64,
    ) -> Result<()> {
        let changed = self.connection.execute(
            r#"INSERT INTO auth_oidc_identities (id, user_id, issuer, subject, display_name, created_at, last_login_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
               ON CONFLICT(issuer, subject) DO UPDATE
               SET last_login_at = excluded.last_login_at, display_name = excluded.display_name
               WHERE auth_oidc_identities.user_id = excluded.user_id"#,
            params![id, user_id, issuer, subject, display_name, now],
        )?;
        if changed == 0 {
            return Err(DnsError::runtime(
                "OIDC identity is already linked to a different local account",
            ));
        }
        Ok(())
    }

    pub(crate) fn oidc_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<OidcIdentityRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT issuer, subject, display_name FROM auth_oidc_identities WHERE user_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map([user_id], |row| {
            Ok(OidcIdentityRecord {
                issuer: row.get(0)?,
                subject: row.get(1)?,
                display_name: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

fn migrate_auth_schema(connection: &Connection) -> Result<()> {
    ensure_supported_auth_schema_version(connection)?;
    let mut statement = connection.prepare("PRAGMA table_info(auth_users)")?;
    let mut rows = statement.query([])?;
    let mut has_totp_last_counter = false;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == "totp_last_counter" {
            has_totp_last_counter = true;
            break;
        }
    }
    drop(rows);
    drop(statement);

    if !has_totp_last_counter {
        connection.execute(
            "ALTER TABLE auth_users ADD COLUMN totp_last_counter INTEGER",
            [],
        )?;
    }
    connection.pragma_update(None, "user_version", AUTH_SCHEMA_VERSION)?;
    Ok(())
}

fn ensure_supported_auth_schema_version(connection: &Connection) -> Result<()> {
    let current_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current_version > AUTH_SCHEMA_VERSION {
        return Err(DnsError::runtime(format!(
            "authentication database schema version {current_version} is newer than supported version {AUTH_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_auth_database_permissions(path: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if path == ":memory:" {
        return Ok(());
    }
    for candidate in [
        path.to_string(),
        format!("{path}-wal"),
        format!("{path}-shm"),
    ] {
        let candidate = Path::new(&candidate);
        if candidate.exists() {
            std::fs::set_permissions(candidate, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_auth_database_permissions(_path: &str) -> std::io::Result<()> {
    Ok(())
}

fn map_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserRecord> {
    Ok(UserRecord {
        id: row.get(0)?,
        webauthn_user_id: row.get(1)?,
        username: row.get(2)?,
        password_hash: row.get(3)?,
        totp_secret: row.get(4)?,
        totp_enabled: row.get::<_, i64>(5)? != 0,
    })
}

#[cfg(test)]
mod totp_replay_tests {
    use super::*;

    #[test]
    fn totp_counter_is_monotonic_and_persists_across_reopen() {
        let directory = tempfile::tempdir().expect("temporary auth directory");
        let path = directory.path().join("auth.db");
        let path_text = path.to_str().expect("UTF-8 database path");

        {
            let mut store = AuthStore::open(path_text).expect("open auth database");
            store
                .create_user("user", "webauthn-user", "admin", "password-hash", 1)
                .expect("create user");
            store
                .set_totp("user", "secret", 100, &[], 2)
                .expect("enable TOTP");
            assert!(!store.consume_totp_counter("user", 100).unwrap());
            assert!(!store.consume_totp_counter("user", 99).unwrap());
            assert!(store.consume_totp_counter("user", 101).unwrap());
            assert!(!store.consume_totp_counter("user", 101).unwrap());
        }

        let store = AuthStore::open(path_text).expect("reopen auth database");
        assert!(!store.consume_totp_counter("user", 101).unwrap());
        assert!(store.consume_totp_counter("user", 102).unwrap());
    }

    #[test]
    fn opening_version_one_database_adds_totp_replay_column() {
        let directory = tempfile::tempdir().expect("temporary auth directory");
        let path = directory.path().join("auth.db");
        let connection = Connection::open(&path).expect("create legacy database");
        connection
            .execute_batch(
                r#"
                CREATE TABLE auth_users (
                    id TEXT PRIMARY KEY,
                    webauthn_user_id TEXT NOT NULL UNIQUE,
                    username TEXT NOT NULL COLLATE NOCASE UNIQUE,
                    password_hash TEXT NOT NULL,
                    totp_secret TEXT,
                    totp_enabled INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    last_login_at INTEGER
                );
                PRAGMA user_version = 1;
                "#,
            )
            .expect("create legacy schema");
        drop(connection);

        let store = AuthStore::open(path.to_str().expect("UTF-8 database path"))
            .expect("migrate auth database");
        store
            .connection
            .prepare("SELECT totp_last_counter FROM auth_users")
            .expect("replay counter column");
        let version: i64 = store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, 2);
    }

    #[test]
    fn passkey_update_requires_the_current_owner_and_credential_version() {
        let store = AuthStore::open(":memory:").expect("open auth database");
        store
            .create_user("user", "webauthn-user", "admin", "password-hash", 1)
            .expect("create user");
        store
            .insert_passkey("key", "user", "Laptop", b"credential", "old", 1)
            .expect("insert passkey");

        assert!(
            store
                .update_passkey_credential("user", b"credential", "old", "new", 2)
                .expect("update current credential")
        );
        assert!(
            !store
                .update_passkey_credential("user", b"credential", "old", "stale", 3)
                .expect("reject stale credential")
        );
        assert!(
            !store
                .update_passkey_credential("other", b"credential", "new", "foreign", 3)
                .expect("reject a different owner")
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn opening_auth_database_restricts_existing_file_permissions() {
        let directory = tempfile::tempdir().expect("temporary auth directory");
        let path = directory.path().join("auth.db");
        std::fs::write(&path, []).expect("create database file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("set deliberately broad permissions");

        let _store = AuthStore::open(path.to_str().expect("UTF-8 database path"))
            .expect("open auth database");
        let mode = std::fs::metadata(path)
            .expect("database metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn permission_helper_restricts_database_sidecars() {
        let directory = tempfile::tempdir().expect("temporary auth directory");
        let path = directory.path().join("auth.db");
        let path_text = path.to_str().expect("UTF-8 database path");
        let candidates = [
            path.clone(),
            std::path::PathBuf::from(format!("{path_text}-wal")),
            std::path::PathBuf::from(format!("{path_text}-shm")),
        ];
        for candidate in &candidates {
            std::fs::write(candidate, []).expect("create database file");
            std::fs::set_permissions(candidate, std::fs::Permissions::from_mode(0o644))
                .expect("set deliberately broad permissions");
        }

        restrict_auth_database_permissions(path_text).expect("restrict auth files");

        for candidate in candidates {
            let mode = std::fs::metadata(candidate)
                .expect("database metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
