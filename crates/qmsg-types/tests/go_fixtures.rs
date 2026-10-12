//! Golden postcard encodings shared with the Go SDK in `sdk/go`, whose tests
//! build the same values and check they encode and decode to these bytes.
//!
//! Run with `QMSG_UPDATE_FIXTURES=1` to rewrite them after changing the
//! types, then update the Go mirror until its tests pass.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use qmsg_types::*;

fn some(s: &str) -> Option<String> {
    Some(s.into())
}

fn guild_channel() -> ChannelRef {
    ChannelRef {
        organization: some("g"),
        channel: "c".into(),
    }
}

fn dm() -> ChannelRef {
    ChannelRef {
        organization: None,
        channel: "dm".into(),
    }
}

fn media(source: MediaSource) -> Media {
    Media {
        name: some("a.png"),
        mime: some("image/png"),
        size: Some(1 << 40),
        source,
        metadata: MediaMetadata {
            width: Some(640),
            height: Some(480),
            duration_ms: Some(1234),
            expires_at_ms: Some(1_700_000_000_000),
            preview_url: some("https://cdn/preview"),
            description: some("an image"),
            spoiler: true,
        },
    }
}

fn all_content() -> Vec<Content> {
    let styles = vec![
        Style::Bold,
        Style::Italic,
        Style::Underline,
        Style::Strikethrough,
        Style::Code,
        Style::CodeBlock {
            language: some("rs"),
        },
        Style::CodeBlock { language: None },
        Style::Quote,
        Style::Spoiler,
        Style::Link("https://x".into()),
        Style::Mention(Mention::User("u".into())),
        Style::Mention(Mention::Channel("c".into())),
        Style::Mention(Mention::Role("r".into())),
        Style::Mention(Mention::Everyone),
    ];
    vec![
        Content::Text("héllo 😀".into()),
        Content::Formatted(FormattedText {
            text: "styled".into(),
            spans: styles
                .into_iter()
                .enumerate()
                .map(|(i, style)| Span {
                    start: i as u32,
                    end: 200 + i as u32,
                    style,
                })
                .collect(),
        }),
        Content::Image(media(MediaSource::Bytes(vec![0, 1, 255]))),
        Content::Video(Media {
            name: None,
            mime: None,
            size: None,
            source: MediaSource::Url("https://cdn/v".into()),
            metadata: Default::default(),
        }),
        Content::Audio(media(MediaSource::Blob("blob-1".into()))),
        Content::File(media(MediaSource::Url("https://cdn/f".into()))),
        Content::Custom {
            kind: "sticker".into(),
            data: vec![9; 130],
        },
    ]
}

fn message() -> Message {
    Message {
        id: "m1".into(),
        channel: guild_channel(),
        author: "u1".into(),
        sent_at: 1_700_000_000_000,
        reply_to: some("m0"),
        content: all_content(),
        edited_at: Some(1_700_000_000_500),
        reactions: vec![
            Reaction {
                key: "👍".into(),
                count: 3,
                me: true,
            },
            Reaction {
                key: "custom:1".into(),
                count: 70000,
                me: false,
            },
        ],
        nonce: some("n1"),
        email: Some(Box::new(Email {
            subject: "Mail 😀".into(),
            from: vec![EmailAddress {
                address: "a@example.test".into(),
                name: some("Alice"),
            }],
            to: vec![EmailAddress {
                address: "b@example.test".into(),
                name: None,
            }],
            cc: vec![],
            bcc: vec![],
            reply_to: vec![],
            message_id: some("m@example.test"),
            in_reply_to: some("old@example.test"),
            references: vec!["old@example.test".into()],
            html: some("<p>Hello</p>"),
            content_ids: BTreeMap::from([("2".into(), "image@example.test".into())]),
            read: true,
            starred: false,
            draft: true,
        })),
    }
}

fn plain_message() -> Message {
    Message {
        id: "m2".into(),
        channel: dm(),
        author: "u2".into(),
        sent_at: 0,
        reply_to: None,
        content: vec![],
        edited_at: None,
        reactions: vec![],
        nonce: None,
        email: None,
    }
}

fn all_values() -> Arguments {
    let mut record = BTreeMap::new();
    record.insert("x".into(), Value::Null);
    [
        ("a", Value::Null),
        ("b", Value::Boolean(true)),
        ("c", Value::Integer(-5)),
        ("d", Value::Integer(i64::MIN)),
        ("d2", Value::Integer(i64::MAX)),
        ("e", Value::Text("é".into())),
        ("f", Value::Bytes(vec![0, 255])),
        (
            "g",
            Value::List(vec![Value::Integer(300), Value::Boolean(false)]),
        ),
        ("h", Value::Record(record)),
        ("i", Value::Content(all_content())),
        ("j", Value::Channel(dm())),
        ("k", Value::Message(message())),
        ("z", Value::Record(BTreeMap::new())),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

fn all_value_types() -> Vec<ValueType> {
    vec![
        ValueType::Any,
        ValueType::Null,
        ValueType::Boolean,
        ValueType::Integer,
        ValueType::Text,
        ValueType::Bytes,
        ValueType::List(Box::new(ValueType::List(Box::new(ValueType::Text)))),
        ValueType::Record(vec![
            Field {
                id: "id".into(),
                label: "Id".into(),
                value_type: ValueType::Text,
                required: true,
            },
            Field {
                id: "n".into(),
                label: "N".into(),
                value_type: ValueType::Record(vec![]),
                required: false,
            },
        ]),
        ValueType::Content,
        ValueType::Channel,
        ValueType::Message,
    ]
}

fn functions() -> Vec<Function> {
    let actions = [
        ActionKind::SendMessage,
        ActionKind::EditMessage,
        ActionKind::DeleteMessage,
        ActionKind::OpenChannel,
        ActionKind::AddReaction,
        ActionKind::RemoveReaction,
        ActionKind::MarkRead,
        ActionKind::MarkUnread,
        ActionKind::Typing,
        ActionKind::Login,
        ActionKind::Logout,
        ActionKind::Custom("pin".into()),
    ];
    let mut functions: Vec<Function> = actions
        .into_iter()
        .enumerate()
        .map(|(i, action)| Function {
            id: format!("a{i}"),
            label: "Action".into(),
            description: String::new(),
            kind: FunctionKind::Action {
                action,
                result: ValueType::Null,
            },
            inputs: vec![],
        })
        .collect();
    let hints = vec![
        InputHint::Plain,
        InputHint::Secret,
        InputHint::Multiline,
        InputHint::Choices(vec![
            Choice {
                label: "One".into(),
                value: Value::Integer(1),
            },
            Choice {
                label: "Two".into(),
                value: Value::Text("2".into()),
            },
        ]),
    ];
    functions.push(Function {
        id: "inputs".into(),
        label: "Inputs".into(),
        description: "every hint and type".into(),
        kind: FunctionKind::Lookup(ValueType::List(Box::new(ValueType::Text))),
        inputs: all_value_types()
            .into_iter()
            .zip(hints.into_iter().cycle())
            .enumerate()
            .map(|(i, (value_type, hint))| FunctionInput {
                id: format!("i{i}"),
                label: "Input".into(),
                description: "d".into(),
                value_type,
                required: i % 2 == 0,
                hint,
                verification: (i % 3 == 0).then(|| "verify".into()),
                completion: (i % 3 == 1).then(|| "complete".into()),
            })
            .collect(),
    });
    functions.push(Function {
        id: "verify".into(),
        label: "Verify".into(),
        description: String::new(),
        kind: FunctionKind::Verification(ValueType::Text),
        inputs: vec![],
    });
    functions.push(Function {
        id: "complete".into(),
        label: "Complete".into(),
        description: String::new(),
        kind: FunctionKind::Completion(ValueType::Channel),
        inputs: vec![],
    });
    functions
}

fn channel() -> Channel {
    Channel {
        topic: None,
        icon: None,
        mailbox: Some(Box::new(Mailbox {
            delimiter: some("/"),
            attributes: vec!["\\Inbox".into()],
        })),
        id: "c".into(),
        name: "general".into(),
        kind: ChannelKind::Text,
        parent: some("cat"),
        position: Some(3),
        members: Some(vec!["u1".into(), "u2".into()]),
        accepted_content: vec![
            ContentRule {
                kind: ContentKind::Text,
                max_size: Some(2000),
                text_unit: TextUnit::Chars,
                mime_types: vec![],
            },
            ContentRule {
                kind: ContentKind::Image,
                max_size: None,
                text_unit: TextUnit::Utf16,
                mime_types: vec!["image/png".into(), "image/*".into()],
            },
            ContentRule {
                kind: ContentKind::Custom("sticker".into()),
                max_size: Some(1),
                text_unit: TextUnit::Bytes,
                mime_types: vec![],
            },
        ],
        limits: MessageLimits {
            max_attachments: Some(10),
            max_total_size: Some(25 << 20),
        },
    }
}

fn channel_kinds() -> Vec<Channel> {
    [
        ChannelKind::Voice,
        ChannelKind::Video,
        ChannelKind::Announcement,
        ChannelKind::Forum,
        ChannelKind::Thread,
        ChannelKind::Category,
        ChannelKind::Direct,
        ChannelKind::Group,
        ChannelKind::Custom("stage".into()),
    ]
    .into_iter()
    .map(|kind| Channel {
        mailbox: None,
        topic: None,
        icon: None,
        id: "k".into(),
        name: "k".into(),
        kind,
        parent: None,
        position: None,
        members: None,
        accepted_content: vec![],
        limits: MessageLimits {
            max_attachments: None,
            max_total_size: None,
        },
    })
    .collect()
}

fn command(c: Command) -> Vec<u8> {
    encode(&HostMessage::Command(c)).unwrap()
}

fn provider(m: ProviderMessage) -> Vec<u8> {
    encode(&m).unwrap()
}

fn reply(request: u64, result: Result<Reply, CommandError>) -> Vec<u8> {
    provider(ProviderMessage::Reply { request, result })
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let mut f: Vec<(String, Vec<u8>)> = vec![];
    let mut add = |name: &str, bytes: Vec<u8>| f.push((name.to_owned(), bytes));

    add(
        "config",
        encode(&ProviderConfig {
            name: "discord".into(),
            host_url: "ws://127.0.0.1:1/t".into(),
            settings: [("b", "2"), ("a", "1")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        })
        .unwrap(),
    );

    // Host messages.
    let contexts = [
        FunctionContext::Provider,
        FunctionContext::Organization("g".into()),
        FunctionContext::Channel(guild_channel()),
        FunctionContext::Message {
            channel: guild_channel(),
            id: "m".into(),
        },
    ];
    for (i, context) in contexts.into_iter().enumerate() {
        add(
            &format!("host_functions_{i}"),
            command(Command::Functions {
                request: i as u64,
                context,
            }),
        );
    }
    add(
        "host_call",
        command(Command::Call {
            request: u64::MAX,
            call: FunctionCall {
                function: "f".into(),
                context: FunctionContext::Organization("g".into()),
                arguments: all_values(),
            },
        }),
    );
    add(
        "host_verify",
        command(Command::Verify {
            request: 3,
            verification: Box::new(VerificationRequest {
                call: FunctionCall {
                    function: "v".into(),
                    context: FunctionContext::Provider,
                    arguments: BTreeMap::new(),
                },
                input: Some(FunctionInputRef {
                    function: "open".into(),
                    input: "members".into(),
                }),
                value: Value::List(vec![Value::Text("1".into())]),
            }),
        }),
    );
    add(
        "host_complete",
        command(Command::Complete {
            request: 4,
            completion: Box::new(CompletionRequest {
                call: FunctionCall {
                    function: "c".into(),
                    context: FunctionContext::Channel(dm()),
                    arguments: BTreeMap::new(),
                },
                input: None,
                query: "ab".into(),
                cursor: some("next"),
                limit: 25,
            }),
        }),
    );
    add(
        "host_send",
        command(Command::Send {
            request: 5,
            channel: dm(),
            reply_to: some("r"),
            content: all_content(),
            nonce: some("n"),
        }),
    );
    add(
        "host_history",
        command(Command::History {
            request: 6,
            channel: guild_channel(),
            cursor: None,
            limit: 50,
        }),
    );
    add(
        "host_get_message",
        command(Command::GetMessage {
            request: 7,
            channel: guild_channel(),
            id: "m".into(),
        }),
    );
    add(
        "host_open_channel",
        command(Command::OpenChannel {
            request: 8,
            organization: None,
            members: vec!["u1".into(), "u2".into()],
        }),
    );
    add(
        "host_read_blob",
        command(Command::ReadBlob {
            request: 9,
            blob: "b".into(),
            offset: 1 << 33,
            len: 4096,
        }),
    );
    add(
        "host_release_blob",
        command(Command::ReleaseBlob { blob: "b".into() }),
    );
    add("host_cancel", command(Command::Cancel { request: 300 }));
    add("host_shutdown", command(Command::Shutdown));
    let secrets = [
        Ok(Some(b"token".to_vec())),
        Ok(None),
        Err("locked".to_owned()),
    ];
    for (i, result) in secrets.into_iter().enumerate() {
        add(
            &format!("host_secret_{i}"),
            encode(&HostMessage::Secret {
                key: "token".into(),
                result,
            })
            .unwrap(),
        );
    }
    for (i, result) in [Ok(()), Err("full".to_owned())].into_iter().enumerate() {
        add(
            &format!("host_secret_stored_{i}"),
            encode(&HostMessage::SecretStored {
                key: "token".into(),
                result,
            })
            .unwrap(),
        );
    }
    for (i, result) in [Ok(vec![1, 2, 3]), Err("gone".to_owned())]
        .into_iter()
        .enumerate()
    {
        add(
            &format!("host_blob_{i}"),
            encode(&HostMessage::Blob {
                blob: "b".into(),
                offset: 7,
                result,
            })
            .unwrap(),
        );
    }
    let addrs: Vec<SocketAddr> = vec![
        "127.0.0.1:443".parse().unwrap(),
        "[2001:db8::1]:8080".parse().unwrap(),
    ];
    for (i, result) in [Ok(addrs), Err("nxdomain".to_owned())]
        .into_iter()
        .enumerate()
    {
        add(
            &format!("host_addresses_{i}"),
            encode(&HostMessage::Addresses {
                name: "x:1".into(),
                result,
            })
            .unwrap(),
        );
    }
    for (i, (request, result)) in [
        (1 << 40, Ok("tok".to_owned())),
        (2, Err("cancelled".into())),
    ]
    .into_iter()
    .enumerate()
    {
        add(
            &format!("host_browser_login_{i}"),
            encode(&HostMessage::BrowserLogin { request, result }).unwrap(),
        );
    }

    // Provider messages.
    for (i, level) in [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
        LogLevel::Trace,
    ]
    .into_iter()
    .enumerate()
    {
        add(
            &format!("log_{i}"),
            provider(ProviderMessage::Log {
                level,
                message: "hi".into(),
            }),
        );
    }
    let events = vec![
        MessageEvent::Received(message()),
        MessageEvent::Received(plain_message()),
        MessageEvent::Edited {
            channel: guild_channel(),
            id: "m".into(),
            content: vec![Content::Text("new".into())],
            edited_at: 5,
        },
        MessageEvent::Deleted {
            channel: dm(),
            id: "m".into(),
        },
        MessageEvent::Reacted {
            channel: dm(),
            id: "m".into(),
            user: "u".into(),
            key: "👍".into(),
            added: true,
        },
        MessageEvent::Read {
            channel: dm(),
            user: "u".into(),
            up_to: some("m"),
        },
        MessageEvent::Read {
            channel: dm(),
            user: "u".into(),
            up_to: None,
        },
        MessageEvent::Typing {
            channel: guild_channel(),
            user: "u".into(),
        },
        MessageEvent::Gap {
            channel: guild_channel(),
        },
        MessageEvent::NotSent {
            channel: dm(),
            nonce: "n".into(),
            error: CommandError::Cancelled,
        },
    ];
    for (i, event) in events.into_iter().enumerate() {
        add(
            &format!("event_{i}"),
            provider(ProviderMessage::Message(event)),
        );
    }
    let statuses = vec![
        ProviderStatus::Connecting,
        ProviderStatus::LoginRequired(LoginStep {
            message: "log in".into(),
            function: some("login"),
            link: some("https://l"),
            qr_code: some("qr"),
        }),
        ProviderStatus::LoginRequired(LoginStep {
            message: "log in".into(),
            function: None,
            link: None,
            qr_code: None,
        }),
        ProviderStatus::Syncing,
        ProviderStatus::Ready,
        ProviderStatus::Disconnected {
            reason: "lost".into(),
        },
        ProviderStatus::Failed("bad".into()),
    ];
    for (i, status) in statuses.into_iter().enumerate() {
        add(
            &format!("status_{i}"),
            provider(ProviderMessage::Status(status)),
        );
    }
    add(
        "secret_get",
        provider(ProviderMessage::SecretGet { key: "k".into() }),
    );
    add(
        "secret_set",
        provider(ProviderMessage::SecretSet {
            key: "k".into(),
            value: vec![1; 200],
        }),
    );
    add(
        "secret_delete",
        provider(ProviderMessage::SecretDelete { key: "k".into() }),
    );
    let mut updates = vec![
        DirectoryUpdate::OrganizationUpserted(Organization {
            id: "g".into(),
            name: "Guild".into(),
            me: some("u1"),
            users: vec![User {
                id: "u1".into(),
                name: "Me".into(),
                username: None,
                avatar: None,
            }],
            channels: vec![channel()],
            icon: None,
        }),
        DirectoryUpdate::OrganizationUpdated {
            id: "g".into(),
            name: "G2".into(),
            icon: None,
        },
        DirectoryUpdate::OrganizationRemoved { id: "g".into() },
        DirectoryUpdate::UserUpserted {
            organization: None,
            user: User {
                id: "u".into(),
                name: "U".into(),
                username: None,
                avatar: None,
            },
        },
        DirectoryUpdate::UserRemoved {
            organization: some("g"),
            id: "u".into(),
        },
        DirectoryUpdate::ChannelRemoved(guild_channel()),
        DirectoryUpdate::Me {
            organization: None,
            id: some("u1"),
        },
        DirectoryUpdate::Me {
            organization: some("g"),
            id: None,
        },
        DirectoryUpdate::Reset,
    ];
    for (i, presence) in [
        Presence::Online,
        Presence::Idle,
        Presence::Busy,
        Presence::Offline,
    ]
    .into_iter()
    .enumerate()
    {
        updates.push(DirectoryUpdate::Presence {
            organization: some("g"),
            user: format!("u{i}"),
            presence,
        });
    }
    for channel in channel_kinds() {
        updates.push(DirectoryUpdate::ChannelUpserted {
            organization: None,
            channel,
        });
    }
    for (i, update) in updates.into_iter().enumerate() {
        add(
            &format!("directory_{i}"),
            provider(ProviderMessage::Directory(update)),
        );
    }
    let replies = vec![
        Reply::Functions(functions()),
        Reply::Value(Value::Record(all_values())),
        Reply::Verified(Verification::Valid),
        Reply::Verified(Verification::Invalid(vec![
            InputIssue {
                input: some("members"),
                code: "unknown_user".into(),
                message: "no".into(),
            },
            InputIssue {
                input: None,
                code: "x".into(),
                message: "y".into(),
            },
        ])),
        Reply::Completed(CompletionPage {
            items: vec![CompletionItem {
                label: "Alice".into(),
                description: some("@alice"),
                value: Value::Text("1".into()),
            }],
            next_cursor: some("2"),
        }),
        Reply::Completed(CompletionPage {
            items: vec![],
            next_cursor: None,
        }),
        Reply::Sent { id: "m9".into() },
        Reply::Opened(dm()),
        Reply::History(HistoryPage {
            messages: vec![message(), plain_message()],
            next_cursor: some("m0"),
        }),
        Reply::Message(message()),
        Reply::Blob(vec![7; 3]),
    ];
    for (i, r) in replies.into_iter().enumerate() {
        add(&format!("reply_ok_{i}"), reply(i as u64 + 100, Ok(r)));
    }
    let violations = vec![
        Violation::Empty,
        Violation::Unsupported {
            part: 1,
            kind: ContentKind::Custom("x".into()),
        },
        Violation::Unsupported {
            part: 2,
            kind: ContentKind::Formatted,
        },
        Violation::UnsupportedMime {
            part: 3,
            mime: some("a/b"),
        },
        Violation::TextTooLong {
            length: 2001,
            max: 2000,
        },
        Violation::InvalidFormatting { part: 4 },
        Violation::TooLarge {
            part: 5,
            size: 9,
            max: 8,
        },
        Violation::TooManyAttachments { count: 11, max: 10 },
        Violation::TotalTooLarge { size: 9, max: 8 },
    ];
    let mut errors = vec![
        CommandError::Unsupported,
        CommandError::UnknownFunction("f".into()),
        CommandError::InvalidInput(vec![InputIssue {
            input: some("i"),
            code: "c".into(),
            message: "m".into(),
        }]),
        CommandError::Provider {
            code: "http_500".into(),
            message: "oops".into(),
            details: Some(Box::new(Value::Integer(500))),
        },
        CommandError::Provider {
            code: "c".into(),
            message: "m".into(),
            details: None,
        },
        CommandError::UnknownOrganization("g".into()),
        CommandError::UnknownChannel(dm()),
        CommandError::UnknownUser("u".into()),
        CommandError::UnknownMessage("m".into()),
        CommandError::UnknownBlob("b".into()),
        CommandError::RateLimited {
            retry_after_ms: Some(1500),
        },
        CommandError::RateLimited {
            retry_after_ms: None,
        },
        CommandError::Forbidden("no".into()),
        CommandError::LoginRequired,
        CommandError::Cancelled,
        CommandError::Failed("f".into()),
    ];
    errors.extend(violations.into_iter().map(CommandError::Rejected));
    for (i, e) in errors.into_iter().enumerate() {
        add(&format!("reply_err_{i}"), reply(i as u64, Err(e)));
    }
    add(
        "read_blob",
        provider(ProviderMessage::ReadBlob {
            blob: "b".into(),
            offset: 1 << 40,
            len: MAX_READ,
        }),
    );
    add(
        "lookup",
        provider(ProviderMessage::Lookup {
            name: "discord.com:443".into(),
        }),
    );
    add(
        "browser_login",
        provider(ProviderMessage::BrowserLogin {
            request: 1 << 40,
            provider: "discord".into(),
        }),
    );
    add(
        "cancel_browser_login",
        provider(ProviderMessage::CancelBrowserLogin { request: 1 << 40 }),
    );
    f
}

#[test]
fn go_fixtures_match() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdk/go/testdata/fixtures");
    let update = std::env::var_os("QMSG_UPDATE_FIXTURES").is_some();
    if update {
        std::fs::create_dir_all(&dir).unwrap();
        for entry in std::fs::read_dir(&dir).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
    }
    let fixtures = fixtures();
    for (name, bytes) in &fixtures {
        let path = dir.join(format!("{name}.bin"));
        if update {
            std::fs::write(&path, bytes).unwrap();
        } else {
            let golden = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("{}: {e}; set QMSG_UPDATE_FIXTURES=1", path.display()));
            assert_eq!(&golden, bytes, "{name} changed; set QMSG_UPDATE_FIXTURES=1");
        }
    }
    if !update {
        let count = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(count, fixtures.len(), "stale fixtures in {}", dir.display());
    }
}
