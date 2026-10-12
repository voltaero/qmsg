use crate::model::{Snapshot, Target};
use anyhow::{Context, Result, bail};
use qmsg_types::{Command, Content, HistoryPage, Reply};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

pub mod wire {
    tonic::include_proto!("qmsg.app.v1");
}
use wire::app_client::AppClient;

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
fn now() -> u64 {
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
        let mut tls = ClientTlsConfig::new().with_webpki_roots();
        if let Some(pem) = &credentials.certificate_pem {
            tls = tls.ca_certificate(Certificate::from_pem(pem));
        }
        endpoint = endpoint.tls_config(tls)?;
    }
    Ok(AppClient::new(endpoint.connect().await?))
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
    let mut client = connect(&credentials).await?;
    let reply = client
        .pair(wire::PairRequest {
            code: pairing.code,
            device_name: "qmsg terminal".into(),
        })
        .await?
        .into_inner();
    credentials.token = reply.token;
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
    use std::io::Write;
    options
        .open(path)?
        .write_all(&serde_json::to_vec_pretty(&credentials)?)?;
    Ok(())
}
pub enum Request {
    Select(Target),
    Older(Target, String),
    Send(Target, String),
    Refresh,
}
pub enum Update {
    Snapshot(Snapshot),
    History(Target, HistoryPage, bool),
    Sent(Target),
    SendFailed(Target, String),
    Error(String),
    Connected,
}
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
    async fn execute(&mut self, target: &Target, command: Command, action: bool) -> Result<Reply> {
        let request = self.request(wire::ExecuteRequest {
            provider: Some(wire::Provider {
                name: target.provider.name.clone(),
                instance: target.provider.instance,
            }),
            command_json: serde_json::to_string(&command)?,
            operation_id: if action {
                uuid::Uuid::new_v4().to_string()
            } else {
                String::new()
            },
            created_at_ms: if action { now() } else { 0 },
            timeout_ms: 25_000,
        })?;
        match self.rpc.execute(request).await?.into_inner().result {
            Some(wire::outcome::Result::Reply(data)) => Ok(serde_json::from_str(&data.json)?),
            Some(wire::outcome::Result::ErrorJson(error)) => bail!("{error}"),
            None => bail!("Server returned no result"),
        }
    }
    async fn history(&mut self, target: &Target, cursor: Option<String>) -> Result<HistoryPage> {
        match self
            .execute(
                target,
                Command::History {
                    request: 0,
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
}
pub async fn run(
    credentials: Credentials,
    mut requests: mpsc::Receiver<Request>,
    updates: std::sync::mpsc::Sender<Update>,
) {
    let mut selected = None;
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
        loop {
            let request = tokio::select! {
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
                        let command = Command::Send { request: 0, channel: target.channel.clone(), reply_to: None,
                            content: vec![Content::Text(text)], nonce: Some(uuid::Uuid::new_v4().to_string()) };
                        match client.execute(&target, command, true).await {
                            Ok(Reply::Sent { .. }) => { let _ = updates.send(Update::Sent(target)); }
                            Ok(_) => { let _ = updates.send(Update::SendFailed(target, "Unexpected send reply; delivery is unknown. Check history before trying again.".into())); }
                            Err(error) => { let _ = updates.send(Update::SendFailed(target, format!("{error}. Check history before trying again; delivery may be unknown."))); }
                        }
                    }
                    Some(Request::Refresh) | None => {
                        let snapshot = client.snapshot().await?;
                        let _ = updates.send(Update::Snapshot(snapshot));
                        if let Some(target) = &selected {
                            let page = client.history(target, None).await?;
                            let _ = updates.send(Update::History(target.clone(), page, false));
                        }
                    }
                }
                Ok(())
            }.await;
            if let Err(error) = result {
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
