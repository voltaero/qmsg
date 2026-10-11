//! Builds Go providers and runs `fakediscord`, for the tests of Go
//! providers. Needs Go, and network access the first time to fetch the
//! pinned tools; see `sdk/go/build.sh`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command as Process, Stdio};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn target() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(|dir| root().join(dir))
        .unwrap_or_else(|| root().join("target"))
}

/// Builds the Go provider in `dir`, relative to the repository, into a
/// component, once per test binary.
pub fn go_component(dir: &str) -> PathBuf {
    static BUILT: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let mut built = BUILT.get_or_init(Default::default).lock().unwrap();
    built
        .entry(dir.to_owned())
        .or_insert_with(|| {
            let wasm = target()
                .join("go")
                .join(format!("{}.wasm", dir.replace('/', "-")));
            let status = Process::new(root().join("sdk/go/build.sh"))
                .arg(root().join(dir))
                .arg(&wasm)
                .status()
                .expect("running sdk/go/build.sh");
            assert!(status.success(), "building {dir} failed");
            wasm
        })
        .clone()
}

/// The `fakediscord` binary.
pub fn fakediscord() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let fake = target().join("go/fakediscord");
        let status = Process::new("go")
            .args(["build", "-o"])
            .arg(&fake)
            .arg("./fakediscord")
            .current_dir(root().join("providers/discord"))
            .status()
            .expect("running go");
        assert!(status.success(), "building fakediscord failed");
        fake
    })
}

#[derive(Deserialize)]
pub struct FakeAddrs {
    /// The TLS server, for discord.com and gateway.discord.gg.
    pub tls: String,
    pub control: String,
    /// The PEM of the CA its certificate is from.
    pub ca: String,
}

/// A running fakediscord, stopped on drop.
pub struct Fake {
    process: Child,
    pub addrs: FakeAddrs,
    pub http: reqwest::Client,
}

impl Fake {
    pub fn start() -> Self {
        let mut process = Process::new(fakediscord())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(process.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        Self {
            process,
            addrs: serde_json::from_str(&line).unwrap(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn control(&self, path: &str) -> serde_json::Value {
        let response = self
            .http
            .post(format!("{}{path}", self.addrs.control))
            .send()
            .await
            .unwrap();
        response.json().await.unwrap_or(serde_json::Value::Null)
    }

    pub async fn requests(&self) -> Vec<String> {
        let state = self.control("/state").await;
        serde_json::from_value(state["requests"].clone()).unwrap()
    }

    pub async fn stat(&self, name: &str) -> u64 {
        self.control("/state").await["stats"][name]
            .as_u64()
            .unwrap_or(0)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
