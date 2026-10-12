//! Live DM test: discord_dm <known username> [--mutate]. Uses the QR smoke test's saved session.
use anyhow::{Context, bail, ensure};
use qmsg_orchestrator::{Encryption, Orchestrator, ProviderEvent, ProviderHandle};
use qmsg_types::{
    Arguments, CommandError, CompletionRequest, Content, DirectoryUpdate, Function, FunctionCall,
    FunctionContext, ProviderStatus, Value, Verification, VerificationRequest, inputs,
};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

fn function<'a>(functions: &'a [Function], id: &str) -> anyhow::Result<&'a Function> {
    functions
        .iter()
        .find(|f| f.id == id)
        .with_context(|| format!("function {id} unavailable"))
}
fn args(key: &str, value: Value) -> Arguments {
    [(key.to_owned(), value)].into()
}
fn helper(id: &str) -> FunctionCall {
    FunctionCall {
        function: id.into(),
        context: FunctionContext::Provider,
        arguments: Arguments::new(),
    }
}

async fn test_dm(
    p: Arc<ProviderHandle>,
    target: String,
    mutate: bool,
    me: Option<String>,
) -> anyhow::Result<()> {
    let functions = p.functions(FunctionContext::Provider).await?;
    let matches = p
        .call_function(
            function(&functions, "find_users")?,
            FunctionContext::Provider,
            args("query", Value::Text(target.clone())),
        )
        .await?;
    let Value::List(matches) = matches else {
        bail!("user lookup returned no list")
    };
    ensure!(
        matches.len() == 1,
        "expected one match for {target}, found {}; no message sent",
        matches.len()
    );
    let Value::Record(user) = &matches[0] else {
        bail!("user lookup returned no record")
    };
    let Some(Value::Text(id)) = user.get("id") else {
        bail!("user lookup returned no id")
    };
    println!("Matched {target}: {}", serde_json::to_string(user)?);
    let checked = p
        .verify(
            function(&functions, "verify_user")?,
            VerificationRequest {
                call: helper("verify_user"),
                input: None,
                value: Value::Text(id.clone()),
            },
        )
        .await?;
    ensure!(
        checked == Verification::Valid,
        "user verification failed: {checked:?}"
    );
    let completed = p
        .complete(
            function(&functions, "complete_user")?,
            CompletionRequest {
                call: helper("complete_user"),
                input: None,
                query: target,
                cursor: None,
                limit: 5,
            },
        )
        .await?;
    ensure!(
        completed
            .items
            .iter()
            .any(|item| item.value == Value::Text(id.clone())),
        "autocomplete didn't include the matched user"
    );
    println!("User verification and autocomplete passed.");
    let channel = p.open_channel(None, vec![id.clone()]).await?;
    let mut history = p.history(channel.clone(), None, 5).await?.messages;
    history.sort_by_key(|m| m.sent_at);
    println!("LAST_FIVE={}", serde_json::to_string(&history)?);
    println!("OWN_USER_ID={}", me.as_deref().unwrap_or("unknown"));
    let command_args: Vec<_> = std::env::args().collect();
    if let Some(index) = command_args.iter().position(|arg| arg == "--check-message") {
        let id = command_args
            .get(index + 1)
            .cloned()
            .context("missing message id")?;
        println!("CHECK_MESSAGE={:?}", p.get_message(channel, id).await);
        return Ok(());
    }
    if !mutate {
        return Ok(());
    }
    let sent_text = "qmsg provider test: sending, editing and deleting this test message.";
    let sent = p
        .send_message(
            channel.clone(),
            None,
            vec![Content::Text(sent_text.into())],
            Some(format!("qmsg-live-{}", std::process::id())),
        )
        .await?;
    println!("SENT={sent}");
    let context = FunctionContext::Message {
        channel: channel.clone(),
        id: sent.clone(),
    };
    let edited_text = "qmsg provider test: edit confirmed. This test message will now be deleted.";
    let result = async {
        let got = p.get_message(channel.clone(), sent.clone()).await?;
        ensure!(
            got.content == vec![Content::Text(sent_text.into())],
            "sent content didn't match"
        );
        let functions = p.functions(context.clone()).await?;
        p.call_function(
            function(&functions, "edit")?,
            context.clone(),
            args(
                inputs::CONTENT,
                Value::Content(vec![Content::Text(edited_text.into())]),
            ),
        )
        .await?;
        let got = p.get_message(channel.clone(), sent.clone()).await?;
        ensure!(
            got.content == vec![Content::Text(edited_text.into())],
            "edited content didn't match"
        );
        println!("EDIT confirmed by get_message.");
        for action in ["react", "unreact"] {
            let functions = p.functions(context.clone()).await?;
            p.call_function(
                function(&functions, action)?,
                context.clone(),
                args(inputs::REACTION, Value::Text("✅".into())),
            )
            .await?;
            println!("{action} passed.");
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    // Delete our known test message even if a later test failed. Never retry a send.
    let functions = p.functions(context.clone()).await?;
    p.call_function(function(&functions, "delete")?, context, Arguments::new())
        .await?;
    let deleted = p.get_message(channel, sent).await;
    ensure!(
        matches!(
            &deleted,
            Err(qmsg_orchestrator::RequestError::Failed(
                CommandError::UnknownMessage(_)
            ))
        ),
        "delete verification returned {deleted:?}"
    );
    println!("DELETE confirmed: get_message returns UnknownMessage.");
    result
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let target = std::env::args()
        .nth(1)
        .context("usage: discord_dm <known username> [--mutate]")?;
    let mutate = std::env::args().any(|arg| arg == "--mutate");
    let host = Orchestrator::new(Path::new("target/live-discord"), Encryption::Keychain).await?;
    let (tx, mut events) = mpsc::channel(256);
    let mut p = host.spawn(
        qmsg_orchestrator::ProviderSpec {
            name: "discord".into(),
            wasm: "target/go/providers-discord.wasm".into(),
            settings: Default::default(),
        },
        tx,
    )?;
    p.set_timeout(Some(Duration::from_secs(120)));
    let p = Arc::new(p);
    let mut task = None;
    let mut me = None;
    let result = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break Ok(()),
            Some(event) = events.recv() => match event {
                ProviderEvent::Directory { update: DirectoryUpdate::Me { organization: None, id }, .. } => me = id,
                ProviderEvent::Status { status: ProviderStatus::Ready, .. } if task.is_none() => {
                    println!("READY; starting DM test.");
                    task = Some(tokio::spawn(test_dm(p.clone(), target.clone(), mutate, me.clone())));
                }
                ProviderEvent::Status { status: ProviderStatus::Failed(reason), .. } => break Err(anyhow::anyhow!(reason)),
                ProviderEvent::Status { status: ProviderStatus::LoginRequired(_), .. } => break Err(anyhow::anyhow!("saved session needs a new login")),
                ProviderEvent::Exited { result, .. } => break Err(anyhow::anyhow!("provider exited: {result:?}")),
                _ => {},
            },
            done = async { task.as_mut().unwrap().await }, if task.is_some() => break done?,
        }
    };
    if let Some(task) = task {
        task.abort();
    }
    p.kill();
    result
}
