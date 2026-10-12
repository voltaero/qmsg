use anyhow::Context as _;
use qmsg_orchestrator::{BrowserConfig, Encryption, Orchestrator, ProviderSpec, app};
use serde::Deserialize;
use std::path::PathBuf;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    data_dir: Option<PathBuf>,
    #[serde(default)]
    browser: BrowserConfig,
    #[serde(default)]
    app: app::Config,
    #[serde(default, rename = "provider")]
    providers: Vec<ProviderSpec>,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap_or_else(|| "orchestrator.toml".into());
    if first == "schema" {
        println!("{}", serde_json::to_string_pretty(&app::schema())?);
        return Ok(());
    }
    if first == "tls" {
        let dir = PathBuf::from(
            args.next()
                .context("usage: qmsg-orchestrator tls DATA_DIR HOST [HOST...]")?,
        );
        let hosts: Vec<_> = args.collect();
        anyhow::ensure!(!hosts.is_empty(), "supply server host names or LAN IPs");
        let (cert, key) = app::create_identity(&dir, hosts)?;
        println!("certificate = {:?}\nprivate_key = {:?}", cert, key);
        return Ok(());
    }
    let pair = first == "pair";
    let path = PathBuf::from(if pair {
        args.next().unwrap_or_else(|| "orchestrator.toml".into())
    } else {
        first
    });
    let config: Config = toml::from_str(
        &tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("reading {}", path.display()))?,
    )?;
    let data_dir = match config.data_dir {
        Some(dir) => dir,
        None => std::env::home_dir()
            .context("set data_dir: no home directory")?
            .join(".qmsg"),
    };
    let json_only = args.any(|arg| arg == "--json");
    if pair {
        let value = app::pairing(&data_dir, &config.app)?;
        if !json_only {
            let qr = qrcode::QrCode::new(value.as_bytes())?;
            println!(
                "{}",
                qr.render::<qrcode::render::unicode::Dense1x2>()
                    .quiet_zone(true)
                    .build()
            );
        }
        println!("{value}");
        return Ok(());
    }
    config.app.validate()?;
    let mut orchestrator = Orchestrator::new(&data_dir, Encryption::Keychain).await?;
    orchestrator.set_browser(config.browser);
    let service = app::Service::managed(&data_dir, orchestrator, config.providers).await?;
    let stopper = service.stopper();
    let result = tokio::select! {
        result = service.serve(config.app) => result,
        _ = stop_signal() => {
            tracing::info!("shutting down");
            Ok(())
        }
    };
    stopper.stop().await;
    result
}

/// Ctrl-C, or SIGTERM from a service manager.
async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            },
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
