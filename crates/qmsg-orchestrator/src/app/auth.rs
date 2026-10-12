use super::wire::{Credential, Device};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tonic::Status;

pub const PAIR_LIFETIME: u64 = 5 * 60 * 1000;
const TOKEN_LIFETIME: u64 = 90 * 24 * 60 * 60 * 1000;
pub const OP_LIFETIME: u64 = 7 * 24 * 60 * 60 * 1000;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn random() -> String {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).expect("OS random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn db_error(_: rusqlite::Error) -> Status {
    Status::internal("device database unavailable")
}

pub struct Auth {
    db: Mutex<Connection>,
}
pub enum Claim {
    New,
    Done(String),
    Unknown,
}
impl Auth {
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        crate::secrets::create_dir(dir)?;
        let path = dir.join("devices.db");
        // Create with owner-only permissions before SQLite writes any secrets.
        if !path.exists() {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&path)?;
        }
        crate::secrets::restrict(&path)?;
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS grants(hash TEXT PRIMARY KEY, expires INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS devices(id TEXT PRIMARY KEY, name TEXT NOT NULL, hash TEXT NOT NULL, expires INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS operations(device TEXT NOT NULL, id TEXT NOT NULL, hash TEXT NOT NULL, created INTEGER NOT NULL, result TEXT, PRIMARY KEY(device,id));")?;
        Ok(Self { db: Mutex::new(db) })
    }
    pub fn grant(&self) -> Result<(String, u64), Status> {
        let code = random();
        let expires = now() + PAIR_LIFETIME;
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(db_error)?;
        tx.execute("DELETE FROM grants WHERE expires<=?", [now() as i64])
            .map_err(db_error)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM grants", [], |r| r.get(0))
            .map_err(db_error)?;
        if count >= 8 {
            return Err(Status::resource_exhausted("too many pairing codes"));
        }
        tx.execute(
            "INSERT INTO grants VALUES(?,?)",
            params![hash(&code), expires as i64],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok((code, expires))
    }
    pub fn pair(&self, code: &str, name: &str) -> Result<Credential, Status> {
        if code.len() != 64
            || name.trim().is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
        {
            return Err(Status::invalid_argument(
                "invalid pairing code or device name",
            ));
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(db_error)?;
        let removed = tx
            .execute(
                "DELETE FROM grants WHERE hash=? AND expires>?",
                params![hash(code), now() as i64],
            )
            .map_err(db_error)?;
        if removed != 1 {
            return Err(Status::permission_denied(
                "pairing code expired or already used",
            ));
        }
        tx.execute("DELETE FROM devices WHERE expires<=?", [now() as i64])
            .map_err(db_error)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM devices", [], |r| r.get(0))
            .map_err(db_error)?;
        if count >= 128 {
            return Err(Status::resource_exhausted("device limit reached"));
        }
        let id = random();
        let token = format!("{id}.{}", random());
        let expires = now() + TOKEN_LIFETIME;
        tx.execute(
            "INSERT INTO devices VALUES(?,?,?,?)",
            params![id, name, hash(&token), expires as i64],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(Credential {
            device_id: id,
            token,
            expires_at_ms: expires,
        })
    }
    pub fn check(&self, token: &str) -> Result<String, Status> {
        if token.len() != 129 || !token.is_ascii() {
            return Err(Status::unauthenticated("invalid device token"));
        }
        let id = &token[..64];
        let valid: bool = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE id=? AND hash=? AND expires>?)",
                params![id, hash(token), now() as i64],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if valid {
            Ok(id.into())
        } else {
            Err(Status::unauthenticated("device token expired or revoked"))
        }
    }
    pub fn rotate(&self, token: &str, replacement_secret: &str) -> Result<Credential, Status> {
        let id = self.check(token)?;
        let secret = if replacement_secret.is_empty() {
            random()
        } else {
            if replacement_secret.len() != 64
                || !replacement_secret
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(Status::invalid_argument(
                    "replacement_secret must contain 32 random bytes in lowercase hex",
                ));
            }
            replacement_secret.into()
        };
        let replacement = format!("{id}.{secret}");
        let expires = now() + TOKEN_LIFETIME;
        let changed = self
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE devices SET hash=?,expires=? WHERE id=? AND hash=? AND expires>?",
                params![
                    hash(&replacement),
                    expires as i64,
                    id,
                    hash(token),
                    now() as i64
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(Status::unauthenticated("device token changed"));
        }
        Ok(Credential {
            device_id: id,
            token: replacement,
            expires_at_ms: expires,
        })
    }
    pub fn devices(&self) -> Result<Vec<Device>, Status> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT id,name,expires FROM devices WHERE expires>? ORDER BY name,id")
            .map_err(db_error)?;
        stmt.query_map([now() as i64], |r| {
            Ok(Device {
                id: r.get(0)?,
                name: r.get(1)?,
                expires_at_ms: r.get::<_, i64>(2)? as u64,
            })
        })
        .map_err(db_error)?
        .collect::<Result<_, _>>()
        .map_err(db_error)
    }
    pub fn revoke(&self, id: &str) -> Result<(), Status> {
        self.db
            .lock()
            .unwrap()
            .execute("DELETE FROM devices WHERE id=?", [id])
            .map_err(db_error)?;
        Ok(())
    }
    pub fn claim(
        &self,
        device: &str,
        id: &str,
        payload: &str,
        created: u64,
        reserve: bool,
    ) -> Result<Claim, Status> {
        let at = now();
        if id.is_empty()
            || id.len() > 128
            || created < at.saturating_sub(OP_LIFETIME)
            || created > at + 300_000
        {
            return Err(Status::invalid_argument(
                "actions require an operation id and original timestamp within seven days",
            ));
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(db_error)?;
        let previous: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT hash,result FROM operations WHERE device=? AND id=?",
                params![device, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        if let Some((digest, result)) = previous {
            if digest != hash(payload) {
                return Err(Status::already_exists(
                    "operation id belongs to another command",
                ));
            }
            return Ok(result.map_or(Claim::Unknown, Claim::Done));
        }
        if !reserve {
            return Ok(Claim::New);
        }
        tx.execute(
            "DELETE FROM operations WHERE created<?",
            [at.saturating_sub(OP_LIFETIME) as i64],
        )
        .map_err(db_error)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
            .map_err(db_error)?;
        let device_count: i64 = tx
            .query_row(
                "SELECT count(*) FROM operations WHERE device=?",
                [device],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let pages: i64 = tx
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(db_error)?;
        let free: i64 = tx
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .map_err(db_error)?;
        let page_size: i64 = tx
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .map_err(db_error)?;
        if count >= 50_000
            || device_count >= 5_000
            || (pages - free) * page_size >= 512 * 1024 * 1024
        {
            return Err(Status::resource_exhausted("operation journal full"));
        }
        tx.execute(
            "INSERT INTO operations(device,id,hash,created) VALUES(?,?,?,?)",
            params![device, id, hash(payload), created as i64],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(Claim::New)
    }
    pub fn finish(&self, device: &str, id: &str, result: &str) -> Result<(), Status> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE operations SET result=? WHERE device=? AND id=?",
                params![result, device, id],
            )
            .map_err(db_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_rotation_revocation_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::open(dir.path()).unwrap();
        let (code, _) = auth.grant().unwrap();
        let c = auth.pair(&code, "phone").unwrap();
        assert!(auth.pair(&code, "other").is_err());
        assert_eq!(auth.check(&c.token).unwrap(), c.device_id);
        let next = auth.rotate(&c.token, "").unwrap();
        assert!(auth.check(&c.token).is_err());
        drop(auth);
        let auth = Auth::open(dir.path()).unwrap();
        assert!(auth.check(&next.token).is_ok());
        auth.revoke(&next.device_id).unwrap();
        assert!(auth.check(&next.token).is_err());
    }
    #[test]
    fn journal_never_replays_unknown_actions() {
        let dir = tempfile::tempdir().unwrap();
        let a = Auth::open(dir.path()).unwrap();
        let at = now();
        assert!(matches!(
            a.claim("d", "op", "body", at, true).unwrap(),
            Claim::New
        ));
        assert!(matches!(
            a.claim("d", "op", "body", at, true).unwrap(),
            Claim::Unknown
        ));
        assert!(a.claim("d", "op", "other", at, true).is_err());
        a.finish("d", "op", "result").unwrap();
        assert!(matches!(
            a.claim("d", "op", "body", at, true).unwrap(),
            Claim::Done(_)
        ));
        assert!(a.claim("d", "old", "body", 0, true).is_err());
    }
}
