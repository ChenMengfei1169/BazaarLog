// Tamper-evident audit chain. Every audit entry stores the hash of the previous
// entry (prev_hash) plus its own content hash (entry_hash), forming a chain
// anchored at a fixed genesis value. Deleting, inserting, or editing any row
// breaks every subsequent link, so tampering is detectable. Audited mutations
// are serialized by AppState.mutation_guard before the anchor is read, so the
// chain cannot fork under concurrent writers.
//
// Scope note: an attacker with raw write access to the database file can still
// rewrite the whole chain and the file ACL is the real boundary. The chain's
// value is that partial or silent tampering becomes detectable and that a
// separate, externally anchored copy, such as a periodic export, can be
// compared.
use sha2::{Digest, Sha256};
use sqlx::AnyPool;
use sqlx::Row;

use crate::encoding::encode_hex;
use crate::error::AppResult;
use crate::handlers::{optional_i64, optional_string};

// Fixed anchor for the first entry in the chain.
pub const AUDIT_CHAIN_GENESIS: &str = "bazaarlog-audit-chain-v1";

/// Hash value recorded for the "no audit rows yet" anchor. Without a distinct
/// value for the empty state, a missing seal file would be indistinguishable
/// from a legitimately empty log, and deleting the seal would silently disable
/// truncation detection.
pub const AUDIT_SEAL_EMPTY: &str = "bazaarlog-audit-seal-empty-v1";

/// The content of one audit row that participates in the chain hash.
/// [`compute_entry_hash`] callers and [`verify_chain`] must build identical
/// values so the recomputed hash matches.
pub struct AuditChainRow<'a> {
    pub id: i64,
    pub class_id: i64,
    pub transaction_id: Option<i64>,
    pub action: &'a str,
    pub operator: &'a str,
    pub payload_before: Option<&'a str>,
    pub payload_after: Option<&'a str>,
    pub occurred_at: &'a str,
}

/// Computes the content hash of one audit row. The audit writer and
/// [`verify_chain`] must build the exact same input string.
pub fn compute_entry_hash(prev_hash: &str, row: &AuditChainRow) -> String {
    let transaction_id_part = row
        .transaction_id
        .map(|value| value.to_string())
        .unwrap_or_default();
    let before_part = row.payload_before.unwrap_or("");
    let after_part = row.payload_after.unwrap_or("");
    let digest_input = format!(
        "{prev_hash}|{id}|{class_id}|{transaction_id_part}|{action}|{operator}|{before_part}|{after_part}|{occurred_at}",
        id = row.id,
        class_id = row.class_id,
        action = row.action,
        operator = row.operator,
        occurred_at = row.occurred_at,
    );
    encode_hex(&Sha256::digest(digest_input.as_bytes()))
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AuditChainReport {
    pub verified: bool,
    // Id of the first row whose link does not match, when verification fails.
    pub first_broken_id: Option<i64>,
    // True when the database's newest entry_hash disagrees with the external
    // seal file, which means the tail of the audit log was deleted or rolled
    // back after the seal was written.
    pub truncated: bool,
}

/// Walks the whole audit table in id order and checks every link.
pub async fn verify_chain(pool: &AnyPool) -> AppResult<AuditChainReport> {
    let rows = sqlx::query(
        "SELECT id, prev_hash, entry_hash, class_id, transaction_id, action, operator, \
         payload_before, payload_after, occurred_at FROM audit_logs ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let mut expected_prev = AUDIT_CHAIN_GENESIS.to_string();
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let prev_hash: String = row.try_get("prev_hash")?;
        let entry_hash: String = row.try_get("entry_hash")?;
        let class_id: i64 = row.try_get("class_id")?;
        let transaction_id = optional_i64(&row, "transaction_id")?;
        let action: String = row.try_get("action")?;
        let operator: String = row.try_get("operator")?;
        let payload_before = optional_string(&row, "payload_before")?;
        let payload_after = optional_string(&row, "payload_after")?;
        let occurred_at: String = row.try_get("occurred_at")?;
        let recomputed = compute_entry_hash(
            &prev_hash,
            &AuditChainRow {
                id,
                class_id,
                transaction_id,
                action: &action,
                operator: &operator,
                payload_before: payload_before.as_deref(),
                payload_after: payload_after.as_deref(),
                occurred_at: &occurred_at,
            },
        );
        if prev_hash != expected_prev || recomputed != entry_hash {
            return Ok(AuditChainReport {
                verified: false,
                first_broken_id: Some(id),
                truncated: false,
            });
        }
        expected_prev = entry_hash;
    }
    Ok(AuditChainReport {
        verified: true,
        first_broken_id: None,
        truncated: false,
    })
}

/// Backfills the chain for audit rows that predate the chain columns. Runs at
/// startup after the columns are added so existing databases get a coherent
/// chain anchored at the earliest unhashed row.
pub async fn backfill_chain(pool: &AnyPool) -> AppResult<()> {
    // Anchor: the newest row that already carries a chain hash.
    let anchor_row = sqlx::query(
        "SELECT id, entry_hash FROM audit_logs WHERE entry_hash <> '' \
         ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let (anchor_id, mut expected_prev) = match anchor_row {
        Some(row) => (
            row.try_get::<i64, _>("id")?,
            row.try_get::<String, _>("entry_hash")?,
        ),
        None => (0, AUDIT_CHAIN_GENESIS.to_string()),
    };
    let rows = sqlx::query(
        "SELECT id, class_id, transaction_id, action, operator, payload_before, \
         payload_after, occurred_at FROM audit_logs \
         WHERE id > ? AND entry_hash = '' ORDER BY id",
    )
    .bind(anchor_id)
    .fetch_all(pool)
    .await?;
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let class_id: i64 = row.try_get("class_id")?;
        let transaction_id = optional_i64(&row, "transaction_id")?;
        let action: String = row.try_get("action")?;
        let operator: String = row.try_get("operator")?;
        let payload_before = optional_string(&row, "payload_before")?;
        let payload_after = optional_string(&row, "payload_after")?;
        let occurred_at: String = row.try_get("occurred_at")?;
        let entry_hash = compute_entry_hash(
            &expected_prev,
            &AuditChainRow {
                id,
                class_id,
                transaction_id,
                action: &action,
                operator: &operator,
                payload_before: payload_before.as_deref(),
                payload_after: payload_after.as_deref(),
                occurred_at: &occurred_at,
            },
        );
        sqlx::query("UPDATE audit_logs SET prev_hash = ?, entry_hash = ? WHERE id = ?")
            .bind(&expected_prev)
            .bind(&entry_hash)
            .bind(id)
            .execute(pool)
            .await?;
        expected_prev = entry_hash;
    }
    Ok(())
}

// --- External seal anchor ---
//
// A hash chain alone cannot detect truncation of the newest entries: deleting
// the last row removes the evidence with it. To close that gap, the newest
// entry_hash is mirrored into a sidecar seal file next to the SQLite database,
// for example bazaarlog.db.audit.seal. Verification compares the database's
// newest hash against the seal, so a tail deletion is flagged. The seal file
// lives in the same filesystem as the database, so it is not a hard boundary
// against an attacker with raw file access; it makes truncation detectable and
// gives a separate copy to compare manually, which is what the chain alone
// cannot do.

/// Path of the seal file next to a SQLite database; None for other backends.
pub fn seal_path_for_database(database_url: &str) -> Option<std::path::PathBuf> {
    if !database_url.starts_with("sqlite") {
        return None;
    }
    let raw_path = database_url
        .trim_start_matches("sqlite://")
        .split('?')
        .next()
        .unwrap_or(database_url);
    let mut seal_name = std::ffi::OsString::from(raw_path);
    seal_name.push(".audit.seal");
    Some(std::path::PathBuf::from(seal_name))
}

/// What the seal file records: the id of the newest audit row at the time of
/// writing, together with that row's entry_hash.
///
/// Recording the id, not just the hash, is what makes verification meaningful
/// across restarts. Comparing a bare hash against the database's newest hash
/// cannot tell "the log grew" from "the anchored row disappeared", so
/// re-anchoring at startup would launder a tail truncation into a fresh,
/// trusted anchor. Asking instead whether the anchored row still exists with the
/// same hash distinguishes the two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealAnchor {
    pub id: i64,
    pub hash: String,
}

impl SealAnchor {
    /// Anchor used when the audit log has no rows.
    pub fn empty() -> Self {
        Self {
            id: 0,
            hash: AUDIT_SEAL_EMPTY.to_string(),
        }
    }

    fn serialize(&self) -> String {
        format!("{} {}\n", self.id, self.hash)
    }

    fn parse(content: &str) -> Option<Self> {
        let mut parts = content.trim().splitn(2, ' ');
        let id = parts.next()?.parse::<i64>().ok()?;
        let hash = parts.next()?.trim().to_string();
        if hash.is_empty() {
            return None;
        }
        Some(Self { id, hash })
    }
}

// What we found on disk. "Missing" and "present but not in the current format"
// are kept apart: the latter is what an upgrade from the single-hash seal looks
// like, and it is safe to re-anchor once.
enum SealFileState {
    Missing,
    Legacy,
    Anchored(SealAnchor),
}

/// Writes the anchor through a fresh, unpredictable temporary file that is
/// created with `create_new`, so an existing file, including a symlink planted
/// by another local user, makes the open fail instead of being followed. The
/// temporary file is then renamed into place.
fn write_seal_file(seal_path: &std::path::Path, anchor: &SealAnchor) -> std::io::Result<()> {
    use std::io::Write;

    let mut temporary_name = seal_path.as_os_str().to_os_string();
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    temporary_name.push(format!(".{}.{unique_suffix}.tmp", std::process::id()));
    let temporary_path = std::path::PathBuf::from(temporary_name);

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)?;
    file.write_all(anchor.serialize().as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary_path, seal_path)?;
    // The rename installs a new file, so any ACL applied to the previous seal is
    // gone. Re-apply it here rather than only at startup, otherwise every
    // audited mutation would silently restore the directory's default
    // permissions on the seal.
    if !crate::db::harden_file_acl(seal_path) {
        tracing::warn!(
            path = %seal_path.display(),
            "could not tighten the ACL of the audit seal file"
        );
    }
    Ok(())
}

fn read_seal_file(seal_path: &std::path::Path) -> SealFileState {
    match std::fs::read_to_string(seal_path) {
        Ok(content) => match SealAnchor::parse(&content) {
            Some(anchor) => SealFileState::Anchored(anchor),
            None => SealFileState::Legacy,
        },
        // Unreadable is treated like missing: either way there is no anchor we
        // can trust, which is the fail-closed direction.
        Err(_) => SealFileState::Missing,
    }
}

/// The newest audit row's id and entry_hash, if the log has any rows.
pub async fn latest_entry(pool: &AnyPool) -> AppResult<Option<(i64, String)>> {
    let row = sqlx::query("SELECT id, entry_hash FROM audit_logs ORDER BY id DESC LIMIT 1")
        .fetch_optional(pool)
        .await?;
    match row {
        Some(row) => Ok(Some((row.try_get("id")?, row.try_get("entry_hash")?))),
        None => Ok(None),
    }
}

/// Whether the row a seal anchor points at is still present and unchanged.
///
/// The empty anchor, id 0, only asserts that the log had no rows when it was
/// written, so any rows now present are growth rather than tampering.
async fn anchored_entry_intact(pool: &AnyPool, anchor: &SealAnchor) -> AppResult<bool> {
    if anchor.id == 0 {
        return Ok(true);
    }
    let row = sqlx::query("SELECT entry_hash FROM audit_logs WHERE id = ?")
        .bind(anchor.id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some(row) => Ok(row.try_get::<String, _>("entry_hash")? == anchor.hash),
        None => Ok(false),
    }
}

/// Re-anchors the seal file to the newest audit row. Best effort: failures are
/// logged, never fatal. Called after every audited commit and at startup.
///
/// The seal is written even when the audit log is empty, so a missing seal file
/// later is unambiguously a deletion rather than "nothing recorded yet".
///
/// Crucially, the existing anchor is verified first. If the row it points at is
/// gone or has been rewritten, the seal is left untouched and an error is logged
/// rather than replaced, otherwise a restart would silently re-anchor a
/// truncated log and report it as healthy.
pub async fn refresh_seal(pool: &AnyPool, database_url: &str) -> AppResult<()> {
    let Some(seal_path) = seal_path_for_database(database_url) else {
        return Ok(());
    };
    let next_anchor = match latest_entry(pool).await? {
        Some((id, hash)) => SealAnchor { id, hash },
        None => SealAnchor::empty(),
    };
    match read_seal_file(&seal_path) {
        SealFileState::Anchored(existing) => {
            if anchored_entry_intact(pool, &existing).await? {
                if let Err(error) = write_seal_file(&seal_path, &next_anchor) {
                    tracing::warn!(error = %error, "failed to write audit seal file");
                }
            } else {
                tracing::error!(
                    sealed_id = existing.id,
                    "audit seal does not match the database: the anchored audit \
                     entry is missing or was rewritten. Refusing to re-anchor so \
                     the tampering stays visible - the audit history may have \
                     been truncated or edited outside the application."
                );
            }
        }
        SealFileState::Legacy => {
            tracing::info!("upgrading the audit seal file to the id+hash format");
            if let Err(error) = write_seal_file(&seal_path, &next_anchor) {
                tracing::warn!(error = %error, "failed to write audit seal file");
            }
        }
        // No anchor exists at all, either on the first run or because someone
        // deleted it. There is nothing to compare against, so a fresh anchor is
        // the only option.
        SealFileState::Missing => {
            if let Err(error) = write_seal_file(&seal_path, &next_anchor) {
                tracing::warn!(error = %error, "failed to write audit seal file");
            }
        }
    }
    Ok(())
}

/// Checks the seal file against the database. A false result means the anchor is
/// missing, unreadable, or points at an audit row that no longer exists or has
/// been rewritten, which means the tail of the log was deleted or rolled back.
///
/// Fails closed on every uncertain state, because those are exactly the states
/// an attacker who removed the anchor would produce.
pub async fn seal_verifies(pool: &AnyPool, seal_path: &std::path::Path) -> AppResult<bool> {
    match read_seal_file(seal_path) {
        SealFileState::Anchored(anchor) => anchored_entry_intact(pool, &anchor).await,
        SealFileState::Missing | SealFileState::Legacy => Ok(false),
    }
}
