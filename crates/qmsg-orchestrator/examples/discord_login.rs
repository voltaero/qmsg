//! Live Discord smoke test. Build the Go component first, then run from the repo root.
//! Prints status and counts, never tokens or message content. Ctrl-C stops it.
use std::{path::PathBuf, sync::Arc, time::Duration};

use qmsg_orchestrator::{BrowserConfig, Encryption, Orchestrator, ProviderEvent, ProviderSpec};
use qmsg_types::{Arguments, FunctionContext, ProviderStatus};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let data = PathBuf::from("target/live-discord");
    let mut host = Orchestrator::new(&data, Encryption::Keychain).await?;
    host.set_browser(BrowserConfig::default());
    let (tx, mut events) = mpsc::channel(256);
    let mut provider = host.spawn(
        ProviderSpec {
            name: "discord".into(),
            wasm: "target/go/providers-discord.wasm".into(),
            settings: Default::default(),
        },
        tx,
    )?;
    provider.set_timeout(Some(Duration::from_secs(15 * 60)));
    let provider = Arc::new(provider);
    let mut login = None;
    let mut directory_events = 0usize;
    println!("Starting Discord WASM provider; session storage uses the OS keychain.");
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            Some(event) = events.recv() => match event {
                ProviderEvent::Status { status, .. } => match status {
                    ProviderStatus::LoginRequired(step) => {
                        if let Some(link) = step.link { println!("LOGIN_VIEWER={link}"); }
                        if let Some(name) = step.function && login.is_none() {
                            let provider = provider.clone();
                            login = Some(tokio::spawn(async move {
                                let functions = provider.functions(FunctionContext::Provider).await?;
                                let function = functions.iter().find(|f| f.id == name)
                                    .ok_or_else(|| anyhow::anyhow!("login function missing"))?;
                                provider.call_function(function, FunctionContext::Provider, Arguments::new()).await?;
                                println!("Login completed; waiting for gateway readiness.");
                                Ok::<_, anyhow::Error>(())
                            }));
                        }
                    }
                    ProviderStatus::Ready => {
                        println!("READY: gateway connected; {directory_events} directory updates received.");
                        let provider = provider.clone();
                        tokio::spawn(async move {
                            match provider.functions(FunctionContext::Provider).await {
                                Ok(functions) => println!("Provider function discovery passed: {} declarations.", functions.len()),
                                Err(error) => eprintln!("Provider function discovery failed: {error}"),
                            }
                        });
                    }
                    other => println!("Status: {other:?}"),
                },
                ProviderEvent::Directory { .. } => directory_events += 1,
                ProviderEvent::Message { .. } => {},
                ProviderEvent::Exited { result, .. } => {
                    println!("Provider exited: {result:?}");
                    break;
                }
            },
            result = async { login.as_mut().unwrap().await }, if login.is_some() => {
                result??;
                login = None;
            }
        }
    }
    if let Some(login) = login {
        login.abort();
    }
    provider.kill();
    Ok(())
}
