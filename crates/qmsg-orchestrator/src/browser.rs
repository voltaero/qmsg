//! Signs providers in through the browser login service in
//! `services/browser-login`, a host process that drives a headless browser
//! with a recipe per platform. Providers never run it themselves: they ask
//! with [`ProviderMessage::BrowserLogin`](qmsg_types::ProviderMessage) and
//! get back the session token.
//!
//! The service is started on the first login, with a private key generated
//! here, and stopped, with its browsers, when the orchestrator is dropped.
//! Users sign in through its viewer page, linked in the provider's
//! [`ProviderStatus::LoginRequired`](qmsg_types::ProviderStatus).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

/// The most browser logins running at once, for all providers.
pub const MAX_BROWSER_LOGINS: usize = 4;

/// How to run the browser login service.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// The program and arguments that start the service.
    pub command: Vec<String>,
    /// The directory to run it in.
    pub dir: PathBuf,
    /// The port it listens on, on localhost. Any free one by default; set
    /// one when a reverse proxy forwards the viewer.
    pub port: Option<u16>,
    /// Where users reach the service's `/view/` pages, such as
    /// `https://qmsg.example.com/browser` behind a reverse proxy. By default
    /// the service's own localhost address, which only works on this
    /// machine.
    pub viewer_url: Option<String>,
    /// How long the service may take to start.
    #[serde(with = "secs")]
    pub start_timeout: Duration,
    /// How long a login may take, at most; the service also ends its own.
    #[serde(with = "secs")]
    pub login_timeout: Duration,
    /// How often a running login is checked on.
    #[serde(skip)]
    pub poll_interval: Duration,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            command: vec!["bun".into(), "src/bootstrap.ts".into()],
            dir: "services/browser-login".into(),
            port: None,
            viewer_url: None,
            start_timeout: Duration::from_secs(30),
            login_timeout: Duration::from_secs(15 * 60),
            poll_interval: Duration::from_millis(500),
        }
    }
}

mod secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(d)?))
    }
}

/// The browser login service, started when first needed.
pub(crate) struct Browser {
    config: BrowserConfig,
    service: tokio::sync::Mutex<Option<Arc<Service>>>,
    slots: Arc<Semaphore>,
}

impl Browser {
    pub(crate) fn new(config: BrowserConfig) -> Self {
        Self {
            config,
            service: Default::default(),
            slots: Arc::new(Semaphore::new(MAX_BROWSER_LOGINS)),
        }
    }

    /// Starts a login with `recipe`. The caller shows the link, then waits
    /// with [`Login::finish`]. Dropping the login closes its browser.
    pub(crate) async fn start(&self, recipe: &str) -> Result<Login, String> {
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| format!("over {MAX_BROWSER_LOGINS} browser logins at once"))?;
        let service = self.service().await?;
        let (session, created) = service.create(recipe).await?;
        if created.id != session.id || !created.path.starts_with("/view/") {
            return Err("browser login service gave a bad viewer path".into());
        }
        let base = self.config.viewer_url.as_deref().unwrap_or(&service.url);
        let link = format!("{}{}", base.trim_end_matches('/'), created.path);
        Ok(Login {
            link,
            deadline: Instant::now() + self.config.login_timeout,
            poll: self.config.poll_interval,
            session: Some(session),
            _slot: slot,
        })
    }

    /// The running service, started if it isn't.
    async fn service(&self) -> Result<Arc<Service>, String> {
        let mut service = self.service.lock().await;
        if let Some(running) = &*service
            && running.alive()
        {
            return Ok(running.clone());
        }
        // Drop a dead one, which also stops what it left behind.
        *service = None;
        let config = self.config.clone();
        let started = tokio::task::spawn_blocking(move || Service::start(&config))
            .await
            .map_err(|_| "starting the browser login service panicked".to_owned())??;
        let started = Arc::new(started);
        *service = Some(started.clone());
        Ok(started)
    }
}

#[derive(Deserialize)]
struct Created {
    id: String,
    path: String,
}

#[derive(Deserialize)]
struct State {
    state: String,
}

#[derive(Deserialize)]
struct Claimed {
    token: String,
}

fn ok(response: reqwest::Response, doing: &str) -> Result<reqwest::Response, String> {
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        Err("too many browser logins are running".into())
    } else {
        Err(format!("{doing} failed: {status}"))
    }
}

/// One login in the service.
pub(crate) struct Login {
    /// The viewer page the user signs in on. It grants control of the login,
    /// so it is never logged.
    pub(crate) link: String,
    deadline: Instant,
    poll: Duration,
    /// Taken once claimed, so dropping doesn't cancel it.
    session: Option<Session>,
    _slot: OwnedSemaphorePermit,
}

struct Session {
    service: Arc<Service>,
    id: String,
    armed: bool,
}

impl Login {
    /// Waits for the user to sign in and returns the session token.
    pub(crate) async fn finish(mut self) -> Result<String, String> {
        let session = self.session.as_ref().expect("taken only here");
        let path = format!("sessions/{}", session.id);
        loop {
            if Instant::now() > self.deadline {
                return Err("the browser login timed out".into());
            }
            tokio::time::sleep(self.poll).await;
            let response = session
                .service
                .request(reqwest::Method::GET, &path)
                .send()
                .await
                .map_err(|e| format!("browser login service: {e}"))?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err("the browser login expired".into());
            }
            let state: State = ok(response, "checking the login")?
                .json()
                .await
                .map_err(|e| format!("browser login service: {e}"))?;
            match state.state.as_str() {
                "starting" | "active" => continue,
                "complete" => break,
                other => return Err(format!("the browser login {other}")),
            }
        }
        let response = session
            .service
            .request(reqwest::Method::POST, &format!("{path}/claim"))
            .send()
            .await
            .map_err(|e| format!("browser login service: {e}"))?;
        // Claiming ends the session in the service, even if reading fails.
        if let Some(mut session) = self.session.take() {
            session.armed = false;
        }
        let claimed: Claimed = ok(response, "claiming the login")?
            .json()
            .await
            .map_err(|e| format!("browser login service: {e}"))?;
        Ok(claimed.token)
    }
}

impl Drop for Session {
    /// Closes the browser of a login that didn't finish, such as when the
    /// provider gave up, disconnected or exited.
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let service = self.service.clone();
        let id = self.id.clone();
        runtime.spawn(async move {
            let delete = service
                .request(reqwest::Method::DELETE, &format!("sessions/{id}"))
                .send();
            if let Err(e) = tokio::time::timeout(Duration::from_secs(10), delete).await {
                tracing::warn!("cancelling a browser login: {e}");
            }
        });
    }
}

/// The running service process.
struct Service {
    /// Taken once stopping.
    process: Mutex<Option<Child>>,
    /// Its localhost address, ending in `/`.
    url: String,
    key: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct Started {
    url: String,
}

impl Service {
    async fn create(self: &Arc<Self>, recipe: &str) -> Result<(Session, Created), String> {
        // Know the session id before awaiting the POST: cancellation can
        // otherwise lose its answer after the service created a browser.
        let session = Session {
            service: self.clone(),
            id: crate::new_token().map_err(|e| e.to_string())?,
            armed: true,
        };
        let created: Created = self
            .request(reqwest::Method::POST, "sessions")
            .json(&serde_json::json!({ "provider": recipe, "id": session.id }))
            .send()
            .await
            .map_err(|e| format!("browser login service: {e}"))
            .and_then(|r| ok(r, "starting the login"))?
            .json()
            .await
            .map_err(|e| format!("browser login service: {e}"))?;
        Ok((session, created))
    }

    /// Starts the service and waits for the address it prints. Blocks.
    fn start(config: &BrowserConfig) -> Result<Self, String> {
        let (program, args) = config
            .command
            .split_first()
            .ok_or("no browser login service command")?;
        let key = crate::new_token().map_err(|e| e.to_string())?;
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(&config.dir)
            .env("QMSG_BROWSER_KEY", &key)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(port) = config.port {
            command.env("QMSG_BROWSER_PORT", port.to_string());
        }
        // Do not trust forwarded headers supplied by viewer requests.
        if let Some(viewer_url) = &config.viewer_url {
            let url = reqwest::Url::parse(viewer_url)
                .map_err(|e| format!("invalid browser viewer_url: {e}"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err("browser viewer_url must be an HTTP or HTTPS URL".into());
            }
            command.env(
                "QMSG_BROWSER_VIEWER_ORIGIN",
                url.origin().ascii_serialization(),
            );
        } else {
            command.env_remove("QMSG_BROWSER_VIEWER_ORIGIN");
        }
        // Its own process group, so stopping it also stops its browsers.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let mut child = command.spawn().map_err(|e| {
            format!(
                "starting the browser login service `{program}` in {}: {e}",
                config.dir.display()
            )
        })?;
        let stdout = child.stdout.take().expect("piped");
        let (first, line) = oneshot::channel();
        // Reads the address, then keeps the pipe drained.
        thread::spawn(move || {
            let mut lines = BufReader::new(stdout).lines();
            let _ = first.send(lines.next());
            for line in lines.map_while(Result::ok) {
                tracing::debug!("browser login service: {line}");
            }
        });
        // The process is stopped if anything below fails.
        let mut service = Self {
            process: Mutex::new(Some(child)),
            url: String::new(),
            key,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .no_proxy()
                .build()
                .map_err(|e| e.to_string())?,
        };
        let line = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(async { tokio::time::timeout(config.start_timeout, line).await });
        let line = match line {
            Ok(Ok(Some(Ok(line)))) => line,
            Err(_) => return Err("the browser login service didn't start in time".into()),
            _ => return Err("the browser login service exited while starting".into()),
        };
        let started: Started = serde_json::from_str(&line)
            .map_err(|_| "the browser login service printed no address".to_owned())?;
        let url = reqwest::Url::parse(&started.url).map_err(|e| e.to_string())?;
        // The key is sent to it, so it must be this machine.
        let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
        if url.scheme() != "http" || !local {
            return Err(format!(
                "the browser login service must listen on localhost, not {url}"
            ));
        }
        service.url = format!("{}/", started.url.trim_end_matches('/'));
        Ok(service)
    }

    fn alive(&self) -> bool {
        let mut process = self.process.lock().unwrap();
        matches!(process.as_mut().map(Child::try_wait), Some(Ok(None)))
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.url))
            .bearer_auth(&self.key)
    }
}

impl Drop for Service {
    /// Asks the service to close its browsers, then kills what's left after
    /// a few seconds, without blocking the caller.
    fn drop(&mut self) {
        let Some(mut child) = self.process.get_mut().unwrap().take() else {
            return;
        };
        #[cfg(unix)]
        let group = child.id() as i32;
        #[cfg(unix)]
        // SAFETY: kill has no memory effects; the group is the service's own.
        unsafe {
            libc::kill(-group, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = child.kill();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if !matches!(child.try_wait(), Ok(None)) {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            #[cfg(unix)]
            // SAFETY: as above. Also stops browsers that outlived it.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn cancelled_creation_closes_a_login_before_its_reply_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (created_tx, created) = oneshot::channel();
        let (deleted_tx, deleted) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut post, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let id = loop {
                let mut part = [0; 1024];
                let n = post.read(&mut part).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&part[..n]);
                if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n")
                    && let Ok(body) =
                        serde_json::from_slice::<serde_json::Value>(&request[at + 4..])
                {
                    break body["id"].as_str().unwrap().to_owned();
                }
            };
            created_tx.send(id.clone()).unwrap();
            // The POST's answer is withheld after the service created the
            // session. Cancellation must still issue its DELETE.
            let (mut delete, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 2048];
            let n = delete.read(&mut bytes).await.unwrap();
            assert!(
                String::from_utf8_lossy(&bytes[..n])
                    .starts_with(&format!("DELETE /sessions/{id} HTTP/1.1"))
            );
            delete
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .await
                .unwrap();
            deleted_tx.send(()).unwrap();
        });
        let service = Arc::new(Service {
            process: Mutex::new(None),
            url: format!("http://{address}/"),
            key: "test-key".into(),
            client: reqwest::Client::new(),
        });
        let create = tokio::spawn(async move { service.create("discord").await });
        tokio::time::timeout(Duration::from_secs(5), created)
            .await
            .unwrap()
            .unwrap();
        create.abort();
        assert!(create.await.is_err());
        tokio::time::timeout(Duration::from_secs(5), deleted)
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
    }
}
