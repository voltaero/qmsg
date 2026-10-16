// An offline server with made-up accounts, so the app can be tried without
// an orchestrator. Nothing leaves the browser.

import type { ActionOperation, Backend, ExecuteResult, ProviderSettings, WatchEvent } from "./client";
import { AppError } from "./errors";
import type {
  Channel,
  ChannelRef,
  Command,
  Content,
  Credential,
  FunctionContext,
  Media,
  Message,
  PairedDevice,
  ProviderCatalog,
  ProviderEvent,
  ProviderId,
  ProviderStatus,
  ProviderView,
  QFunction,
  Reply,
  Scope,
  Snapshot,
  User,
  Value,
} from "./types";

const text: Channel["accepted_content"] = [
  { kind: "Text", max_size: 2000, text_unit: "Chars", mime_types: [] },
  { kind: "Image", max_size: 10 * 1024 * 1024, text_unit: "Bytes", mime_types: ["image/*"] },
  { kind: "Video", max_size: 10 * 1024 * 1024, text_unit: "Bytes", mime_types: [] },
  { kind: "File", max_size: 10 * 1024 * 1024, text_unit: "Bytes", mime_types: [] },
];
const limits = { max_attachments: 10, max_total_size: 25 * 1024 * 1024 };

function channel(id: string, name: string, kind: Channel["kind"], extra: Partial<Channel> = {}): Channel {
  return { id, name, kind, accepted_content: kind === "Category" ? [] : text, limits, ...extra };
}

const users: User[] = [
  { id: "me", name: "You", username: "you" },
  { id: "ada", name: "Ada Lovelace", username: "ada" },
  { id: "linus", name: "Linus", username: "linus" },
  { id: "grace", name: "Grace Hopper", username: "grace" },
  { id: "ken", name: "Ken", username: "ken" },
];
const byId = Object.fromEntries(users.map((u) => [u.id, u]));

function svg(label: string, from: string, to: string): Media {
  const body = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="400" viewBox="0 0 640 400"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${from}"/><stop offset="1" stop-color="${to}"/></linearGradient></defs><rect width="640" height="400" fill="url(#g)"/><circle cx="500" cy="110" r="56" fill="#fff" fill-opacity=".85"/><path d="M0 330 L170 190 L300 300 L420 220 L640 360 L640 400 L0 400Z" fill="#000" fill-opacity=".25"/><text x="32" y="372" font-family="system-ui,sans-serif" font-size="28" fill="#fff" fill-opacity=".9">${label}</text></svg>`;
  return {
    name: `${label.toLowerCase().replace(/\s+/g, "-")}.svg`,
    mime: "image/svg+xml",
    size: body.length,
    source: { Bytes: Array.from(new TextEncoder().encode(body)) },
    metadata: { width: 640, height: 400 },
  };
}

const minute = 60_000;
// ASCII, so string indices are UTF-8 byte offsets.
const grpcText = "The web client speaks gRPC-Web now, so a browser can talk to the orchestrator directly. Try cargo run -p qmsg-orchestrator.";

interface Room {
  provider: string;
  ref: ChannelRef;
  messages: Message[];
}

export class DemoBackend implements Backend {
  readonly demo = true;
  private rooms = new Map<string, Room>();
  private listeners = new Set<(e: ProviderEvent) => void>();
  private sequence = 0;
  private slackStatus: ProviderStatus = {
    LoginRequired: { message: "Sign in to Slack with a token. Any text works in the demo.", function: "login" },
  };
  private nextId = 1000;

  constructor() {
    const now = Date.now();
    const at = (minutesAgo: number) => now - minutesAgo * minute;
    const m = (provider: string, ref: ChannelRef, id: string, author: string, ago: number, content: Content[], extra: Partial<Message> = {}): Message => {
      const message: Message = { id, channel: ref, author, sent_at: at(ago), content, reactions: [], ...extra };
      this.room(provider, ref).messages.push(message);
      return message;
    };
    const general = { organization: "qmsg", channel: "general" };
    m("discord", general, "1", "ada", 60 * 26, [{ Text: "Morning! Pushed the new provider ABI notes." }]);
    m("discord", general, "2", "linus", 60 * 25, [{ Text: "Reading them now." }]);
    m("discord", general, "3", "grace", 95, [
      {
        Formatted: {
          text: grpcText,
          spans: [
            { start: 4, end: 14, style: "Bold" },
            { start: grpcText.indexOf("cargo"), end: grpcText.length - 1, style: "Code" },
          ],
        },
      },
    ], { reactions: [{ key: "🎉", count: 3, me: true }, { key: "🚀", count: 1, me: false }] });
    m("discord", general, "4", "ken", 80, [{ Text: "Screenshot of the sidebar:" }, { Image: svg("Sidebar", "#4c6ef5", "#15aabf") }]);
    m("discord", general, "5", "ada", 42, [{ Text: "Looks great. The reply chain renders too." }], { reply_to: "4" });
    m("discord", general, "6", "me", 30, [{ Text: "Thanks! Working on mail next." }]);
    m("discord", general, "7", "linus", 4, [{ Text: "Ship it 🚢" }]);
    const design = { organization: "qmsg", channel: "design" };
    m("discord", design, "20", "grace", 300, [{ Text: "Palette proposal attached." }, { Image: svg("Palette", "#f59f00", "#e64980") }]);
    m("discord", design, "21", "ada", 200, [{ Text: "I like the warmer one." }]);
    const releases = { organization: "qmsg", channel: "releases" };
    m("discord", releases, "30", "ken", 60 * 50, [{ Text: "v0.1.0 is out: terminal app, iOS app and orchestrator." }]);
    const dmAda = { organization: null, channel: "dm-ada" };
    m("discord", dmAda, "40", "ada", 12, [{ Text: "Do you have a minute to look at the pairing flow?" }]);
    m("discord", dmAda, "41", "me", 10, [{ Text: "Sure, send it over." }]);
    m("discord", dmAda, "42", "ada", 2, [{ Text: "Here's the doc: https://example.com/pairing" }]);
    const dmLinus = { organization: null, channel: "dm-linus" };
    m("discord", dmLinus, "50", "linus", 60 * 30, [{ Text: "Patch looks good." }]);
    const group = { organization: null, channel: "group-1" };
    m("discord", group, "60", "grace", 60 * 5, [{ Text: "Lunch on Friday?" }]);
    m("discord", group, "61", "ken", 60 * 4, [{ Text: "In." }]);

    const inbox = { organization: null, channel: "INBOX" };
    const mail = (id: string, from: [string, string], subject: string, body: string, ago: number, html?: string, read = true) =>
      m("email", inbox, id, from[1], ago, [{ Text: body }], {
        email: {
          subject,
          from: [{ name: from[0], address: from[1] }],
          to: [{ name: "You", address: "you@example.com" }],
          cc: [],
          bcc: [],
          reply_to: [],
          references: [],
          html: html ?? null,
          content_ids: {},
          read,
          starred: false,
          draft: false,
        },
      });
    mail("1:101", ["Grace Hopper", "grace@example.com"], "Design review notes", "Here are the notes from today's review.\n\n1. Sidebar density is right.\n2. Composer should keep drafts per channel.\n\n— Grace", 60 * 3, "<p>Here are the notes from today's review.</p><ol><li>Sidebar density is <b>right</b>.</li><li>Composer should keep drafts per channel.</li></ol><p>— Grace</p>", false);
    mail("1:100", ["GitHub", "noreply@github.com"], "[qmsg] CI passed on main", "All checks have passed.", 60 * 20);
    mail("1:99", ["Ada Lovelace", "ada@example.com"], "Re: Analytical engine", "The notes are attached; let me know what you think.", 60 * 48);
  }

  private room(provider: string, ref: ChannelRef): Room {
    const key = `${provider}/${ref.organization ?? ""}/${ref.channel}`;
    let room = this.rooms.get(key);
    if (!room) {
      room = { provider, ref, messages: [] };
      this.rooms.set(key, room);
    }
    return room;
  }

  private emit(event: ProviderEvent) {
    for (const listener of this.listeners) listener(event);
  }

  private providers(): ProviderView[] {
    const discordScope: Scope = {
      me: "me",
      users: Object.fromEntries(users.map((u) => [u.id, u])),
      channels: Object.fromEntries(
        [
          channel("info", "Information", "Category", { position: 0 }),
          channel("announcements", "announcements", "Announcement", { parent: "info", position: 0, topic: "Release notes and news" }),
          channel("releases", "releases", "Text", { parent: "info", position: 1 }),
          channel("text", "Text channels", "Category", { position: 1 }),
          channel("general", "general", "Text", { parent: "text", position: 0, topic: "Anything about qmsg" }),
          channel("design", "design", "Text", { parent: "text", position: 1 }),
          channel("voice", "Lounge", "Voice", { parent: "text", position: 2, accepted_content: [] }),
        ].map((c) => [c.id, c]),
      ),
      presence: { ada: "Online", linus: "Idle", grace: "Busy", ken: "Offline", me: "Online" },
    };
    const dms: Scope = {
      me: "me",
      users: byId,
      channels: {
        "dm-ada": channel("dm-ada", "", "Direct", { members: ["me", "ada"] }),
        "dm-linus": channel("dm-linus", "", "Direct", { members: ["me", "linus"] }),
        "group-1": channel("group-1", "", "Group", { members: ["me", "grace", "ken"] }),
      },
      presence: { ada: "Online", linus: "Idle" },
    };
    const mailScope: Scope = {
      me: "you@example.com",
      users: {
        "you@example.com": { id: "you@example.com", name: "You", username: "you@example.com" },
        "grace@example.com": { id: "grace@example.com", name: "Grace Hopper" },
        "noreply@github.com": { id: "noreply@github.com", name: "GitHub" },
        "ada@example.com": { id: "ada@example.com", name: "Ada Lovelace" },
      },
      channels: {
        INBOX: channel("INBOX", "INBOX", { Custom: "mailbox" }, { mailbox: { delimiter: "/", attributes: [] } }),
        Sent: channel("Sent", "Sent", { Custom: "mailbox" }, { mailbox: { delimiter: "/", attributes: ["\\Sent"] } }),
        Trash: channel("Trash", "Trash", { Custom: "mailbox" }, { mailbox: { delimiter: "/", attributes: ["\\Trash"] } }),
      },
      presence: {},
    };
    return [
      {
        provider: { name: "discord", instance: 1 },
        status: "Ready",
        organizations: [{ id: "qmsg", name: "qmsg", scope: discordScope }],
        standalone: dms,
      },
      { provider: { name: "email", instance: 2 }, status: "Ready", organizations: [], standalone: mailScope },
      { provider: { name: "slack", instance: 3 }, status: this.slackStatus, organizations: [], standalone: null },
    ];
  }

  setToken() {}

  async pair(): Promise<Credential> {
    return { deviceId: "demo", token: "demo", expiresAtMs: Date.now() + 90 * 86_400_000 };
  }

  rotate = this.pair;

  async devices(): Promise<PairedDevice[]> {
    return [{ id: "demo", name: "This browser (demo)", expiresAtMs: Date.now() + 90 * 86_400_000 }];
  }

  async revoke() {}

  async providerCatalog(): Promise<ProviderCatalog> {
    return {
      types: [
        { id: "discord", label: "Discord", fields: [], defaults: {} },
        {
          id: "email",
          label: "Email",
          fields: [
            { id: "address", label: "Address", required: true, secret: false },
            { id: "imap_addr", label: "IMAP server", required: false, secret: false },
            { id: "smtp_addr", label: "SMTP server", required: false, secret: false },
          ],
          defaults: { service: "imap" },
        },
      ],
      accounts: [
        { name: "discord", label: "Discord", type: "discord", enabled: true, settings: {} },
        { name: "email", label: "Work mail", type: "email", enabled: true, settings: { address: "you@example.com" } },
        { name: "slack", label: "Slack", type: "discord", enabled: true, settings: {} },
      ],
    };
  }

  async configureProvider(_settings: ProviderSettings) {
    throw new Error("Accounts can't be changed in the demo.");
  }

  async removeProvider() {
    throw new Error("Accounts can't be changed in the demo.");
  }

  async snapshot(): Promise<Snapshot> {
    return { providers: this.providers(), epoch: "demo", sequence: this.sequence };
  }

  async *watch(_epoch: string, _after: number, signal: AbortSignal): AsyncGenerator<WatchEvent> {
    const queue: ProviderEvent[] = [];
    let wake: (() => void) | null = null;
    const listener = (e: ProviderEvent) => {
      queue.push(e);
      wake?.();
    };
    this.listeners.add(listener);
    signal.addEventListener("abort", () => wake?.(), { once: true });
    try {
      yield { type: "snapshot", snapshot: await this.snapshot() };
      while (!signal.aborted) {
        const event = queue.shift();
        if (event) {
          yield { type: "update", event, epoch: "demo", sequence: ++this.sequence };
          continue;
        }
        await new Promise<void>((resolve) => (wake = resolve));
        wake = null;
      }
    } finally {
      this.listeners.delete(listener);
    }
  }

  private functionsFor(provider: string, context: FunctionContext): QFunction[] {
    if (provider === "slack" && context === "Provider") {
      return [
        {
          id: "login",
          label: "Sign in",
          description: "Paste a Slack user token.",
          kind: { Action: { action: "Login", result: "Null" } },
          inputs: [
            { id: "workspace", label: "Workspace", description: "", value_type: "Text", required: true, hint: "Plain" },
            { id: "token", label: "Token", description: "Starts with xoxp-", value_type: "Text", required: true, hint: "Secret" },
          ],
        },
      ];
    }
    if (context === "Provider" || (typeof context === "object" && "Organization" in context)) {
      return [
        {
          id: "open",
          label: "New conversation",
          description: "Start a direct or group conversation.",
          kind: { Action: { action: "OpenChannel", result: "Channel" } },
          inputs: [
            {
              id: "members",
              label: "People",
              description: "",
              value_type: { List: "Text" },
              required: true,
              hint: "Plain",
              completion: "complete-user",
            },
          ],
        },
        {
          id: "complete-user",
          label: "Find people",
          description: "",
          kind: { Completion: "Text" },
          inputs: [],
        },
        {
          id: "status",
          label: "Set status",
          description: "Shown to people you talk to.",
          kind: { Action: { action: { Custom: "set-status" }, result: "Null" } },
          inputs: [
            {
              id: "presence",
              label: "Presence",
              description: "",
              value_type: "Text",
              required: true,
              hint: { Choices: ["Online", "Idle", "Busy", "Invisible"].map((v) => ({ label: v, value: { Text: v } })) },
            },
            { id: "text", label: "Custom status", description: "", value_type: "Text", required: false, hint: "Plain" },
          ],
        },
      ];
    }
    if (typeof context === "object" && "Message" in context) {
      const message = this.find(provider, context.Message.channel, context.Message.id);
      const fns: QFunction[] = [
        {
          id: "react",
          label: "Add reaction",
          description: "",
          kind: { Action: { action: "AddReaction", result: "Null" } },
          inputs: [{ id: "reaction", label: "Reaction", description: "", value_type: "Text", required: true, hint: "Plain" }],
        },
        {
          id: "unreact",
          label: "Remove reaction",
          description: "",
          kind: { Action: { action: "RemoveReaction", result: "Null" } },
          inputs: [{ id: "reaction", label: "Reaction", description: "", value_type: "Text", required: true, hint: "Plain" }],
        },
        { id: "read", label: "Mark read", description: "", kind: { Action: { action: "MarkRead", result: "Null" } }, inputs: [] },
        { id: "unread", label: "Mark unread", description: "", kind: { Action: { action: "MarkUnread", result: "Null" } }, inputs: [] },
      ];
      if (message?.author === "me") {
        fns.push(
          {
            id: "edit",
            label: "Edit",
            description: "",
            kind: { Action: { action: "EditMessage", result: "Null" } },
            inputs: [{ id: "content", label: "Message", description: "", value_type: "Content", required: true, hint: "Multiline" }],
          },
          { id: "delete", label: "Delete", description: "", kind: { Action: { action: "DeleteMessage", result: "Null" } }, inputs: [] },
        );
      }
      fns.push({
        id: "pin",
        label: "Pin message",
        description: "Pin it to the channel.",
        kind: { Action: { action: { Custom: "pin" }, result: "Null" } },
        inputs: [],
      });
      return fns;
    }
    return [];
  }

  private find(provider: string, ref: ChannelRef, id: string): Message | undefined {
    return this.room(provider, ref).messages.find((m) => m.id === id);
  }

  private later(ms: number, f: () => void) {
    setTimeout(f, ms);
  }

  async execute(command: Command, provider: ProviderId, _operation?: ActionOperation): Promise<ExecuteResult> {
    await new Promise((r) => setTimeout(r, 120 + Math.random() * 180));
    const ok = (reply: Reply): ExecuteResult => ({ ok: true, reply });
    const fail = (message: string, command?: AppError["command"]): ExecuteResult => ({
      ok: false,
      error: new AppError({ code: "provider", message, command: command ?? { Failed: message }, delivery_unknown: false }),
    });
    const p = provider.name;
    if ("History" in command) {
      const { channel, cursor, limit } = command.History;
      const all = [...this.room(p, channel).messages].sort((a, b) => a.sent_at - b.sent_at);
      const end = cursor ? Number(cursor) : all.length;
      const start = Math.max(0, end - limit);
      return ok({ History: { messages: all.slice(start, end), next_cursor: start > 0 ? String(start) : null } });
    }
    if ("GetMessage" in command) {
      const m = this.find(p, command.GetMessage.channel, command.GetMessage.id);
      return m ? ok({ Message: m }) : fail("Unknown message", { UnknownMessage: command.GetMessage.id });
    }
    if ("Functions" in command) return ok({ Functions: this.functionsFor(p, command.Functions.context) });
    if ("Send" in command) {
      const { channel, content, nonce, reply_to } = command.Send;
      const id = String(this.nextId++);
      const message: Message = { id, channel, author: p === "email" ? "you@example.com" : "me", sent_at: Date.now(), content, reactions: [], nonce, reply_to };
      this.room(p, channel).messages.push(message);
      this.emit({ Message: { provider, event: { Received: message } } });
      if (p === "email") return ok({ Sent: { id } });
      this.later(900, () => this.emit({ Message: { provider, event: { Typing: { channel, user: "ada" } } } }));
      this.later(3200, () => {
        const answer: Message = {
          id: String(this.nextId++),
          channel,
          author: "ada",
          sent_at: Date.now(),
          content: [{ Text: pickReply() }],
          reactions: [],
        };
        this.room(p, channel).messages.push(answer);
        this.emit({ Message: { provider, event: { Received: answer } } });
      });
      return ok({ Sent: { id } });
    }
    if ("OpenChannel" in command) {
      const members = command.OpenChannel.members;
      const existing = members.length === 1 && byId[members[0]!] ? `dm-${members[0]}` : null;
      if (existing === "dm-ada" || existing === "dm-linus") return ok({ Opened: { organization: null, channel: existing } });
      const id = `group-${this.nextId++}`;
      const created = channel(id, "", members.length === 1 ? "Direct" : "Group", { members: ["me", ...members] });
      this.emit({ Directory: { provider, update: { ChannelUpserted: { organization: null, channel: created } } } });
      return ok({ Opened: { organization: null, channel: id } });
    }
    if ("Complete" in command) {
      const q = command.Complete.query.toLowerCase();
      const items = users
        .filter((u) => u.id !== "me" && (u.name.toLowerCase().includes(q) || u.username?.includes(q)))
        .map((u) => ({ label: u.name, description: `@${u.username}`, value: { Text: u.id } as Value }));
      return ok({ Completed: { items, next_cursor: null } });
    }
    if ("Verify" in command) return ok({ Verified: "Valid" });
    if ("Call" in command) {
      const { function: fn, context, arguments: args } = command.Call;
      const text = (v: Value | undefined) => (v && typeof v === "object" && "Text" in v ? v.Text : "");
      if (fn === "login") {
        if (!text(args.token)) return fail("Enter a token.", { InvalidInput: [{ input: "token", code: "required", message: "Enter a token." }] });
        this.slackStatus = "Syncing";
        this.emit({ Status: { provider, status: "Syncing" } });
        this.later(800, () => {
          this.slackStatus = "Ready";
          this.emit({
            Directory: {
              provider,
              update: {
                OrganizationUpserted: {
                  id: "acme",
                  name: text(args.workspace) || "Acme",
                  me: "me",
                  users,
                  channels: [channel("random", "random", "Text"), channel("eng", "engineering", "Text")],
                },
              },
            },
          });
          this.emit({ Status: { provider, status: "Ready" } });
        });
        return ok({ Value: "Null" });
      }
      if (typeof context === "object" && "Message" in context) {
        const { channel: ref, id } = context.Message;
        const message = this.find(p, ref, id);
        if (!message) return fail("Unknown message");
        switch (fn) {
          case "react":
          case "unreact":
            this.emit({ Message: { provider, event: { Reacted: { channel: ref, id, user: "me", key: text(args.reaction), added: fn === "react" } } } });
            break;
          case "edit": {
            const content = args.content && typeof args.content === "object" && "Content" in args.content ? args.content.Content : [];
            message.content = content;
            message.edited_at = Date.now();
            this.emit({ Message: { provider, event: { Edited: { channel: ref, id, content, edited_at: message.edited_at } } } });
            break;
          }
          case "delete": {
            const room = this.room(p, ref);
            room.messages = room.messages.filter((m) => m.id !== id);
            this.emit({ Message: { provider, event: { Deleted: { channel: ref, id } } } });
            break;
          }
        }
      }
      return ok({ Value: "Null" });
    }
    return fail("Not supported in the demo", "Unsupported");
  }

  async upload(_provider: ProviderId, file: File): Promise<Media> {
    const bytes = new Uint8Array(await file.arrayBuffer());
    return { name: file.name, mime: file.type, size: file.size, source: { Bytes: Array.from(bytes) } };
  }

  async readBlob(): Promise<Uint8Array<ArrayBuffer>> {
    throw new Error("No files in the demo.");
  }

  async releaseBlob() {}
}

const replies = [
  "Nice, that works for me.",
  "Agreed 👍",
  "Let me check and get back to you.",
  "Ha, perfect.",
  "Can you send a screenshot?",
];
let replyIndex = 0;
function pickReply() {
  return replies[replyIndex++ % replies.length]!;
}
