use super::auth::{now, random};
use crate::{Blob, BlobSource, FileBlob, ProviderHandle, ProviderId};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tonic::Status;
pub const LIFETIME: u64 = 10 * 60 * 1000;
pub const CHUNK: u32 = 256 * 1024;
pub const UPLOAD_LIMIT: u64 = 100 * 1024 * 1024;
pub struct Remote {
    provider: Arc<ProviderHandle>,
    id: String,
    bytes: Option<Arc<Vec<u8>>>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        if self.bytes.is_none() {
            let _ = self.provider.release_blob(&self.id);
        }
    }
}
pub struct Stored {
    pub json: Value,
    pub pins: Vec<Arc<Remote>>,
}
enum Source {
    Remote(Arc<Remote>),
    Upload {
        blob: Blob,
        file: Arc<FileBlob>,
        _temp: tempfile::NamedTempFile,
    },
}
struct Lease {
    device: String,
    provider: ProviderId,
    expires: u64,
    source: Arc<Source>,
}
#[derive(Default)]
struct State {
    remote: HashMap<(ProviderId, String), Weak<Remote>>,
    leases: HashMap<String, Lease>,
}
#[derive(Default)]
pub struct Media {
    state: Mutex<State>,
}
fn sources(
    v: &mut Value,
    f: &mut impl FnMut(&mut String) -> Result<(), Status>,
) -> Result<(), Status> {
    match v {
        Value::Object(map) => {
            if let Some(Value::Object(source)) = map.get_mut("source")
                && let Some(Value::String(id)) = source.get_mut("Blob")
            {
                f(id)?;
            }
            for (key, value) in map {
                if key != "source" {
                    sources(value, f)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                sources(value, f)?;
            }
        }
        _ => {}
    };
    Ok(())
}
impl Media {
    pub fn retain(
        &self,
        provider: &Arc<ProviderHandle>,
        mut json: Value,
    ) -> Result<Stored, Status> {
        let mut state = self.state.lock().unwrap();
        state.remote.retain(|_, v| v.strong_count() > 0);
        let mut pins = Vec::new();
        sources(&mut json, &mut |id| {
            if pins.len() >= 4096 || state.remote.len() >= 8192 {
                return Err(Status::resource_exhausted(
                    "provider media capacity reached",
                ));
            }
            let key = (provider.id().clone(), id.clone());
            let remote = state
                .remote
                .get(&key)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| {
                    let remote = Arc::new(Remote {
                        provider: provider.clone(),
                        id: id.clone(),
                        bytes: None,
                    });
                    state.remote.insert(key, Arc::downgrade(&remote));
                    remote
                });
            pins.push(remote);
            Ok(())
        })?;
        Ok(Stored { json, pins })
    }
    pub fn encode(
        &self,
        device: &str,
        provider: &ProviderId,
        stored: &Stored,
    ) -> Result<super::wire::Data, Status> {
        let mut state = self.state.lock().unwrap();
        Self::expire(&mut state);
        let mut json = stored.json.clone();
        let expires = now() + LIFETIME;
        sources(&mut json, &mut |id| {
            let source = stored
                .pins
                .iter()
                .find(|p| p.id == *id)
                .ok_or_else(|| Status::not_found("media no longer available"))?;
            if let Some((lease_id, lease)) = state.leases.iter_mut().find(|(_, l)| {
                l.device == device
                    && l.provider == *provider
                    && matches!(&*l.source,Source::Remote(r) if Arc::ptr_eq(r,source))
            }) {
                lease.expires = expires;
                *id = lease_id.clone();
                return Ok(());
            }
            Self::capacity(&state, device)?;
            let lease_id = random();
            state.leases.insert(
                lease_id.clone(),
                Lease {
                    device: device.into(),
                    provider: provider.clone(),
                    expires,
                    source: Arc::new(Source::Remote(source.clone())),
                },
            );
            *id = lease_id;
            Ok(())
        })?;
        let json =
            serde_json::to_string(&json).map_err(|_| Status::internal("serialization failed"))?;
        if json.len() > 8 * 1024 * 1024 {
            return Err(Status::resource_exhausted("result exceeds app frame limit"));
        }
        Ok(super::wire::Data {
            json,
            blob_expires_at_ms: expires,
        })
    }
    fn expire(state: &mut State) {
        let at = now();
        state.leases.retain(|_, l| l.expires > at);
    }
    fn capacity(state: &State, device: &str) -> Result<(), Status> {
        if state.leases.len() >= 4096
            || state.leases.values().filter(|l| l.device == device).count() >= 256
        {
            Err(Status::resource_exhausted("release unused media handles"))
        } else {
            Ok(())
        }
    }
    pub fn cleanup(&self) {
        Self::expire(&mut self.state.lock().unwrap());
    }
    pub fn upload(
        &self,
        device: &str,
        provider: &Arc<ProviderHandle>,
        temp: tempfile::NamedTempFile,
        name: String,
        mime: String,
    ) -> Result<super::wire::Data, Status> {
        let file = Arc::new(
            FileBlob::open(temp.path()).map_err(|_| Status::internal("upload read failed"))?,
        );
        let blob = provider
            .add_blob(file.clone())
            .map_err(|_| Status::internal("upload registration failed"))?;
        let size = file.size();
        let mut state = self.state.lock().unwrap();
        Self::expire(&mut state);
        Self::capacity(&state, device)?;
        let total: u64 = state
            .leases
            .values()
            .filter(|l| l.device == device)
            .filter_map(|l| match &*l.source {
                Source::Upload { file, .. } => Some(file.size()),
                _ => None,
            })
            .sum();
        if total + size > 512 * 1024 * 1024 {
            return Err(Status::resource_exhausted("device upload quota exceeded"));
        }
        let id = random();
        let expires = now() + LIFETIME;
        state.leases.insert(
            id.clone(),
            Lease {
                device: device.into(),
                provider: provider.id().clone(),
                expires,
                source: Arc::new(Source::Upload {
                    blob,
                    file,
                    _temp: temp,
                }),
            },
        );
        Ok(super::wire::Data {
            json: serde_json::json!({"name":name,"mime":mime,"size":size,"source":{"Blob":id}})
                .to_string(),
            blob_expires_at_ms: expires,
        })
    }
    pub fn input(
        &self,
        device: &str,
        provider: &ProviderId,
        json: &mut Value,
    ) -> Result<Vec<Arc<impl Send + Sync + 'static>>, Status> {
        let mut state = self.state.lock().unwrap();
        Self::expire(&mut state);
        let mut pins = Vec::new();
        sources(json, &mut |id| {
            let lease = state
                .leases
                .get(id)
                .filter(|l| l.device == device && l.provider == *provider)
                .ok_or_else(|| {
                    Status::not_found("upload expired or belongs to another device or provider")
                })?;
            if let Source::Upload { blob, .. } = &*lease.source {
                *id = blob.id().into();
                pins.push(lease.source.clone());
                Ok(())
            } else {
                Err(Status::invalid_argument(
                    "downloaded media must be uploaded before sending",
                ))
            }
        })?;
        Ok(pins)
    }
    pub fn release(&self, device: &str, id: &str) -> Result<(), Status> {
        let mut state = self.state.lock().unwrap();
        if let Some(l) = state.leases.get(id)
            && l.device != device
        {
            return Err(Status::not_found("unknown media handle"));
        }
        state.leases.remove(id);
        Ok(())
    }
    pub async fn read(
        &self,
        device: &str,
        id: &str,
        offset: u64,
        length: u32,
    ) -> Result<Vec<u8>, Status> {
        if length > CHUNK || offset.checked_add(length as u64).is_none() {
            return Err(Status::invalid_argument("invalid read range"));
        }
        let source = {
            let mut state = self.state.lock().unwrap();
            Self::expire(&mut state);
            state
                .leases
                .get(id)
                .filter(|l| l.device == device)
                .map(|l| l.source.clone())
                .ok_or_else(|| Status::not_found("unknown or expired media handle"))?
        };
        match &*source {
            Source::Remote(remote) => {
                if let Some(bytes) = &remote.bytes {
                    return bytes
                        .read_at(offset, length as usize)
                        .map_err(|_| Status::internal("media read failed"));
                }
                remote
                    .provider
                    .read_blob(&remote.id, offset, length)
                    .await
                    .map_err(|e| Status::unavailable(e.to_string()))
            }
            Source::Upload { file, .. } => {
                let file = file.clone();
                tokio::task::spawn_blocking(move || file.read_at(offset, length as usize))
                    .await
                    .map_err(|_| Status::internal("read failed"))?
                    .map_err(|_| Status::internal("read failed"))
            }
        }
    }
}

impl Media {
    fn inline(
        &self,
        p: &Arc<ProviderHandle>,
        m: &mut qmsg_types::Media,
        pins: &mut Vec<Arc<Remote>>,
    ) -> Result<(), Status> {
        let qmsg_types::MediaSource::Bytes(bytes) = &mut m.source else {
            return Ok(());
        };
        let mut state = self.state.lock().unwrap();
        state.remote.retain(|_, v| v.strong_count() > 0);
        let allocated: usize = state
            .remote
            .values()
            .filter_map(Weak::upgrade)
            .filter_map(|r| r.bytes.as_ref().map(|b| b.len()))
            .sum();
        if state.remote.len() >= 8192 || allocated.saturating_add(bytes.len()) > 64 * 1024 * 1024 {
            return Err(Status::resource_exhausted("inline media capacity reached"));
        }
        let size = bytes.len() as u64;
        let id = format!("inline-{}", random());
        let source = Arc::new(Remote {
            provider: p.clone(),
            id: id.clone(),
            bytes: Some(Arc::new(std::mem::take(bytes))),
        });
        state
            .remote
            .insert((p.id().clone(), id.clone()), Arc::downgrade(&source));
        pins.push(source);
        m.source = qmsg_types::MediaSource::Blob(id);
        m.size = Some(size);
        Ok(())
    }
    pub fn reply(
        &self,
        p: &Arc<ProviderHandle>,
        mut reply: qmsg_types::Reply,
    ) -> Result<Stored, Status> {
        let mut pins = Vec::new();
        let mut visit = |m: &mut qmsg_types::Media| self.inline(p, m, &mut pins);
        match &mut reply {
            qmsg_types::Reply::Value(v) => values(v, &mut visit)?,
            qmsg_types::Reply::Message(m) => content(&mut m.content, &mut visit)?,
            qmsg_types::Reply::History(page) => {
                for m in &mut page.messages {
                    content(&mut m.content, &mut visit)?
                }
            }
            qmsg_types::Reply::Completed(page) => {
                for item in &mut page.items {
                    values(&mut item.value, &mut visit)?
                }
            }
            _ => {}
        }
        self.retain(
            p,
            serde_json::to_value(reply).map_err(|_| Status::internal("serialization failed"))?,
        )
    }
    pub fn event(
        &self,
        p: &Arc<ProviderHandle>,
        event: &mut crate::ProviderEvent,
    ) -> Result<Stored, Status> {
        use qmsg_types::{DirectoryUpdate as D, MessageEvent as M};
        let mut pins = Vec::new();
        let mut visit = |m: &mut qmsg_types::Media| self.inline(p, m, &mut pins);
        match event {
            crate::ProviderEvent::Message { event, .. } => match event {
                M::Received(m) => content(&mut m.content, &mut visit)?,
                M::Edited { content: parts, .. } => content(parts, &mut visit)?,
                _ => {}
            },
            crate::ProviderEvent::Directory { update, .. } => match update {
                D::OrganizationUpserted(o) => {
                    optional(&mut o.icon, &mut visit)?;
                    for u in &mut o.users {
                        optional(&mut u.avatar, &mut visit)?
                    }
                    for c in &mut o.channels {
                        optional(&mut c.icon, &mut visit)?
                    }
                }
                D::OrganizationUpdated { icon, .. } => optional(icon, &mut visit)?,
                D::UserUpserted { user, .. } => optional(&mut user.avatar, &mut visit)?,
                D::ChannelUpserted { channel, .. } => optional(&mut channel.icon, &mut visit)?,
                _ => {}
            },
            _ => {}
        }
        self.retain(
            p,
            serde_json::to_value(event).map_err(|_| Status::internal("serialization failed"))?,
        )
    }
}
fn optional(
    m: &mut Option<Box<qmsg_types::Media>>,
    visit: &mut impl FnMut(&mut qmsg_types::Media) -> Result<(), Status>,
) -> Result<(), Status> {
    if let Some(m) = m {
        visit(m)?
    }
    Ok(())
}
fn content(
    parts: &mut [qmsg_types::Content],
    visit: &mut impl FnMut(&mut qmsg_types::Media) -> Result<(), Status>,
) -> Result<(), Status> {
    for c in parts {
        match c {
            qmsg_types::Content::Image(m)
            | qmsg_types::Content::Video(m)
            | qmsg_types::Content::Audio(m)
            | qmsg_types::Content::File(m) => visit(m)?,
            _ => {}
        }
    }
    Ok(())
}
fn values(
    v: &mut qmsg_types::Value,
    visit: &mut impl FnMut(&mut qmsg_types::Media) -> Result<(), Status>,
) -> Result<(), Status> {
    use qmsg_types::Value as V;
    match v {
        V::Content(c) => content(c, visit)?,
        V::Message(m) => content(&mut m.content, visit)?,
        V::List(values) => {
            for v in values {
                self::values(v, visit)?
            }
        }
        V::Record(values) => {
            for v in values.values_mut() {
                self::values(v, visit)?
            }
        }
        _ => {}
    }
    Ok(())
}
