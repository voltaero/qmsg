//! Real WASM IMAP/SMTP provider against temporary TLS mail servers.
use qmsg_orchestrator::{Encryption, Orchestrator, ProviderEvent, ProviderSpec};
use qmsg_types::{Arguments, ChannelRef, Content, FunctionContext, ProviderStatus, Value};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
mod support;
#[derive(serde::Deserialize)]
struct Addresses {
    imap: String,
    smtp: String,
    ca: String,
    oauth: String,
}
struct MailServer {
    process: Child,
    addresses: Addresses,
}
impl MailServer {
    fn start() -> Self {
        static BIN: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        let bin = BIN.get_or_init(|| {
            let bin = support::root().join("target/go/fakemail");
            assert!(
                Command::new("go")
                    .args(["build", "-o"])
                    .arg(&bin)
                    .arg("./cmd/fakemail")
                    .current_dir(support::root().join("providers/email"))
                    .status()
                    .unwrap()
                    .success()
            );
            bin
        });
        let mut process = Command::new(bin)
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
            addresses: serde_json::from_str(&line).unwrap(),
        }
    }
}
impl Drop for MailServer {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
async fn status(events: &mut mpsc::Receiver<ProviderEvent>) -> ProviderStatus {
    loop {
        match timeout(Duration::from_secs(300), events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            ProviderEvent::Status { status, .. } => return status,
            ProviderEvent::Exited { result, .. } => panic!("provider exited: {result:?}"),
            _ => {}
        }
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn email_component_logs_in_reads_sends_replies_and_sets_flags() {
    let fake = MailServer::start();
    let dir = tempfile::tempdir().unwrap();
    let host = Orchestrator::new(dir.path(), Encryption::Plaintext)
        .await
        .unwrap();
    let (tx, mut events) = mpsc::channel(64);
    let p = host
        .spawn(
            ProviderSpec {
                name: "email".into(),
                wasm: support::go_component("providers/email"),
                settings: [
                    ("address".into(), "alice@example.test".into()),
                    ("imap_addr".into(), "imap.example.test:993".into()),
                    ("smtp_addr".into(), "smtp.example.test:465".into()),
                    ("sent_mailbox".into(), "Sent".into()),
                    ("ca_pem".into(), fake.addresses.ca.clone()),
                    (
                        "test_dial".into(),
                        format!(
                            "imap.example.test={},smtp.example.test={}",
                            fake.addresses.imap, fake.addresses.smtp
                        ),
                    ),
                ]
                .into(),
            },
            tx,
        )
        .unwrap();
    assert_eq!(status(&mut events).await, ProviderStatus::Connecting);
    assert!(matches!(
        status(&mut events).await,
        ProviderStatus::LoginRequired(_)
    ));
    let fs = p.functions(FunctionContext::Provider).await.unwrap();
    let login = fs.iter().find(|f| f.id == "login").unwrap();
    p.call_function(
        login,
        FunctionContext::Provider,
        Arguments::from([("password".into(), Value::Text("fixture-password".into()))]),
    )
    .await
    .unwrap();
    assert_eq!(status(&mut events).await, ProviderStatus::Ready);
    let inbox = ChannelRef::new(Some("alice@example.test"), "INBOX");
    let page = p.history(inbox.clone(), None, 5).await.unwrap();
    assert_eq!(page.messages.len(), 5);
    assert_eq!(page.messages[0].email.as_ref().unwrap().subject, "Test 6");
    assert!(page.next_cursor.is_some());
    let next = p.history(inbox.clone(), page.next_cursor, 5).await.unwrap();
    assert_eq!(next.messages.len(), 1);
    let fs = p.functions(FunctionContext::Provider).await.unwrap();
    let compose = fs.iter().find(|f| f.id == "compose").unwrap();
    let attachment = vec![0x61; 700_000];
    let blob = p.add_blob(attachment.clone()).unwrap();
    let result = p
        .call_function(
            compose,
            FunctionContext::Provider,
            Arguments::from([
                (
                    "to".into(),
                    Value::List(vec![Value::Text("alice@example.test".into())]),
                ),
                ("subject".into(), Value::Text("WASM mail".into())),
                (
                    "content".into(),
                    Value::Content(vec![
                        Content::Text("Sent through WASM".into()),
                        Content::File(
                            blob.media(Some("sample.txt".into()), Some("text/plain".into())),
                        ),
                    ]),
                ),
            ]),
        )
        .await
        .unwrap();
    assert!(
        matches!(result,Value::Record(ref r) if r.get("accepted")==Some(&Value::Boolean(true)) && r.get("sent_copy_saved")==Some(&Value::Boolean(true)))
    );
    let page = p.history(inbox.clone(), None, 5).await.unwrap();
    let m = p
        .get_message(inbox.clone(), page.messages[0].id.clone())
        .await
        .unwrap();
    assert_eq!(m.email.as_ref().unwrap().subject, "WASM mail");
    let Content::File(media) = &m.content[1] else {
        panic!("missing attachment")
    };
    assert_eq!(media.name.as_deref(), Some("sample.txt"));
    assert!(matches!(&media.source,qmsg_types::MediaSource::Bytes(bytes) if bytes==&attachment));
    let fc = FunctionContext::Message {
        channel: inbox.clone(),
        id: m.id.clone(),
    };
    let fs = p.functions(fc.clone()).await.unwrap();
    let read = fs.iter().find(|f| f.id == "mark_read").unwrap();
    p.call_function(read, fc.clone(), Arguments::new())
        .await
        .unwrap();
    assert!(
        p.get_message(inbox.clone(), m.id.clone())
            .await
            .unwrap()
            .email
            .unwrap()
            .read
    );
    let reply = fs.iter().find(|f| f.id == "reply").unwrap();
    p.call_function(
        reply,
        fc,
        Arguments::from([(
            "content".into(),
            Value::Content(vec![Content::Text("Reply through WASM".into())]),
        )]),
    )
    .await
    .unwrap();
    let replies = p.history(inbox, None, 5).await.unwrap();
    assert_eq!(
        replies.messages[0].email.as_ref().unwrap().subject,
        "Re: WASM mail"
    );
    assert_eq!(
        replies.messages[0].email.as_ref().unwrap().in_reply_to,
        m.email.as_ref().unwrap().message_id
    );
    p.shutdown().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn gmail_component_refreshes_oauth_and_uses_xoauth2() {
    gmail_oauth_flow(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gmail_ios_component_uses_registered_callback_and_xoauth2() {
    gmail_oauth_flow(true).await;
}

async fn gmail_oauth_flow(ios: bool) {
    let fake = MailServer::start();
    let dir = tempfile::tempdir().unwrap();
    let host = Orchestrator::new(dir.path(), Encryption::Plaintext)
        .await
        .unwrap();
    let (tx, mut events) = mpsc::channel(64);
    let p = host
        .spawn(
            ProviderSpec {
                name: "gmail".into(),
                wasm: support::go_component("providers/email"),
                settings: [
                    ("service".into(), "gmail".into()),
                    // The fixture server has no X-GM-RAW search for tabs.
                    ("gmail_tabs".into(), "false".into()),
                    (
                        "oauth_client_id".into(),
                        if ios {
                            "123-qmsg.apps.googleusercontent.com"
                        } else {
                            "fixture-client"
                        }
                        .into(),
                    ),
                    (
                        "oauth_redirect_uri".into(),
                        if ios {
                            "com.googleusercontent.apps.123-qmsg:/oauth2redirect"
                        } else {
                            ""
                        }
                        .into(),
                    ),
                    ("address".into(), "alice@example.test".into()),
                    ("ca_pem".into(), fake.addresses.ca.clone()),
                    (
                        "test_dial".into(),
                        format!(
                            "imap.gmail.com={},smtp.gmail.com={},oauth2.googleapis.com={}",
                            fake.addresses.imap, fake.addresses.smtp, fake.addresses.oauth
                        ),
                    ),
                ]
                .into(),
            },
            tx,
        )
        .unwrap();
    assert_eq!(status(&mut events).await, ProviderStatus::Connecting);
    assert!(matches!(
        status(&mut events).await,
        ProviderStatus::LoginRequired(_)
    ));
    let fs = p.functions(FunctionContext::Provider).await.unwrap();
    let login = fs.iter().find(|f| f.id == "login").unwrap();
    let started = p
        .call_function(
            login,
            FunctionContext::Provider,
            if ios {
                Arguments::new()
            } else {
                Arguments::from([(
                    "redirect_uri".into(),
                    Value::Text("http://127.0.0.1:43210/callback".into()),
                )])
            },
        )
        .await
        .unwrap();
    let Value::Record(started) = started else {
        panic!("OAuth result");
    };
    let state = started["state"].clone();
    assert!(
        matches!(&started["url"], Value::Text(url) if url.starts_with("https://accounts.google.com/"))
    );
    let fs = p.functions(FunctionContext::Provider).await.unwrap();
    let finish = fs.iter().find(|f| f.id == "finish_login").unwrap();
    p.call_function(
        finish,
        FunctionContext::Provider,
        Arguments::from([
            ("code".into(), Value::Text("fixture-code".into())),
            ("state".into(), state),
        ]),
    )
    .await
    .unwrap();
    let inbox = ChannelRef::new(Some("alice@example.test"), "INBOX");
    assert_eq!(
        p.history(inbox.clone(), None, 5)
            .await
            .unwrap()
            .messages
            .len(),
        5
    );
    let fs = p.functions(FunctionContext::Provider).await.unwrap();
    let compose = fs.iter().find(|f| f.id == "compose").unwrap();
    p.call_function(
        compose,
        FunctionContext::Provider,
        Arguments::from([
            (
                "to".into(),
                Value::List(vec![Value::Text("alice@example.test".into())]),
            ),
            ("subject".into(), Value::Text("OAuth WASM".into())),
            (
                "content".into(),
                Value::Content(vec![Content::Text("OAuth".into())]),
            ),
        ]),
    )
    .await
    .unwrap();
    assert_eq!(
        p.history(inbox, None, 5).await.unwrap().messages[0]
            .email
            .as_ref()
            .unwrap()
            .subject,
        "OAuth WASM"
    );
    p.shutdown().unwrap();
}
