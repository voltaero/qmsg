//! App-facing gRPC API. Provider secrets and host control commands never cross it.
mod auth;
mod hub;
mod media;
pub mod wire {
    tonic::include_proto!("qmsg.app.v1");
}
use crate::{ProviderEvent, ProviderHandle, RequestError};
use anyhow::Context as _;
use qmsg_types::*;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};
use tonic::{Request, Response, Status};
use wire::app_server::{App, AppServer};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub listen: SocketAddr,
    pub endpoint: Option<String>,
    pub certificate: Option<PathBuf>,
    pub private_key: Option<PathBuf>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:50051".parse().unwrap(),
            endpoint: None,
            certificate: None,
            private_key: None,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum Command {
    Functions {
        context: FunctionContext,
    },
    Call {
        function: String,
        context: FunctionContext,
        arguments: Arguments,
    },
    Verify(VerificationRequest),
    Complete(CompletionRequest),
    History {
        channel: ChannelRef,
        cursor: Option<String>,
        limit: u32,
    },
    GetMessage {
        channel: ChannelRef,
        id: String,
    },
    Send {
        channel: ChannelRef,
        reply_to: Option<String>,
        content: Vec<Content>,
        nonce: Option<String>,
    },
    OpenChannel {
        organization: Option<String>,
        members: Vec<String>,
    },
}
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub command: Option<CommandError>,
    pub delivery_unknown: bool,
}
#[derive(Serialize, Deserialize)]
enum Cached {
    Reply { json: String, expires: u64 },
    Error(String),
}
#[derive(Clone)]
pub struct Service {
    auth: Arc<auth::Auth>,
    hub: Arc<hub::Hub>,
    data_dir: PathBuf,
    requests: Arc<Semaphore>,
    streams: Arc<Semaphore>,
    uploads: Arc<Semaphore>,
    reads: Arc<Semaphore>,
    devices: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
}
impl Service {
    pub fn new(
        data_dir: &Path,
        providers: Vec<Arc<ProviderHandle>>,
        events: mpsc::Receiver<ProviderEvent>,
    ) -> anyhow::Result<Self> {
        let auth = Arc::new(auth::Auth::open(data_dir)?);
        let media = Arc::new(media::Media::default());
        Ok(Self {
            auth,
            hub: hub::Hub::new(providers, events, media),
            data_dir: data_dir.into(),
            requests: Arc::new(Semaphore::new(64)),
            streams: Arc::new(Semaphore::new(64)),
            uploads: Arc::new(Semaphore::new(4)),
            reads: Arc::new(Semaphore::new(4)),
            devices: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    fn authenticate<T>(&self, request: &Request<T>) -> Result<(String, String), Status> {
        let token = request
            .metadata()
            .get("authorization")
            .and_then(|s| s.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or_else(|| Status::unauthenticated("Bearer device token required"))?;
        Ok((self.auth.check(token)?, token.into()))
    }
    fn slots(&self, device: &str) -> Result<tokio::sync::OwnedSemaphorePermit, Status> {
        let mut devices = self.devices.lock().unwrap();
        devices.retain(|_, s| s.available_permits() != 16 || Arc::strong_count(s) > 1);
        let semaphore = devices
            .entry(device.into())
            .or_insert_with(|| Arc::new(Semaphore::new(16)))
            .clone();
        semaphore
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too many device requests"))
    }
    pub async fn serve(self, config: Config) -> anyhow::Result<()> {
        config.validate()?;
        let mut server = tonic::transport::Server::builder()
            .http2_keepalive_interval(Some(Duration::from_secs(30)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .concurrency_limit_per_connection(32)
            .timeout(Duration::from_secs(900));
        match (&config.certificate, &config.private_key) {
            (Some(cert), Some(key)) => {
                let cert = tokio::fs::read(cert).await?;
                let key = tokio::fs::read(key).await?;
                server = server.tls_config(
                    tonic::transport::ServerTlsConfig::new()
                        .identity(tonic::transport::Identity::from_pem(cert, key)),
                )?;
            }
            (None, None) if config.listen.ip().is_loopback() => {}
            _ => anyhow::bail!(
                "LAN listeners require certificate and private_key; use localhost for a secure tunnel"
            ),
        }
        tracing::info!(listen=%config.listen,"app gRPC API listening");
        server
            .add_service(
                AppServer::new(self)
                    .max_decoding_message_size(1024 * 1024)
                    .max_encoding_message_size(8 * 1024 * 1024),
            )
            .serve(config.listen)
            .await?;
        Ok(())
    }
    async fn declaration(
        p: &ProviderHandle,
        id: &str,
        context: FunctionContext,
    ) -> Result<Function, RequestError> {
        p.functions(context)
            .await?
            .into_iter()
            .find(|f| f.id == id)
            .ok_or_else(|| RequestError::Failed(CommandError::UnknownFunction(id.into())))
    }
    async fn dispatch(
        p: &ProviderHandle,
        command: Command,
        declared: Option<Function>,
    ) -> Result<Reply, RequestError> {
        match command {
            Command::Functions { context } => p.functions(context).await.map(Reply::Functions),
            Command::Call {
                function,
                context,
                arguments,
            } => {
                let f = declared.ok_or(RequestError::Failed(CommandError::UnknownFunction(
                    function,
                )))?;
                p.call_function(&f, context, arguments)
                    .await
                    .map(Reply::Value)
            }
            Command::Verify(v) => {
                let f = Self::declaration(p, &v.call.function, v.call.context.clone()).await?;
                p.verify(&f, v).await.map(Reply::Verified)
            }
            Command::Complete(c) => {
                let f = Self::declaration(p, &c.call.function, c.call.context.clone()).await?;
                p.complete(&f, c).await.map(Reply::Completed)
            }
            Command::History {
                channel,
                cursor,
                limit,
            } => {
                let mut page = p.history(channel, cursor, limit).await?;
                page.messages
                    .sort_by(|a, b| a.sent_at.cmp(&b.sent_at).then_with(|| a.id.cmp(&b.id)));
                Ok(Reply::History(page))
            }
            Command::GetMessage { channel, id } => {
                p.get_message(channel, id).await.map(Reply::Message)
            }
            Command::Send {
                channel,
                reply_to,
                content,
                nonce,
            } => p
                .send_message(channel, reply_to, content, nonce)
                .await
                .map(|id| Reply::Sent { id }),
            Command::OpenChannel {
                organization,
                members,
            } => p
                .open_channel(organization, members)
                .await
                .map(Reply::Opened),
        }
    }
    fn error(error: RequestError, action: bool) -> Cached {
        let proven = matches!(&error,
            RequestError::Failed(CommandError::Provider {details:Some(details),..})
                if matches!(details.as_ref(),Value::Record(data) if data.get("delivery_unknown")==Some(&Value::Boolean(false)))
        );
        let unknown = action
            && !proven
            && matches!(
                error,
                RequestError::NotRunning
                    | RequestError::TimedOut
                    | RequestError::BadReply(_)
                    | RequestError::Failed(CommandError::Failed(_) | CommandError::Provider { .. })
            );
        let code = match &error {
            RequestError::Failed(_) => "provider",
            RequestError::Busy => "busy",
            RequestError::TooLarge { .. } => "too_large",
            RequestError::TimedOut => "timeout",
            RequestError::NotRunning => "not_running",
            RequestError::BadReply(_) => "bad_reply",
            RequestError::WrongFunction(_) => "wrong_function",
        };
        let command = if let RequestError::Failed(e) = &error {
            Some(e.clone())
        } else {
            None
        };
        Cached::Error(
            serde_json::to_string(&Error {
                code: code.into(),
                message: error.to_string(),
                command,
                delivery_unknown: unknown,
            })
            .unwrap(),
        )
    }
    async fn previous(&self, claim: auth::Claim) -> Result<Option<wire::Outcome>, Status> {
        let result=match claim{auth::Claim::New=>return Ok(None),auth::Claim::Done(result)=>serde_json::from_str(&result).map_err(|_|Status::internal("stored result invalid"))?,auth::Claim::Unknown=>Cached::Error(serde_json::to_string(&Error{code:"delivery_unknown".into(),message:"operation already started; check messages or state before taking further action".into(),command:None,delivery_unknown:true}).unwrap())};
        Ok(Some(self.outcome(result).await))
    }
    async fn outcome(&self, result: Cached) -> wire::Outcome {
        let sequence = self.hub.barrier().await;
        wire::Outcome {
            epoch: self.hub.epoch.clone(),
            observed_sequence: sequence,
            result: Some(match result {
                Cached::Reply { json, expires } => wire::outcome::Result::Reply(wire::Data {
                    json,
                    blob_expires_at_ms: expires,
                }),
                Cached::Error(json) => wire::outcome::Result::ErrorJson(json),
            }),
        }
    }
}
#[tonic::async_trait]
impl App for Service {
    async fn pair(
        &self,
        request: Request<wire::PairRequest>,
    ) -> Result<Response<wire::Credential>, Status> {
        let _permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("server busy"))?;
        let r = request.into_inner();
        Ok(Response::new(self.auth.pair(&r.code, &r.device_name)?))
    }
    async fn rotate(
        &self,
        request: Request<wire::RotateRequest>,
    ) -> Result<Response<wire::Credential>, Status> {
        let (_, token) = self.authenticate(&request)?;
        Ok(Response::new(self.auth.rotate(
            &token,
            &request.into_inner().replacement_secret,
        )?))
    }
    async fn devices(
        &self,
        request: Request<wire::Empty>,
    ) -> Result<Response<wire::DeviceList>, Status> {
        self.authenticate(&request)?;
        Ok(Response::new(wire::DeviceList {
            devices: self.auth.devices()?,
        }))
    }
    async fn revoke(
        &self,
        request: Request<wire::RevokeRequest>,
    ) -> Result<Response<wire::Empty>, Status> {
        self.authenticate(&request)?;
        self.auth.revoke(&request.into_inner().device_id)?;
        Ok(Response::new(wire::Empty {}))
    }
    async fn snapshot(
        &self,
        request: Request<wire::Empty>,
    ) -> Result<Response<wire::Data>, Status> {
        let (device, _) = self.authenticate(&request)?;
        let _permit = self.slots(&device)?;
        let event = self.hub.snapshot(&device)?;
        let Some(wire::event::Body::Snapshot(mut data)) = event.body else {
            unreachable!()
        };
        let mut value: serde_json::Value = serde_json::from_str(&data.json).unwrap();
        value["epoch"] = event.epoch.into();
        value["sequence"] = event.sequence.into();
        data.json = value.to_string();
        Ok(Response::new(data))
    }
    type WatchStream =
        Pin<Box<dyn futures_util::Stream<Item = Result<wire::Event, Status>> + Send>>;
    async fn watch(
        &self,
        request: Request<wire::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let (device, token) = self.authenticate(&request)?;
        let device_slot = self.slots(&device)?;
        let global_slot = self
            .streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too many streams"))?;
        let stream = self
            .hub
            .watch(device, request.into_inner(), self.auth.clone(), token)?;
        Ok(Response::new(Box::pin(
            async_stream::stream! {let _slots=(device_slot,global_slot);for await item in stream {yield item;}},
        )))
    }
    async fn execute(
        &self,
        request: Request<wire::ExecuteRequest>,
    ) -> Result<Response<wire::Outcome>, Status> {
        let (device, token) = self.authenticate(&request)?;
        let _slot = self.slots(&device)?;
        let _global = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("server busy"))?;
        let r = request.into_inner();
        let provider = self.hub.provider(r.provider.as_ref())?;
        let mut json: serde_json::Value = serde_json::from_str(&r.command_json)
            .map_err(|_| Status::invalid_argument("invalid command JSON"))?;
        let parsed: Command = serde_json::from_value(json.clone())
            .map_err(|e| Status::invalid_argument(format!("invalid command: {e}")))?;
        match &parsed {
            Command::History { limit, .. } if *limit == 0 || *limit > 100 => {
                return Err(Status::invalid_argument("history limit must be 1..100"));
            }
            Command::Complete(c) if c.limit == 0 || c.limit > 100 => {
                return Err(Status::invalid_argument("completion limit must be 1..100"));
            }
            _ => {}
        }
        let timeout = Duration::from_millis(if r.timeout_ms == 0 {
            60_000
        } else {
            r.timeout_ms.clamp(100, 900_000)
        });
        let payload = serde_json::json!({"provider":provider.id().name,"command":json}).to_string();
        if !r.operation_id.is_empty()
            && let Some(outcome) = self
                .previous(self.auth.claim(
                    &device,
                    &r.operation_id,
                    &payload,
                    r.created_at_ms,
                    false,
                )?)
                .await?
        {
            return Ok(Response::new(outcome));
        }
        let mut declared = None;
        let action = match &parsed {
            Command::Send { .. } | Command::OpenChannel { .. } => true,
            Command::Call {
                function, context, ..
            } => {
                let f = tokio::time::timeout(
                    timeout,
                    Self::declaration(&provider, function, context.clone()),
                )
                .await
                .map_err(|_| Status::deadline_exceeded("function discovery timed out"))?
                .map_err(|e| Status::failed_precondition(e.to_string()))?;
                let action = matches!(f.kind, FunctionKind::Action { .. });
                declared = Some(f);
                action
            }
            _ => false,
        };
        self.auth.check(&token)?;

        if action
            && let Some(outcome) = self
                .previous(self.auth.claim(
                    &device,
                    &r.operation_id,
                    &payload,
                    r.created_at_ms,
                    false,
                )?)
                .await?
        {
            return Ok(Response::new(outcome));
        }
        let _media_pins = self.hub.media.input(&device, provider.id(), &mut json)?;
        if action
            && let Some(outcome) = self
                .previous(self.auth.claim(
                    &device,
                    &r.operation_id,
                    &payload,
                    r.created_at_ms,
                    true,
                )?)
                .await?
        {
            return Ok(Response::new(outcome));
        }
        let mut command: Command = serde_json::from_value(json)
            .map_err(|_| Status::invalid_argument("invalid media input"))?;
        if let Command::Send { nonce, .. } = &mut command
            && nonce.is_none()
        {
            *nonce = Some(auth::hash(&format!("{device}:{}", r.operation_id))[..24].into());
        }
        let result =
            match tokio::time::timeout(timeout, Self::dispatch(&provider, command, declared)).await
            {
                Ok(Ok(reply)) => {
                    let stored = self.hub.media.reply(&provider, reply)?;
                    let data = self.hub.media.encode(&device, provider.id(), &stored)?;
                    Cached::Reply {
                        json: data.json,
                        expires: data.blob_expires_at_ms,
                    }
                }
                Ok(Err(error)) => Self::error(error, action),
                Err(_) => Self::error(RequestError::TimedOut, action),
            };
        if action {
            let saved = serde_json::to_string(&result).unwrap();
            if saved.len() > 128 * 1024 {
                return Err(Status::resource_exhausted(
                    "action result too large; check state before retrying",
                ));
            }
            self.auth.finish(&device, &r.operation_id, &saved)?;
        }
        Ok(Response::new(self.outcome(result).await))
    }
    async fn upload(
        &self,
        request: Request<tonic::Streaming<wire::UploadChunk>>,
    ) -> Result<Response<wire::Data>, Status> {
        let (device, token) = self.authenticate(&request)?;
        let _slot = self.slots(&device)?;
        let _upload = self
            .uploads
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too many uploads"))?;
        let mut stream = request.into_inner();
        let result = tokio::time::timeout(Duration::from_secs(120), async {
            let first = stream
                .message()
                .await?
                .ok_or_else(|| Status::invalid_argument("upload metadata required"))?;
            let p = self.hub.provider(first.provider.as_ref())?;
            if first.size > media::UPLOAD_LIMIT || first.name.len() > 255 || first.mime.len() > 128
            {
                return Err(Status::invalid_argument("invalid upload metadata or size"));
            }
            let temp = tempfile::NamedTempFile::new_in(&self.data_dir)
                .map_err(|_| Status::internal("upload storage unavailable"))?;
            let file = temp
                .reopen()
                .map_err(|_| Status::internal("upload storage unavailable"))?;
            let mut file = tokio::fs::File::from_std(file);
            let mut size = 0;
            async fn chunk(
                file: &mut tokio::fs::File,
                size: &mut u64,
                expected: u64,
                data: &[u8],
            ) -> Result<(), Status> {
                use tokio::io::AsyncWriteExt;
                if data.len() > media::CHUNK as usize
                    || (*size).saturating_add(data.len() as u64) > expected
                {
                    return Err(Status::invalid_argument(
                        "upload chunk exceeds declared size or 256 KiB",
                    ));
                }
                file.write_all(data)
                    .await
                    .map_err(|_| Status::internal("upload write failed"))?;
                *size += data.len() as u64;
                Ok(())
            }
            chunk(&mut file, &mut size, first.size, &first.data).await?;
            while let Some(next) = stream.message().await? {
                self.auth.check(&token)?;
                if next.provider.is_some()
                    || !next.name.is_empty()
                    || !next.mime.is_empty()
                    || next.size != 0
                {
                    return Err(Status::invalid_argument(
                        "metadata only allowed in first chunk",
                    ));
                }
                chunk(&mut file, &mut size, first.size, &next.data).await?;
            }
            if size != first.size {
                return Err(Status::invalid_argument("upload size mismatch"));
            }
            use tokio::io::AsyncWriteExt;
            file.flush()
                .await
                .map_err(|_| Status::internal("upload write failed"))?;
            self.auth.check(&token)?;
            self.hub
                .media
                .upload(&device, &p, temp, first.name, first.mime)
        })
        .await
        .map_err(|_| Status::deadline_exceeded("upload timed out"))??;
        Ok(Response::new(result))
    }
    async fn read_blob(
        &self,
        request: Request<wire::ReadBlobRequest>,
    ) -> Result<Response<wire::BlobChunk>, Status> {
        let (device, _) = self.authenticate(&request)?;
        let _slot = self.slots(&device)?;
        let _global = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("server busy"))?;
        let _read = self
            .reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too many media reads"))?;
        let r = request.into_inner();
        let data = tokio::time::timeout(
            Duration::from_secs(60),
            self.hub.media.read(&device, &r.id, r.offset, r.length),
        )
        .await
        .map_err(|_| Status::deadline_exceeded("media read timed out"))??;
        Ok(Response::new(wire::BlobChunk { data }))
    }
    async fn release_blob(
        &self,
        request: Request<wire::ReleaseBlobRequest>,
    ) -> Result<Response<wire::Empty>, Status> {
        let (device, _) = self.authenticate(&request)?;
        self.hub.media.release(&device, &request.into_inner().id)?;
        Ok(Response::new(wire::Empty {}))
    }
}
/// Mint a pairing grant through the local CLI, never through an unauthenticated RPC.
pub fn pairing(data_dir: &Path, config: &Config) -> anyhow::Result<String> {
    config.validate()?;
    let endpoint = config.endpoint.clone().unwrap_or_else(|| {
        format!(
            "{}://{}",
            if config.certificate.is_some() {
                "https"
            } else {
                "http"
            },
            config.listen
        )
    });
    let certificate = if let Some(path) = &config.certificate {
        Some(std::fs::read_to_string(path)?)
    } else {
        None
    };
    if !config.listen.ip().is_loopback() && certificate.is_none() {
        anyhow::bail!("LAN pairing requires TLS");
    }
    let fingerprint = if let Some(pem) = &certificate {
        let der = rustls_pemfile::certs(&mut pem.as_bytes())
            .next()
            .context("certificate is empty")??;
        Some(
            sha2::Sha256::digest(der.as_ref())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
    } else {
        None
    };
    let (code, expires) = auth::Auth::open(data_dir)?.grant()?;
    Ok(serde_json::json!({"version":1,"endpoint":endpoint,"code":code,"expires_at_ms":expires,"certificate_pem":certificate,"certificate_sha256":fingerprint}).to_string())
}
#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
struct ProviderView {
    provider: crate::ProviderId,
    status: ProviderStatus,
    organizations: Vec<crate::OrganizationEntry>,
    standalone: Option<crate::Scope>,
}
#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
struct Snapshot {
    providers: Vec<ProviderView>,
    epoch: String,
    sequence: u64,
}
pub fn schema() -> serde_json::Value {
    serde_json::json!({"command":schemars::schema_for!(Command),"reply":schemars::schema_for!(Reply),"error":schemars::schema_for!(Error),"snapshot":schemars::schema_for!(Snapshot),"event":schemars::schema_for!(ProviderEvent)})
}
/// Create a local TLS identity without overwriting an existing key.
pub fn create_identity(data_dir: &Path, hosts: Vec<String>) -> anyhow::Result<(PathBuf, PathBuf)> {
    crate::secrets::create_dir(data_dir)?;
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(hosts)?;
    let certificate = data_dir.join("app-cert.pem");
    let private_key = data_dir.join("app-key.pem");
    anyhow::ensure!(
        !certificate.exists() && !private_key.exists(),
        "TLS identity already exists; refusing to overwrite it"
    );
    for (path, bytes) in [
        (&private_key, signing_key.serialize_pem()),
        (&certificate, cert.pem()),
    ] {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options.open(path)?.write_all(bytes.as_bytes())?;
    }
    Ok((certificate, private_key))
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.listen.ip().is_unspecified() || self.endpoint.is_some(),
            "set app.endpoint to a reachable host when listening on all interfaces"
        );
        anyhow::ensure!(
            self.certificate.is_some() == self.private_key.is_some(),
            "set both certificate and private_key"
        );
        anyhow::ensure!(
            self.listen.ip().is_loopback() || self.certificate.is_some(),
            "LAN listeners require TLS; use localhost for a secure tunnel"
        );
        if let Some(endpoint) = &self.endpoint {
            let url = reqwest::Url::parse(endpoint)?;
            anyhow::ensure!(
                url.username().is_empty()
                    && url.password().is_none()
                    && url.path() == "/"
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "endpoint must contain only scheme, host and port"
            );
            let loopback = url.host_str().is_some_and(|h| {
                h == "localhost"
                    || h.parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            });
            anyhow::ensure!(
                url.scheme() == "https" || (url.scheme() == "http" && loopback),
                "pairing endpoint requires HTTPS, except localhost inside a secure tunnel"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::app_client::AppClient;
    fn authorized<T>(body: T, token: &str) -> Request<T> {
        let mut r = Request::new(body);
        r.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        r
    }
    #[tokio::test]
    async fn grpc_pair_auth_resume_and_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let (_tx, rx) = mpsc::channel(8);
        let service = Service::new(dir.path(), vec![], rx).unwrap();
        let (code, _) = service.auth.grant().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(AppServer::new(service.clone()))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
        );
        let mut client = AppClient::connect(format!("http://{address}"))
            .await
            .unwrap();
        assert_eq!(
            client.snapshot(wire::Empty {}).await.unwrap_err().code(),
            tonic::Code::Unauthenticated
        );
        let credential = client
            .pair(wire::PairRequest {
                code: code.clone(),
                device_name: "test phone".into(),
            })
            .await
            .unwrap()
            .into_inner();
        assert!(
            client
                .pair(wire::PairRequest {
                    code,
                    device_name: "another".into()
                })
                .await
                .is_err()
        );
        let snapshot = client
            .snapshot(authorized(wire::Empty {}, &credential.token))
            .await
            .unwrap()
            .into_inner();
        let json: serde_json::Value = serde_json::from_str(&snapshot.json).unwrap();
        assert!(json["epoch"].is_string());
        let mut stream = client
            .watch(authorized(
                wire::WatchRequest {
                    epoch: "old".into(),
                    after_sequence: 999,
                },
                &credential.token,
            ))
            .await
            .unwrap()
            .into_inner();
        let snapshot = stream.message().await.unwrap().unwrap();
        assert!(matches!(
            snapshot.body,
            Some(wire::event::Body::Snapshot(_))
        ));
        let rotated = client
            .rotate(authorized(
                wire::RotateRequest {
                    replacement_secret: String::new(),
                },
                &credential.token,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(
            client
                .devices(authorized(wire::Empty {}, &credential.token))
                .await
                .is_err()
        );
        // The old stream checks the old token even though its HTTP/2 connection remains open.
        let error = tokio::time::timeout(Duration::from_secs(16), async {
            loop {
                match stream.message().await {
                    Err(e) => break e,
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("unexpected stream closure"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(error.code(), tonic::Code::Unauthenticated);
        client
            .revoke(authorized(
                wire::RevokeRequest {
                    device_id: rotated.device_id,
                },
                &rotated.token,
            ))
            .await
            .unwrap();
        assert!(
            client
                .snapshot(authorized(wire::Empty {}, &rotated.token))
                .await
                .is_err()
        );
        server.abort();
    }
    #[test]
    fn provider_can_report_a_definite_action_failure() {
        for (details, expected) in [
            (None, true),
            (
                Some(Box::new(Value::Record(Arguments::from([(
                    "delivery_unknown".into(),
                    Value::Boolean(false),
                )])))),
                false,
            ),
        ] {
            let cached = Service::error(
                RequestError::Failed(CommandError::Provider {
                    code: "smtp_550".into(),
                    message: "Recipient rejected".into(),
                    details,
                }),
                true,
            );
            let Cached::Error(json) = cached else {
                panic!("expected error")
            };
            let error: Error = serde_json::from_str(&json).unwrap();
            assert_eq!(error.delivery_unknown, expected);
        }
    }

    #[test]
    fn refuse_cleartext_lan_and_bad_endpoints() {
        let mut c = Config {
            listen: "0.0.0.0:50051".parse().unwrap(),
            ..Default::default()
        };
        assert!(c.validate().is_err());
        c.listen = "127.0.0.1:50051".parse().unwrap();
        assert!(c.validate().is_ok());
        c.endpoint = Some("http://192.168.1.2:50051".into());
        assert!(c.validate().is_err());
        c.endpoint = Some("https://host/path".into());
        assert!(c.validate().is_err());
    }
    #[test]
    fn identity_pairing_and_schema() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = create_identity(dir.path(), vec!["localhost".into()]).unwrap();
        assert!(create_identity(dir.path(), vec!["localhost".into()]).is_err());
        let c = Config {
            certificate: Some(cert),
            private_key: Some(key),
            ..Default::default()
        };
        let payload: serde_json::Value =
            serde_json::from_str(&pairing(dir.path(), &c).unwrap()).unwrap();
        assert_eq!(payload["certificate_sha256"].as_str().unwrap().len(), 64);
        assert!(
            payload["certificate_pem"]
                .as_str()
                .unwrap()
                .contains("BEGIN CERTIFICATE")
        );
        assert!(schema()["command"].is_object());
    }
}

#[cfg(test)]
mod tls_tests {
    use super::*;
    #[tokio::test]
    async fn tls_uses_qr_certificate_and_checks_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = create_identity(dir.path(), vec!["localhost".into()]).unwrap();
        let pem = tokio::fs::read(cert).await.unwrap();
        let private = tokio::fs::read(key).await.unwrap();
        let (_tx, events) = mpsc::channel(4);
        let service = Service::new(dir.path(), vec![], events).unwrap();
        let (code, _) = service.auth.grant().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(
            tonic::transport::Server::builder()
                .tls_config(
                    tonic::transport::ServerTlsConfig::new()
                        .identity(tonic::transport::Identity::from_pem(&pem, private)),
                )
                .unwrap()
                .add_service(AppServer::new(service))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
        );
        let endpoint = tonic::transport::Endpoint::from_shared(format!("https://{address}"))
            .unwrap()
            .connect_timeout(Duration::from_secs(3));
        let trust = tonic::transport::Certificate::from_pem(&pem);
        assert!(
            endpoint
                .clone()
                .tls_config(
                    tonic::transport::ClientTlsConfig::new()
                        .domain_name("wrong-host")
                        .ca_certificate(trust.clone())
                )
                .unwrap()
                .connect()
                .await
                .is_err()
        );
        let channel = endpoint
            .tls_config(
                tonic::transport::ClientTlsConfig::new()
                    .domain_name("localhost")
                    .ca_certificate(trust),
            )
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = wire::app_client::AppClient::new(channel);
        let device = client
            .pair(wire::PairRequest {
                code,
                device_name: "TLS fixture".into(),
            })
            .await
            .unwrap()
            .into_inner();
        let secret = auth::random();
        let proposed = format!("{}.{}", device.device_id, secret);
        let mut request = Request::new(wire::RotateRequest {
            replacement_secret: secret,
        });
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", device.token).parse().unwrap(),
        );
        let rotated = client.rotate(request).await.unwrap().into_inner();
        assert_eq!(rotated.token, proposed);
        let mut request = Request::new(wire::Empty {});
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {proposed}").parse().unwrap(),
        );
        assert!(client.snapshot(request).await.is_ok());
        server.abort();
    }
}
