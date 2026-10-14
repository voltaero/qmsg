use crate::media;
use crate::model::ProviderId;
use crate::model::{Snapshot, Target};
use anyhow::{Context, Result, bail};
use qmsg_types::{
    Arguments, ChannelRef, Content, Function, FunctionContext, HistoryPage, Media, MediaSource,
    Reply,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Semaphore, mpsc, watch};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

pub mod wire {
    tonic::include_proto!("qmsg.app.v1");
}
use wire::app_client::AppClient;

// The app API omits provider request ids and flattens function calls.
#[derive(Serialize)]
enum Command {
    Functions {
        context: FunctionContext,
    },
    Call {
        function: String,
        context: FunctionContext,
        arguments: Arguments,
    },
    History {
        channel: ChannelRef,
        cursor: Option<String>,
        limit: u32,
    },
    Send {
        channel: ChannelRef,
        reply_to: Option<String>,
        content: Vec<Content>,
        nonce: Option<String>,
    },
}

fn send_command(channel: ChannelRef, text: String) -> Command {
    Command::Send {
        channel,
        reply_to: None,
        content: vec![Content::Text(text)],
        nonce: None,
    }
}

#[derive(Debug, Deserialize)]
struct OperationError {
    message: String,
    delivery_unknown: bool,
}
impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if self.delivery_unknown {
            write!(
                f,
                "; delivery is unknown. Check history before sending again."
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for OperationError {}

#[derive(Serialize, Deserialize)]
pub struct Credentials {
    pub endpoint: String,
    pub token: String,
    pub certificate_pem: Option<String>,
}
#[derive(Deserialize)]
struct Pairing {
    version: u32,
    endpoint: String,
    code: String,
    expires_at_ms: u64,
    certificate_pem: Option<String>,
    certificate_sha256: Option<String>,
}
pub fn config_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .context("Set HOME or XDG_CONFIG_HOME")?;
    Ok(base.join("qmsg/device.json"))
}
pub fn load(path: &Path) -> Result<Credentials> {
    serde_json::from_slice(&std::fs::read(path).with_context(|| {
        format!(
            "No paired device at {}. Use qmsg --pair pairing.json",
            path.display()
        )
    })?)
    .map_err(Into::into)
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
async fn connect(credentials: &Credentials) -> Result<AppClient<Channel>> {
    let url = url::Url::parse(&credentials.endpoint)?;
    let loopback = url.host_str().is_some_and(|h| {
        h == "localhost"
            || h.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "Plaintext is allowed only on loopback; use HTTPS or a secure tunnel"
    );
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "Endpoint must contain only scheme, host and port"
    );
    let mut endpoint = Endpoint::from_shared(credentials.endpoint.clone())?
        .connect_timeout(Duration::from_secs(10));
    if url.scheme() == "https" {
        let tls = match &credentials.certificate_pem {
            Some(pem) => ClientTlsConfig::new().ca_certificate(Certificate::from_pem(pem)),
            None => ClientTlsConfig::new().with_webpki_roots(),
        };
        endpoint = endpoint.tls_config(tls)?;
    }
    Ok(AppClient::new(endpoint.connect().await?).max_decoding_message_size(8 * 1024 * 1024))
}
pub async fn pair(file: &Path, path: &Path) -> Result<()> {
    use sha2::{Digest, Sha256};
    let pairing: Pairing = serde_json::from_slice(&std::fs::read(file)?)?;
    anyhow::ensure!(
        pairing.version == 1 && pairing.expires_at_ms > now(),
        "Pairing code is expired or unsupported"
    );
    if let Some(pem) = &pairing.certificate_pem {
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                rustls_pemfile::certs(&mut pem.as_bytes())
                    .next()
                    .context("No certificate in pairing payload")??
                    .as_ref()
            )
        );
        anyhow::ensure!(
            pairing.certificate_sha256.as_deref() == Some(&fingerprint),
            "Pairing certificate fingerprint does not match"
        );
    }
    anyhow::ensure!(
        !path.exists(),
        "Device file already exists; choose another --config path"
    );
    let mut credentials = Credentials {
        endpoint: pairing.endpoint,
        token: String::new(),
        certificate_pem: pairing.certificate_pem,
    };
    // Open the device file first so a write failure cannot spend the one-time code.
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut device = options.open(path)?;
    let reply = async {
        let mut client = connect(&credentials).await?;
        let request = wire::PairRequest {
            code: pairing.code,
            device_name: "qmsg terminal".into(),
        };
        anyhow::Ok(client.pair(request).await?.into_inner())
    }
    .await;
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) => {
            drop(device);
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
    };
    credentials.token = reply.token;
    use std::io::Write;
    device.write_all(&serde_json::to_vec_pretty(&credentials)?)?;
    Ok(())
}
pub enum Request {
    Select(Target),
    Older(Target, String),
    Send(Target, String),
    Refresh,
    Login(ProviderId, String),
    SubmitLogin(ProviderId, Function, Arguments),
    Media {
        key: String,
        media: Media,
        kind: media::Kind,
    },
    /// Blob ids the UI no longer needs. Best-effort.
    Release(Vec<String>),
}
#[derive(Clone)]
pub struct Requests {
    actions: mpsc::Sender<Request>,
    selection: watch::Sender<Option<Target>>,
}
pub fn requests(
    capacity: usize,
) -> (
    Requests,
    mpsc::Receiver<Request>,
    watch::Receiver<Option<Target>>,
) {
    let (actions, receiver) = mpsc::channel(capacity);
    let (selection, selected) = watch::channel(None);
    (Requests { actions, selection }, receiver, selected)
}
impl Requests {
    pub fn select(&self, target: Option<Target>) -> Result<(), &'static str> {
        if self.selection.is_closed() {
            return Err("Backend disconnected");
        }
        self.selection.send_replace(target);
        Ok(())
    }
    pub fn try_send(&self, request: Request) -> Result<(), &'static str> {
        if let Request::Select(target) = request {
            self.select(Some(target))
        } else {
            self.actions
                .try_send(request)
                .map_err(|_| "Busy or disconnected")
        }
    }
}

pub enum Update {
    Snapshot(Snapshot),
    History(Target, HistoryPage, bool),
    Sent(Target),
    SendFailed(Target, String),
    Error(String),
    Connected,
    LoginForm(ProviderId, Function),
    LoginDone,
    Media(String, Result<media::Loaded, String>),
}
#[derive(Clone)]
struct Client {
    rpc: AppClient<Channel>,
    token: String,
}
impl Client {
    fn request<T>(&self, body: T) -> Result<tonic::Request<T>> {
        let mut request = tonic::Request::new(body);
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {}", self.token).parse()?);
        request.set_timeout(Duration::from_secs(30));
        Ok(request)
    }
    async fn snapshot(&mut self) -> Result<Snapshot> {
        let request = self.request(wire::Empty {})?;
        let data = self.rpc.snapshot(request).await?.into_inner();
        Ok(serde_json::from_str(&data.json)?)
    }
    async fn execute(
        &mut self,
        provider: &ProviderId,
        command: Command,
        action: bool,
    ) -> Result<Reply> {
        let login = matches!(command, Command::Call { .. });
        let mut request = self.request(wire::ExecuteRequest {
            provider: Some(wire::Provider {
                name: provider.name.clone(),
                instance: provider.instance,
            }),
            command_json: serde_json::to_string(&command)?,
            operation_id: if action {
                uuid::Uuid::new_v4().to_string()
            } else {
                String::new()
            },
            created_at_ms: if action { now() } else { 0 },
            timeout_ms: if login { 850_000 } else { 25_000 },
        })?;
        if login {
            request.set_timeout(Duration::from_secs(860));
        }
        match self.rpc.execute(request).await?.into_inner().result {
            Some(wire::outcome::Result::Reply(data)) => Ok(serde_json::from_str(&data.json)?),
            Some(wire::outcome::Result::ErrorJson(error)) => {
                Err(serde_json::from_str::<OperationError>(&error)?.into())
            }
            None => bail!("Server returned no result"),
        }
    }
    async fn history(&mut self, target: &Target, cursor: Option<String>) -> Result<HistoryPage> {
        match self
            .execute(
                &target.provider,
                Command::History {
                    channel: target.channel.clone(),
                    cursor,
                    limit: 50,
                },
                false,
            )
            .await?
        {
            Reply::History(page) => Ok(page),
            _ => bail!("Unexpected history reply"),
        }
    }
    /// Reads a whole blob, then releases it whatever happened.
    async fn read_blob(&mut self, id: &str, limit: u64) -> Result<Vec<u8>, String> {
        let result = self.read_blob_chunks(id, limit).await;
        self.release_blob(id).await;
        result
    }
    async fn read_blob_chunks(&mut self, id: &str, limit: u64) -> Result<Vec<u8>, String> {
        const CHUNK: u32 = 256 * 1024;
        let mut data = Vec::new();
        loop {
            let request = self
                .request(wire::ReadBlobRequest {
                    id: id.into(),
                    offset: data.len() as u64,
                    length: CHUNK,
                })
                .map_err(|_| "Could not read media")?;
            let chunk = self
                .rpc
                .read_blob(request)
                .await
                .map_err(|status| format!("Could not read media: {}", status.message()))?
                .into_inner()
                .data;
            if (data.len() + chunk.len()) as u64 > limit {
                return Err(media::too_large(limit));
            }
            data.extend_from_slice(&chunk);
            if chunk.len() < CHUNK as usize {
                return Ok(data);
            }
        }
    }
    async fn release_blob(&mut self, id: &str) {
        if let Ok(request) = self.request(wire::ReleaseBlobRequest { id: id.into() }) {
            let _ = self.rpc.release_blob(request).await;
        }
    }
    async fn load_media(
        mut self,
        http: Result<media::Http, String>,
        media: Media,
        kind: media::Kind,
    ) -> Result<media::Loaded, String> {
        let bytes = match (media::check(&media, kind, now()), media.source) {
            (Err(error), MediaSource::Blob(id)) => {
                self.release_blob(&id).await;
                return Err(error);
            }
            (Err(error), _) => return Err(error),
            (Ok(()), MediaSource::Bytes(bytes)) => bytes,
            (Ok(()), MediaSource::Url(url)) => media::fetch_url(&http?, &url, kind.limit()).await?,
            (Ok(()), MediaSource::Blob(id)) => self.read_blob(&id, kind.limit()).await?,
        };
        tokio::task::spawn_blocking(move || media::load(&bytes, media.mime.as_deref(), kind))
            .await
            .map_err(|_| "Could not load media".to_string())?
    }
}
pub async fn run(
    credentials: Credentials,
    mut requests: mpsc::Receiver<Request>,
    mut selection: watch::Receiver<Option<Target>>,
    updates: std::sync::mpsc::Sender<Update>,
) {
    let mut selected = None;
    let mut login_task: Option<tokio::task::JoinHandle<()>> = None;
    // Reads share the backend's per-device quota of four.
    let media_reads = Arc::new(Semaphore::new(4));
    let http = media::http_client();
    loop {
        let rpc = match connect(&credentials).await {
            Ok(rpc) => rpc,
            Err(error) => {
                if updates
                    .send(Update::Error(format!(
                        "Connection failed: {error}. Retrying…"
                    )))
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
                if requests.is_closed() {
                    return;
                }
                continue;
            }
        };
        let mut client = Client {
            rpc,
            token: credentials.token.clone(),
        };
        let _ = updates.send(Update::Connected);
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut recovering = false;
        loop {
            let request = tokio::select! {
                biased;
                changed = selection.changed() => {
                    if changed.is_err() { return; }
                    let target = selection.borrow_and_update().clone();
                    if target.is_none() { selected = None; }
                    target.map(Request::Select)
                },
                request = requests.recv() => { let Some(request) = request else { return; }; Some(request) },
                _ = interval.tick() => None,
            };
            let result: Result<()> = async {
                match request {
                    Some(Request::Select(target)) => {
                        selected = Some(target.clone());
                        let page = client.history(&target, None).await?;
                        let _ = updates.send(Update::History(target, page, false));
                    }
                    Some(Request::Older(target, cursor)) => {
                        let page = client.history(&target, Some(cursor)).await?;
                        let _ = updates.send(Update::History(target, page, true));
                    }
                    Some(Request::Send(target, text)) => {
                        let command = send_command(target.channel.clone(), text);
                        match client.execute(&target.provider, command, true).await {
                            Ok(Reply::Sent { .. }) => { let _ = updates.send(Update::Sent(target)); }
                            Ok(_) => { let _ = updates.send(Update::SendFailed(target, "Unexpected send reply; delivery is unknown. Check history before trying again.".into())); }
                            Err(error) => {
                                let message = if error.is::<OperationError>() { error.to_string() } else { format!("Delivery is unknown. Check history before sending again. {error}") };
                                let _ = updates.send(Update::SendFailed(target, message));
                            }
                        }
                    }
                    Some(Request::Login(provider, function)) => {
                        match client.execute(&provider, Command::Functions { context: FunctionContext::Provider }, false).await? {
                            Reply::Functions(functions) => {
                                let function = functions.into_iter().find(|f| f.id == function && matches!(f.kind, qmsg_types::FunctionKind::Action { action: qmsg_types::ActionKind::Login, .. })).context("Login function is no longer available")?;
                                let _ = updates.send(Update::LoginForm(provider, function));
                            }
                            _ => bail!("Unexpected function reply"),
                        }
                    }
                    Some(Request::SubmitLogin(provider, function, arguments)) => {
                        anyhow::ensure!(!login_task.as_ref().is_some_and(|task| !task.is_finished()), "A sign-in is already running");
                        // Browser sign-in can take minutes. Keep snapshots and keys live.
                        let mut client = client.clone();
                        let updates = updates.clone();
                        login_task = Some(tokio::spawn(async move {
                            let result = client.execute(&provider, Command::Call { function: function.id, context: FunctionContext::Provider, arguments }, true).await;
                            let _ = updates.send(match result {
                                Ok(_) => Update::LoginDone,
                                Err(error) => Update::Error(format!("Sign-in failed: {error}")),
                            });
                        }));
                    }
                    Some(Request::Media { key, media, kind }) => {
                        let (client, updates, reads, http) = (client.clone(), updates.clone(), media_reads.clone(), http.clone());
                        tokio::spawn(async move {
                            let result = match reads.acquire_owned().await {
                                Ok(_permit) => client.load_media(http, media, kind).await,
                                Err(_) => Err("Could not load media".into()),
                            };
                            let _ = updates.send(Update::Media(key, result));
                        });
                    }
                    Some(Request::Release(ids)) => {
                        let mut client = client.clone();
                        tokio::spawn(async move {
                            for id in ids {
                                client.release_blob(&id).await;
                            }
                        });
                    }
                    Some(Request::Refresh) | None => {
                        let snapshot = client.snapshot().await?;
                        let _ = updates.send(Update::Snapshot(snapshot));
                        if let Some(target) = &selected {
                            let page = client.history(target, None).await?;
                            let _ = updates.send(Update::History(target.clone(), page, false));
                        }
                        if recovering {
                            recovering = false;
                            let _ = updates.send(Update::Connected);
                        }
                    }
                }
                Ok(())
            }.await;
            if let Err(error) = result {
                recovering = true;
                if updates.send(Update::Error(error.to_string())).is_err() {
                    return;
                }
            }
            if requests.is_closed() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pairing_file(name: &str, endpoint: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qmsg-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("pairing.json");
        let pairing = serde_json::json!({
            "version": 1,
            "endpoint": endpoint,
            "code": "code",
            "expires_at_ms": now() + 60_000,
        });
        std::fs::write(&file, pairing.to_string()).unwrap();
        file
    }

    #[tokio::test]
    async fn unwritable_device_file_fails_before_spending_the_code() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let file = pairing_file("unwritable", &endpoint);
        // The parent of the device path is a regular file, so it cannot be created.
        let path = file.join("device.json");
        let paired = tokio::time::timeout(Duration::from_secs(5), pair(&file, &path)).await;
        assert!(paired.expect("pairing waited on the server").is_err());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "pairing contacted the server before the device file was writable"
        );
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn failed_pairing_removes_the_empty_device_file() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let file = pairing_file("refused", &format!("http://127.0.0.1:{port}"));
        let path = file.with_file_name("nested").join("device.json");
        assert!(pair(&file, &path).await.is_err());
        assert!(!path.exists());
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn newest_selection_survives_a_full_action_queue() {
        let (tx, mut actions, mut selected) = requests(1);
        tx.try_send(Request::Refresh).unwrap();
        for channel in ["first", "last"] {
            tx.try_send(Request::Select(Target {
                provider: ProviderId {
                    name: "test".into(),
                    instance: 1,
                },
                channel: ChannelRef::new(None, channel),
            }))
            .unwrap();
        }
        assert_eq!(
            selected
                .borrow_and_update()
                .as_ref()
                .unwrap()
                .channel
                .channel,
            "last"
        );
        assert!(matches!(actions.try_recv(), Ok(Request::Refresh)));
        assert!(actions.try_recv().is_err());
    }

    #[test]
    fn send_nonce_fits_discord_limit() {
        let command = send_command(ChannelRef::new(None, "general"), "hello".into());
        let Command::Send { nonce, .. } = command else {
            unreachable!()
        };
        // The host supplies a 24-character nonce when the client omits it.
        assert!(
            nonce.as_ref().is_none_or(|nonce| nonce.len() <= 25),
            "Discord refuses nonces longer than 25 characters"
        );
    }

    #[test]
    fn send_errors_show_message_and_only_warn_when_delivery_is_unknown() {
        let known: OperationError = serde_json::from_str(r#"{"code":"invalid_input","message":"Message is too long","command":null,"delivery_unknown":false}"#).unwrap();
        assert_eq!(known.to_string(), "Message is too long");
        let unknown = OperationError {
            message: "Timed out".into(),
            delivery_unknown: true,
        };
        assert!(unknown.to_string().contains("delivery is unknown"));
        assert!(!unknown.to_string().contains("delivery_unknown"));
    }

    #[tokio::test]
    async fn refuses_remote_plaintext_before_connecting() {
        let credentials = Credentials {
            endpoint: "http://192.0.2.1:50051".into(),
            token: "secret".into(),
            certificate_pem: None,
        };
        assert!(
            connect(&credentials)
                .await
                .unwrap_err()
                .to_string()
                .contains("Plaintext")
        );
    }
    #[test]
    fn commands_match_published_app_fields() {
        // Validate names against the checked-in server contract, not provider Command.
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../qmsg-orchestrator/proto/schema/command.json"
        ))
        .unwrap();
        let commands = [
            Command::History {
                channel: ChannelRef::new(None, "a"),
                cursor: None,
                limit: 50,
            },
            Command::Send {
                channel: ChannelRef::new(None, "a"),
                reply_to: None,
                content: vec![],
                nonce: None,
            },
            Command::Functions {
                context: FunctionContext::Provider,
            },
            Command::Call {
                function: "login".into(),
                context: FunctionContext::Provider,
                arguments: Arguments::new(),
            },
        ];
        for command in commands {
            let value = serde_json::to_value(command).unwrap();
            let (name, body) = value.as_object().unwrap().iter().next().unwrap();
            let variant = schema["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["properties"].get(name).is_some())
                .unwrap();
            let fields = variant["properties"][name]["properties"]
                .as_object()
                .unwrap();
            for key in body.as_object().unwrap().keys() {
                assert!(fields.contains_key(key), "{name}.{key} is not an app field");
            }
            for required in variant["properties"][name]["required"].as_array().unwrap() {
                assert!(body.get(required.as_str().unwrap()).is_some());
            }
        }
    }
}
