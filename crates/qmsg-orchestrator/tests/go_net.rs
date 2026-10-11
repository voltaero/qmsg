//! Checks Go providers' networking under the orchestrator's Wasmtime: name
//! lookups, TCP, TLS, HTTP and WebSocket, timeouts, cancellation, and that
//! sockets are freed. Runs `sdk/go/internal/nettest`, whose `probe` lookup
//! reports what one check did, against local servers.

use std::time::{Duration, Instant};

use qmsg_orchestrator::{Encryption, Orchestrator, ProviderEvent, ProviderSpec};
use qmsg_types::{Arguments, Function, FunctionContext, ProviderStatus, Value};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;

mod support;
use support::{Fake, go_component};

fn probe_args(kind: &str, target: &str) -> Arguments {
    [
        ("kind".to_owned(), Value::Text(kind.into())),
        ("target".to_owned(), Value::Text(target.into())),
    ]
    .into()
}

/// A server that accepts connections and never answers. Reports each
/// connection's end.
async fn silent_server() -> (String, mpsc::UnboundedReceiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (closed_tx, closed) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let closed = closed_tx.clone();
            tokio::spawn(async move {
                let mut buf = [0; 64];
                while matches!(socket.read(&mut buf).await, Ok(n) if n > 0) {}
                let _ = closed.send(());
            });
        }
    });
    (addr, closed)
}

#[tokio::test(flavor = "multi_thread")]
async fn go_networking_works_in_wasm() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = Orchestrator::new(dir.path(), Encryption::Plaintext)
        .await
        .unwrap();
    let (events_tx, mut events) = mpsc::channel(16);
    let spec = ProviderSpec {
        name: "nettest".into(),
        wasm: go_component("sdk/go/internal/nettest"),
        settings: [
            ("tls".to_owned(), fake.addrs.tls.clone()),
            ("ca".to_owned(), fake.addrs.ca.clone()),
        ]
        .into(),
    };
    let provider = orchestrator.spawn(spec, events_tx).unwrap();
    let ready = timeout(Duration::from_secs(300), events.recv())
        .await
        .unwrap();
    assert!(
        matches!(
            ready,
            Some(ProviderEvent::Status {
                status: ProviderStatus::Ready,
                ..
            })
        ),
        "{ready:?}"
    );
    let functions = provider.functions(FunctionContext::Provider).await.unwrap();
    let probe: Function = functions[0].clone();
    let run = |kind: &'static str, target: String| {
        let probe = probe.clone();
        let provider = &provider;
        async move {
            let Ok(Value::Text(out)) = provider
                .call_function(&probe, FunctionContext::Provider, probe_args(kind, &target))
                .await
            else {
                panic!("probe {kind} failed");
            };
            out
        }
    };

    // Name lookups through wasi:sockets.
    let out = run("resolve", "localhost".into()).await;
    assert!(out.contains("127.0.0.1") || out.contains("::1"), "{out}");
    let out = run("resolve", "nonexistent.invalid".into()).await;
    assert!(out.starts_with("error: "), "{out}");

    // Connection errors and timeouts come back as errors, in time.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let refused = closed.local_addr().unwrap().to_string();
    drop(closed);
    let out = run("dial", refused).await;
    assert!(out.starts_with("error: "), "{out}");
    // Unanswered connects give up at their deadline (here the address may
    // also be unreachable), and the provider carries on.
    let start = Instant::now();
    let out = run("dial", "10.255.255.1:9".into()).await;
    assert!(out.starts_with("error: "), "{out}");
    assert!(start.elapsed() < Duration::from_secs(2), "{out}");
    // A connect given up on by cancelling the request, and the provider
    // carries on.
    let dial = provider.call_function(
        &probe,
        FunctionContext::Provider,
        probe_args("dial-until-cancelled", "10.255.255.1:9"),
    );
    assert!(timeout(Duration::from_millis(300), dial).await.is_err());
    // Long enough for the poller to have checked what is waiting since.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let out = run("resolve", "localhost".into()).await;
    assert!(!out.starts_with("error"), "{out}");
    let (silent, mut ended) = silent_server().await;
    let out = run("read-timeout", silent.clone()).await;
    assert!(out.contains("timeout") || out.contains("deadline"), "{out}");
    timeout(Duration::from_secs(5), ended.recv())
        .await
        .expect("closed after the timeout");

    // Deadlines apply to operations already waiting, including when the
    // old deadline is extended or cleared.
    for kind in ["deadline-update", "deadline-extend", "deadline-clear"] {
        let out = run(kind, silent.clone()).await;
        assert!(out.starts_with("ok"), "{kind}: {out}");
        timeout(Duration::from_secs(5), ended.recv())
            .await
            .expect("closed after updating the deadline");
    }

    // Sockets are freed after use.
    let out = run("churn", silent.clone()).await;
    assert!(out.starts_with("ok"), "{out}");
    for _ in 0..300 {
        timeout(Duration::from_secs(10), ended.recv())
            .await
            .expect("each connection closed");
    }

    // One of the SDK's 256 is the connection to the orchestrator.
    let out = run("conns", silent.clone()).await;
    assert!(
        out.starts_with("255 open, then dial tcp: too many open connections"),
        "{out}"
    );
    for _ in 0..255 {
        timeout(Duration::from_secs(10), ended.recv())
            .await
            .expect("each connection closed");
    }

    // HTTPS with a large body, read through the bounded host buffers, and a
    // WebSocket over TLS. Certificates are checked against the host name.
    let out = run("https", "https://discord.com/api/v9/big?n=8000000".into()).await;
    assert!(out.starts_with("200 8000000 "), "{out}");
    let out = run("wss", "wss://gateway.discord.gg/?v=9".into()).await;
    assert!(out.starts_with("op 10, op 11"), "{out}");
    let out = run("https", "https://localhost/api/v9/gateway".into()).await;
    assert!(out.contains("certificate"), "{out}");

    // A blocked read neither blocks other requests nor outlives its
    // cancellation.
    let hang = provider.call_function(
        &probe,
        FunctionContext::Provider,
        probe_args("hang", &silent),
    );
    assert!(timeout(Duration::from_millis(300), hang).await.is_err());
    timeout(Duration::from_secs(5), ended.recv())
        .await
        .expect("the cancelled read closed its socket");
    let start = Instant::now();
    provider.functions(FunctionContext::Provider).await.unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));

    // Shutdown ends the provider promptly.
    provider.shutdown().unwrap();
    loop {
        match timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
        {
            Some(ProviderEvent::Exited { result, .. }) => {
                assert_eq!(result, Ok(()));
                break;
            }
            Some(_) => {}
            None => panic!("no exit"),
        }
    }
}
