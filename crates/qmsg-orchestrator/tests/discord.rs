//! Runs the Go Discord provider, a Wasm component built from
//! `providers/discord` with `sdk/go/build.sh`, against `fakediscord`: a
//! Discord-like REST API and gateway over TLS on localhost, and a stand-in
//! for the browser login service. No real account or network is used.
//!
//! Needs Go, and network access the first time to fetch the pinned tools.

use std::path::Path;
use std::time::Duration;

use qmsg_orchestrator::{
    BrowserConfig, Encryption, Orchestrator, ProviderEvent, ProviderHandle, ProviderSpec,
    RequestError,
};
use qmsg_types::{
    ActionKind, Arguments, ChannelRef, CommandError, CompletionRequest, Content, DirectoryUpdate,
    Function, FunctionCall, FunctionContext, FunctionKind, MessageEvent, Organization,
    ProviderStatus, Value, Verification, VerificationRequest, inputs,
};
use tokio::sync::mpsc;
use tokio::time::timeout;

mod support;
use support::{Fake, fakediscord, go_component, root};

fn spec(fake: &Fake, name: &str) -> ProviderSpec {
    ProviderSpec {
        name: name.into(),
        wasm: go_component("providers/discord"),
        settings: [
            (
                "test_dial".to_owned(),
                // Through a name lookup for the REST API.
                format!(
                    "discord.com=localhost:{},gateway.discord.gg={}",
                    fake.addrs.tls.rsplit(':').next().unwrap(),
                    fake.addrs.tls
                ),
            ),
            ("test_ca_pem".to_owned(), fake.addrs.ca.clone()),
        ]
        .into(),
    }
}

/// An orchestrator whose browser login service is fakediscord's.
async fn orchestrator(dir: &Path) -> Orchestrator {
    let mut orchestrator = Orchestrator::new(dir, Encryption::Plaintext).await.unwrap();
    orchestrator.set_browser(BrowserConfig {
        command: vec![
            fakediscord().to_string_lossy().into_owned(),
            "browser".into(),
        ],
        dir: root(),
        poll_interval: Duration::from_millis(50),
        ..BrowserConfig::default()
    });
    orchestrator
}

/// Long, since the first test builds the provider and each spawn compiles
/// it.
async fn next(events: &mut mpsc::Receiver<ProviderEvent>) -> ProviderEvent {
    timeout(Duration::from_secs(300), events.recv())
        .await
        .expect("timed out waiting for provider event")
        .expect("event channel closed")
}

/// The next status, skipping other events.
async fn status(events: &mut mpsc::Receiver<ProviderEvent>) -> ProviderStatus {
    loop {
        match next(events).await {
            ProviderEvent::Status { status, .. } => return status,
            ProviderEvent::Exited { result, .. } => panic!("provider exited: {result:?}"),
            _ => {}
        }
    }
}

async fn message_event(events: &mut mpsc::Receiver<ProviderEvent>) -> MessageEvent {
    loop {
        match next(events).await {
            ProviderEvent::Message { event, .. } => return event,
            ProviderEvent::Exited { result, .. } => panic!("provider exited: {result:?}"),
            _ => {}
        }
    }
}

/// Reads a sync through Ready, returning the guild.
async fn synced(events: &mut mpsc::Receiver<ProviderEvent>) -> Organization {
    let mut guild = None;
    let mut me = None;
    loop {
        match next(events).await {
            ProviderEvent::Directory { update, .. } => match update {
                DirectoryUpdate::OrganizationUpserted(o) => guild = Some(o),
                DirectoryUpdate::Me {
                    organization: None,
                    id,
                } => me = id,
                _ => {}
            },
            ProviderEvent::Status {
                status: ProviderStatus::Ready,
                ..
            } => break,
            ProviderEvent::Status {
                status: ProviderStatus::Syncing | ProviderStatus::Connecting,
                ..
            } => {}
            other => panic!("unexpected {other:?} while syncing"),
        }
    }
    assert_eq!(me.as_deref(), Some("1001"));
    guild.expect("the guild")
}

fn find<'a>(functions: &'a [Function], id: &str) -> &'a Function {
    functions
        .iter()
        .find(|f| f.id == id)
        .unwrap_or_else(|| panic!("no function `{id}` in {functions:?}"))
}

fn general() -> ChannelRef {
    ChannelRef::new(Some("2001"), "3001")
}

fn text(s: &str) -> Vec<Content> {
    vec![Content::Text(s.into())]
}

/// Waits for the login link, then finishes the login in the fake browser.
async fn sign_in(fake: &Fake, events: &mut mpsc::Receiver<ProviderEvent>, token: &str) {
    let link = browser_link(events).await;
    fake.http
        .post(format!("{link}/complete?token={token}"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}

async fn browser_link(events: &mut mpsc::Receiver<ProviderEvent>) -> String {
    loop {
        if let ProviderStatus::LoginRequired(step) = status(events).await
            && let Some(link) = step.link
        {
            assert!(link.starts_with("http://127.0.0.1:"), "{link}");
            assert!(step.function.is_none());
            return link;
        }
    }
}

/// Calls the login action, which waits for the browser login.
fn start_login(
    provider: &std::sync::Arc<ProviderHandle>,
    functions: &[Function],
) -> tokio::task::JoinHandle<Result<Value, RequestError>> {
    let login = find(functions, "login").clone();
    assert!(matches!(
        login.kind,
        FunctionKind::Action {
            action: ActionKind::Login,
            ..
        }
    ));
    let provider = provider.clone();
    tokio::spawn(async move {
        provider
            .call_function(&login, FunctionContext::Provider, Arguments::new())
            .await
    })
}

async fn login_required(events: &mut mpsc::Receiver<ProviderEvent>) {
    loop {
        if let ProviderStatus::LoginRequired(step) = status(events).await {
            assert_eq!(step.function.as_deref(), Some("login"));
            assert!(step.link.is_none());
            return;
        }
    }
}

async fn browser_events(link: &str) -> Vec<String> {
    let base = &link[..link.find("/view/").unwrap()];
    reqwest::get(format!("{base}/events"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_login_then_a_restored_session() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (events_tx, mut events) = mpsc::channel(64);
    let provider = std::sync::Arc::new(
        orchestrator
            .spawn(spec(&fake, "discord"), events_tx.clone())
            .unwrap(),
    );

    login_required(&mut events).await;
    assert_eq!(
        provider.history(general(), None, 10).await,
        Err(RequestError::Failed(CommandError::LoginRequired))
    );
    let functions = provider.functions(FunctionContext::Provider).await.unwrap();
    assert_eq!(functions.len(), 1, "only signing in: {functions:?}");
    let login = start_login(&provider, &functions);
    sign_in(&fake, &mut events, "fake-token").await;
    assert_eq!(login.await.unwrap(), Ok(Value::Null));

    let guild = synced(&mut events).await;
    assert_eq!(guild.id, "2001");
    let channels: Vec<_> = guild.channels.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(channels, ["3001", "3002"], "the hidden channel is left out");
    // Channels that can't be sent to accept nothing.
    assert!(!guild.channels[0].accepted_content.is_empty());
    assert!(guild.channels[1].accepted_content.is_empty());
    assert_eq!(guild.users.len(), 2);
    assert_eq!(fake.stat("identify").await, 1);

    // The token is a secret now, so a new instance connects without a login.
    drop(provider);
    loop {
        if let ProviderEvent::Exited { .. } = next(&mut events).await {
            break;
        }
    }
    let provider = timeout(Duration::from_secs(30), async {
        loop {
            match orchestrator.spawn(spec(&fake, "discord"), events_tx.clone()) {
                Ok(provider) => return provider,
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .unwrap();
    synced(&mut events).await;
    assert_eq!(provider.status(), ProviderStatus::Ready);
    assert_eq!(fake.stat("identify").await, 2);
}

/// Spawns a provider and signs it in through the fake browser.
async fn signed_in(
    fake: &Fake,
    orchestrator: &Orchestrator,
) -> (
    std::sync::Arc<ProviderHandle>,
    mpsc::Receiver<ProviderEvent>,
) {
    let (events_tx, mut events) = mpsc::channel(256);
    let provider = std::sync::Arc::new(
        orchestrator
            .spawn(spec(fake, "discord"), events_tx)
            .unwrap(),
    );
    login_required(&mut events).await;
    let functions = provider.functions(FunctionContext::Provider).await.unwrap();
    let login = start_login(&provider, &functions);
    sign_in(fake, &mut events, "fake-token").await;
    assert_eq!(login.await.unwrap(), Ok(Value::Null));
    synced(&mut events).await;
    (provider, events)
}

fn args<const N: usize>(arguments: [(&str, Value); N]) -> Arguments {
    arguments.into_iter().map(|(k, v)| (k.into(), v)).collect()
}

fn ids(functions: &[Function]) -> Vec<&str> {
    functions.iter().map(|f| f.id.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_actions_and_lookups() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;

    // History pages newest first.
    let first = provider.history(general(), None, 3).await.unwrap();
    let texts = |page: &qmsg_types::HistoryPage| -> Vec<Vec<Content>> {
        page.messages.iter().map(|m| m.content.clone()).collect()
    };
    assert_eq!(texts(&first), [text("old 4"), text("old 3"), text("old 2")]);
    let second = provider
        .history(general(), first.next_cursor.clone(), 3)
        .await
        .unwrap();
    assert_eq!(texts(&second), [text("old 1"), text("old 0")]);
    assert_eq!(second.next_cursor, None);
    let old = &first.messages[0];
    assert_eq!(old.author, "1002");
    assert_eq!(old.channel, general());
    assert_eq!(
        provider
            .get_message(general(), old.id.clone())
            .await
            .unwrap(),
        *old
    );
    assert_eq!(
        provider.get_message(general(), "1".into()).await,
        Err(RequestError::Failed(CommandError::UnknownMessage(
            "1".into()
        )))
    );
    let hidden = ChannelRef::new(Some("2001"), "3003");
    assert_eq!(
        provider.history(hidden.clone(), None, 3).await,
        Err(RequestError::Failed(CommandError::UnknownChannel(hidden)))
    );

    // Live messages, typing and permissions.
    fake.control("/message?channel=3001&content=hi").await;
    let MessageEvent::Received(live) = message_event(&mut events).await else {
        panic!("expected a message");
    };
    assert_eq!(
        (live.author.as_str(), live.content.clone()),
        ("1002", text("hi"))
    );
    fake.control("/typing").await;
    assert_eq!(
        message_event(&mut events).await,
        MessageEvent::Typing {
            channel: general(),
            user: "1002".into()
        }
    );
    let announcements = ChannelRef::new(Some("2001"), "3002");
    let functions = provider
        .functions(FunctionContext::Channel(announcements.clone()))
        .await
        .unwrap();
    assert!(functions.is_empty(), "{functions:?}");
    assert!(matches!(
        provider
            .send_message(announcements, None, text("x"), None)
            .await,
        Err(RequestError::Failed(CommandError::Forbidden(_)))
    ));

    // Sending, with the nonce on the message that comes back.
    let in_general = FunctionContext::Channel(general());
    let functions = provider.functions(in_general.clone()).await.unwrap();
    assert_eq!(ids(&functions), ["send", "typing"]);
    assert_eq!(
        provider
            .call_function(
                find(&functions, "typing"),
                in_general.clone(),
                Arguments::new()
            )
            .await,
        Ok(Value::Null)
    );
    let id = provider
        .send_message(
            general(),
            Some(live.id.clone()),
            text("hello"),
            Some("n-1".into()),
        )
        .await
        .unwrap();
    let MessageEvent::Received(sent) = message_event(&mut events).await else {
        panic!("expected the sent message");
    };
    assert_eq!(sent.id, id);
    assert_eq!(sent.author, "1001");
    assert_eq!(sent.nonce.as_deref(), Some("n-1"));
    let send = find(&functions, "send");
    let Value::Text(id2) = provider
        .call_function(
            send,
            in_general.clone(),
            args([(inputs::CONTENT, Value::Content(text("again")))]),
        )
        .await
        .unwrap()
    else {
        panic!("expected the id");
    };
    message_event(&mut events).await;

    // Actions on our own message and on someone else's.
    let own = FunctionContext::Message {
        channel: general(),
        id: id.clone(),
    };
    let functions = provider.functions(own.clone()).await.unwrap();
    assert_eq!(
        ids(&functions),
        ["edit", "delete", "react", "unreact", "mark_read"]
    );
    let theirs = FunctionContext::Message {
        channel: general(),
        id: live.id.clone(),
    };
    let their_functions = provider.functions(theirs.clone()).await.unwrap();
    assert_eq!(ids(&their_functions), ["react", "unreact", "mark_read"]);

    provider
        .call_function(
            find(&functions, "edit"),
            own.clone(),
            args([(inputs::CONTENT, Value::Content(text("edited")))]),
        )
        .await
        .unwrap();
    assert!(matches!(message_event(&mut events).await,
        MessageEvent::Edited { id: edited, content, .. } if edited == id && content == text("edited")));
    let thumbs = args([(inputs::REACTION, Value::Text("👍".into()))]);
    provider
        .call_function(
            find(&their_functions, "react"),
            theirs.clone(),
            thumbs.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        message_event(&mut events).await,
        MessageEvent::Reacted {
            channel: general(),
            id: live.id.clone(),
            user: "1001".into(),
            key: "👍".into(),
            added: true
        }
    );
    provider
        .call_function(find(&their_functions, "unreact"), theirs.clone(), thumbs)
        .await
        .unwrap();
    assert!(matches!(
        message_event(&mut events).await,
        MessageEvent::Reacted { added: false, .. }
    ));
    provider
        .call_function(
            find(&their_functions, "mark_read"),
            theirs,
            Arguments::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        message_event(&mut events).await,
        MessageEvent::Read {
            channel: general(),
            user: "1001".into(),
            up_to: Some(live.id.clone())
        }
    );
    provider
        .call_function(find(&functions, "delete"), own, Arguments::new())
        .await
        .unwrap();
    assert_eq!(
        message_event(&mut events).await,
        MessageEvent::Deleted {
            channel: general(),
            id: id.clone()
        }
    );
    assert_eq!(
        provider.get_message(general(), id.clone()).await,
        Err(RequestError::Failed(CommandError::UnknownMessage(id)))
    );
    assert!(!id2.is_empty());

    // Lookups, verification and completion of users.
    let functions = provider.functions(FunctionContext::Provider).await.unwrap();
    assert_eq!(
        ids(&functions),
        [
            "logout",
            "open_dm",
            "find_users",
            "verify_user",
            "complete_user"
        ]
    );
    let Value::List(found) = provider
        .call_function(
            find(&functions, "find_users"),
            FunctionContext::Provider,
            args([("query", Value::Text("fri".into()))]),
        )
        .await
        .unwrap()
    else {
        panic!("expected users");
    };
    assert_eq!(found.len(), 1);
    let call = |function: &str| FunctionCall {
        function: function.into(),
        context: FunctionContext::Provider,
        arguments: Arguments::new(),
    };
    for (user, valid) in [("1002", true), ("999", false), ("x", false)] {
        let verified = provider
            .verify(
                find(&functions, "verify_user"),
                VerificationRequest {
                    call: call("verify_user"),
                    input: None,
                    value: Value::Text(user.into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            verified == Verification::Valid,
            valid,
            "{user}: {verified:?}"
        );
    }
    let page = provider
        .complete(
            find(&functions, "complete_user"),
            CompletionRequest {
                call: call("complete_user"),
                input: None,
                query: "fr".into(),
                cursor: None,
                limit: 5,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].value, Value::Text("1002".into()));

    // Direct messages, which belong to no organization.
    let dm = ChannelRef::new(None::<&str>, "3100");
    assert_eq!(
        provider.open_channel(None, vec!["1002".into()]).await,
        Ok(dm.clone())
    );
    assert!(matches!(
        provider.open_channel(None, vec!["999".into()]).await,
        Err(RequestError::Failed(CommandError::InvalidInput(_)))
    ));
    provider
        .send_message(dm.clone(), None, text("psst"), None)
        .await
        .unwrap();
    loop {
        if let MessageEvent::Received(m) = message_event(&mut events).await {
            assert_eq!(m.channel, dm);
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnects_cancels_and_logs_out() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;

    // A dropped gateway resumes, and live events keep coming.
    fake.control("/drop?code=4000").await;
    assert!(matches!(
        status(&mut events).await,
        ProviderStatus::Disconnected { .. }
    ));
    assert_eq!(status(&mut events).await, ProviderStatus::Ready);
    assert_eq!(fake.stat("resume").await, 1);
    fake.control("/message?channel=3001&content=after").await;
    let MessageEvent::Received(m) = message_event(&mut events).await else {
        panic!("expected a message");
    };
    assert_eq!(m.content, text("after"));

    // Cancelling an already submitted HTTP request leaves delivery unknown,
    // even when this fake happens to stop before accepting it.
    fake.control("/delay?ms=10000").await;
    let send = provider.send_message(general(), None, text("slow"), Some("n-slow".into()));
    assert!(timeout(Duration::from_millis(500), send).await.is_err());
    assert!(
        timeout(Duration::from_millis(500), message_event(&mut events))
            .await
            .is_err()
    );
    fake.control("/delay?ms=0").await;
    let page = provider.history(general(), None, 1).await.unwrap();
    assert_eq!(page.messages[0].content, text("after"), "never sent");
    assert!(
        fake.requests()
            .await
            .contains(&"POST /channels/3001/messages".to_owned())
    );

    // A revoked session logs out, forgetting the token.
    fake.control("/revoke").await;
    fake.control("/drop?code=4004").await;
    login_required(&mut events).await;
    assert_eq!(
        provider.history(general(), None, 1).await,
        Err(RequestError::Failed(CommandError::LoginRequired))
    );

    // Giving up on a login closes its browser; one runs at a time.
    let functions = provider.functions(FunctionContext::Provider).await.unwrap();
    let login = start_login(&provider, &functions);
    let link = browser_link(&mut events).await;
    let again = start_login(&provider, &functions);
    assert!(matches!(again.await.unwrap(),
        Err(RequestError::Failed(CommandError::Failed(reason))) if reason.contains("already")));
    let created = browser_events(&link)
        .await
        .into_iter()
        .filter_map(|event| event.strip_prefix("create ").map(str::to_owned))
        .next_back()
        .unwrap();
    login.abort();
    login_required(&mut events).await;
    timeout(Duration::from_secs(10), async {
        while !browser_events(&link)
            .await
            .contains(&format!("delete {created}"))
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the login was closed");

    // So does the provider exiting while one runs.
    let login = start_login(&provider, &functions);
    let link = browser_link(&mut events).await;
    let created = browser_events(&link)
        .await
        .into_iter()
        .filter_map(|event| event.strip_prefix("create ").map(str::to_owned))
        .next_back()
        .unwrap();
    provider.kill();
    let _ = login.await;
    timeout(Duration::from_secs(10), async {
        while !browser_events(&link)
            .await
            .contains(&format!("delete {created}"))
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the login was closed");

    // Dropping the orchestrator stops the service.
    drop(orchestrator);
    timeout(Duration::from_secs(10), async {
        while reqwest::get(&link).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the service stopped");
}

/// With the real browser login service in `services/browser-login`: the
/// orchestrator starts it for the login, links its viewer, closes the login
/// when the provider gives up, and stops it when dropped. Needs Bun, the
/// service's packages and Playwright's Chromium; Discord's page itself may
/// not load. Run with `cargo test --test discord -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Bun and Chromium"]
async fn the_real_browser_service_is_started_linked_and_stopped() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let mut orchestrator = Orchestrator::new(dir.path(), Encryption::Plaintext)
        .await
        .unwrap();
    orchestrator.set_browser(BrowserConfig {
        dir: root().join("services/browser-login"),
        ..BrowserConfig::default()
    });
    let (events_tx, mut events) = mpsc::channel(64);
    let provider = std::sync::Arc::new(
        orchestrator
            .spawn(spec(&fake, "discord"), events_tx)
            .unwrap(),
    );
    login_required(&mut events).await;
    let functions = provider.functions(FunctionContext::Provider).await.unwrap();
    let login = start_login(&provider, &functions);
    let link = browser_link(&mut events).await;
    let viewer = reqwest::get(&link).await.unwrap();
    assert_eq!(viewer.status(), 200);
    assert!(viewer.text().await.unwrap().contains("<html"));
    // The host API needs the key the orchestrator made up.
    let base = &link[..link.find("/view/").unwrap()];
    let sessions = reqwest::Client::new()
        .post(format!("{base}/sessions"))
        .body(r#"{"provider":"discord"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(sessions.status(), 401);

    login.abort();
    login_required(&mut events).await;
    timeout(Duration::from_secs(10), async {
        while reqwest::get(&link).await.unwrap().status() != 404 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the login was closed");

    drop(provider);
    drop(orchestrator);
    timeout(Duration::from_secs(10), async {
        while reqwest::get(&link).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the service stopped");
}

// A completed REST mutation must already have queued its event when its
// answer arrives, even when the gateway confirms it much later.
fn queued_message(events: &mut mpsc::Receiver<ProviderEvent>) -> MessageEvent {
    loop {
        match events.try_recv().expect("event must precede the answer") {
            ProviderEvent::Message { event, .. } => return event,
            ProviderEvent::Directory { .. } => {}
            event => panic!("unexpected event: {event:?}"),
        }
    }
}

async fn no_duplicate(events: &mut mpsc::Receiver<ProviderEvent>) {
    assert!(
        timeout(Duration::from_millis(700), message_event(events))
            .await
            .is_err(),
        "late gateway confirmation must not emit a duplicate"
    );
}

async fn wait_stat(fake: &Fake, name: &str, count: u64) {
    timeout(Duration::from_secs(10), async {
        while fake.stat(name).await < count {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn rest_events_precede_answers_and_gateway_duplicates_are_ignored() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;
    fake.control("/event-delay?ms=400").await;
    let id = provider
        .send_message(
            general(),
            None,
            text("confirmed"),
            Some("confirmed-nonce".into()),
        )
        .await
        .unwrap();
    assert!(matches!(queued_message(&mut events), MessageEvent::Received(m) if m.id == id));
    no_duplicate(&mut events).await;
    let context = FunctionContext::Message {
        channel: general(),
        id: id.clone(),
    };
    let functions = provider.functions(context.clone()).await.unwrap();
    for (name, arguments) in [
        (
            "edit",
            args([(inputs::CONTENT, Value::Content(text("changed")))]),
        ),
        (
            "react",
            args([(inputs::REACTION, Value::Text("👍".into()))]),
        ),
        (
            "unreact",
            args([(inputs::REACTION, Value::Text("👍".into()))]),
        ),
        // Opposite changes must not make a second add look like a duplicate.
        (
            "react",
            args([(inputs::REACTION, Value::Text("👍".into()))]),
        ),
        ("mark_read", Arguments::new()),
        ("delete", Arguments::new()),
    ] {
        provider
            .call_function(find(&functions, name), context.clone(), arguments)
            .await
            .unwrap();
        let event = queued_message(&mut events);
        assert!(
            match (name, event) {
                ("edit", MessageEvent::Edited { content, .. }) => content == text("changed"),
                ("react", MessageEvent::Reacted { added, .. }) => added,
                ("unreact", MessageEvent::Reacted { added, .. }) => !added,
                ("mark_read", MessageEvent::Read { up_to, .. }) => up_to.as_deref() == Some(&id),
                ("delete", MessageEvent::Deleted { id: deleted, .. }) => deleted == id,
                _ => false,
            },
            "wrong event for {name}"
        );
        no_duplicate(&mut events).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn accepted_send_cancellation_stays_unknown_and_discovery_cancels() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;
    fake.control("/hold-response?on=1").await;
    let p = provider.clone();
    let send = tokio::spawn(async move {
        p.send_message(
            general(),
            None,
            text("accepted"),
            Some("accepted-nonce".into()),
        )
        .await
    });
    wait_stat(&fake, "accepted", 1).await;
    send.abort();
    assert!(matches!(message_event(&mut events).await,
        MessageEvent::Received(m) if m.nonce.as_deref() == Some("accepted-nonce")));
    no_duplicate(&mut events).await; // In particular, no false NotSent.
    fake.control("/hold-response?on=0").await;
    // A preflight rejection is known not to have reached Discord.
    assert!(
        provider
            .send_message(general(), None, vec![], Some("preflight".into()))
            .await
            .is_err()
    );
    assert!(
        matches!(queued_message(&mut events), MessageEvent::NotSent { nonce, .. } if nonce == "preflight")
    );
    assert_eq!(fake.stat("accepted").await, 1);

    fake.control("/block-discovery?on=1").await;
    let p = provider.clone();
    let discovery = tokio::spawn(async move {
        p.functions(FunctionContext::Message {
            channel: general(),
            id: "999999999".into(),
        })
        .await
    });
    wait_stat(&fake, "discovery_blocked", 1).await;
    discovery.abort();
    wait_stat(&fake, "discovery_cancelled", 1).await;
    provider.shutdown().unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            if let ProviderEvent::Exited { result, .. } = next(&mut events).await {
                assert!(result.is_ok(), "{result:?}");
                break;
            }
        }
    })
    .await
    .expect("cancelled discovery must not block shutdown");
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_send_cancellation_reports_not_sent_without_submission() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;
    fake.control("/hold-response?on=1").await;
    let mut running = Vec::new();
    for n in 0..16 {
        let p = provider.clone();
        running.push(tokio::spawn(async move {
            p.send_message(general(), None, text("busy"), Some(format!("busy-{n}")))
                .await
        }));
    }
    wait_stat(&fake, "accepted", 1).await;
    // Discord serializes a channel's REST bucket; the remaining workers
    // wait in that bucket behind its held request.
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The reader takes one more request while waiting for a worker slot.
    // The following request stays queued in the SDK and can be skipped.
    let p = provider.clone();
    running.push(tokio::spawn(async move {
        p.send_message(general(), None, text("waiting"), None).await
    }));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let p = provider.clone();
    let queued = tokio::spawn(async move {
        p.send_message(general(), None, text("queued"), Some("queued-nonce".into()))
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    queued.abort();
    timeout(Duration::from_secs(10), async {
        loop {
            match message_event(&mut events).await {
                MessageEvent::NotSent { nonce, error, .. } => {
                    assert_eq!(nonce, "queued-nonce");
                    assert_eq!(error, CommandError::Cancelled);
                    break;
                }
                MessageEvent::Received(_) => {}
                event => panic!("unexpected event: {event:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(fake.stat("accepted").await, 1);
    for task in running {
        task.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn picked_user_mentions_reach_the_platform() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;
    provider
        .send_message(
            general(),
            None,
            text("Hello <@1002>"),
            Some("mention-test".into()),
        )
        .await
        .unwrap();
    assert_eq!(
        fake.stat("user_mentions").await,
        1,
        "the REST body must allow the picked user mention"
    );
    let MessageEvent::Received(message) = message_event(&mut events).await else {
        panic!("expected the sent mention");
    };
    assert_eq!(message.content, text("Hello <@1002>"));
}

#[tokio::test(flavor = "multi_thread")]
async fn history_reports_authors_missing_from_the_initial_member_list() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;
    fake.control("/history-author").await;
    let page = provider.history(general(), None, 3).await.unwrap();
    assert_eq!(page.messages[0].author, "1009");
    let event = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("history never reported its author name")
        .unwrap();
    match event {
        ProviderEvent::Directory {
            update: DirectoryUpdate::UserUpserted { organization, user },
            ..
        } => {
            assert_eq!(organization, general().organization);
            assert_eq!(user.id, "1009");
            assert_eq!(user.name, "History-user");
        }
        event => panic!("expected author profile, got {event:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_message_body_is_definitely_not_sent() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;
    let nonce = "12345678-1234-1234-1234-123456789012";
    let error = provider
        .send_message(general(), None, text("rejected"), Some(nonce.into()))
        .await
        .unwrap_err();
    let RequestError::Failed(CommandError::Provider {
        details: Some(details),
        ..
    }) = error
    else {
        panic!("expected a body rejection: {error:?}");
    };
    let Value::Record(details) = *details else {
        panic!("expected rejection details");
    };
    assert_eq!(
        details.get("delivery_unknown"),
        Some(&Value::Boolean(false))
    );
    match message_event(&mut events).await {
        MessageEvent::NotSent {
            nonce: rejected, ..
        } => assert_eq!(rejected, nonce),
        event => panic!("expected NotSent, got {event:?}"),
    }
    assert_eq!(fake.stat("accepted").await, 0);
    provider
        .send_message(
            general(),
            None,
            text("accepted"),
            Some("short-nonce".into()),
        )
        .await
        .unwrap();
    assert_eq!(fake.stat("accepted").await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_attachment_from_host_blob_and_keeps_display_metadata() {
    let fake = Fake::start();
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(dir.path()).await;
    let (provider, mut events) = signed_in(&fake, &orchestrator).await;
    let bytes = vec![0x61; 700_000]; // More than two SDK read chunks.
    let blob = provider.add_blob(bytes.clone()).unwrap();
    let mut media = blob.media(Some("sample.txt".into()), Some("text/plain".into()));
    media.metadata.spoiler = true;
    let id = provider
        .send_message(
            general(),
            None,
            vec![Content::Text("caption".into()), Content::File(media)],
            Some("media-test".into()),
        )
        .await
        .unwrap();
    let MessageEvent::Received(message) = message_event(&mut events).await else {
        panic!("missing media event")
    };
    assert_eq!(message.id, id);
    let Content::File(media) = &message.content[1] else {
        panic!("missing attachment")
    };
    assert_eq!(media.name.as_deref(), Some("SPOILER_sample.txt"));
    assert_eq!(media.size, Some(bytes.len() as u64));
    assert_eq!(media.metadata.width, Some(640));
    assert!(media.metadata.spoiler);
    assert!(media.metadata.expires_at_ms.is_some());
    assert_eq!(fake.stat("uploaded_bytes").await, bytes.len() as u64);
    let fetched = provider.get_message(general(), id).await.unwrap();
    assert_eq!(fetched.content, message.content);
    assert!(
        provider
            .send_message(
                general(),
                None,
                vec![Content::File(qmsg_types::Media {
                    name: None,
                    mime: None,
                    size: None,
                    source: qmsg_types::MediaSource::Url("http://127.0.0.1/private".into()),
                    metadata: Default::default()
                })],
                None
            )
            .await
            .is_err()
    );
    provider.shutdown().unwrap();
}
