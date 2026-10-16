// Everything the app shows, kept current by one `Watch` stream. Mirrors the
// iOS app's QmsgStore.

import { create } from "zustand";
import { accounts, apiBase, parsePairing, proposedToken, randomSecret, type Account } from "~/api/account";
import { newOperation, QmsgClient, type ActionOperation, type Backend, type ExecuteResult, type WatchEvent } from "~/api/client";
import { AppError, describe, isUnauthenticated } from "~/api/errors";
import { Code, RpcError } from "~/api/grpcweb";
import {
  actionOf,
  eventChannel,
  tag,
  type Arguments,
  type Command,
  type CompletionItem,
  type Content,
  type FunctionContext,
  type FunctionInput,
  type Message,
  type ProviderEvent,
  type QFunction,
  type Snapshot,
  type Value,
} from "~/api/types";
import { contentForFile, plainText } from "~/lib/content";
import * as conv from "./conversation";
import {
  applyDirectory,
  channelOf,
  channelTitle,
  keyString,
  newProvider,
  providerState,
  sameKey,
  scopeOf,
  userName,
  type ChannelKey,
  type ProviderState,
} from "./directory";

export type Link =
  | { kind: "idle" }
  | { kind: "connecting" }
  | { kind: "live" }
  | { kind: "retrying"; message: string };

/** What the chat list shows for a channel. */
export interface Activity {
  last: Message | null;
  unread: number;
}

export interface State {
  account: Account | null;
  demo: boolean;
  /** The device token was rejected; pair again. */
  needsPairing: boolean;
  link: Link;
  providers: Record<string, ProviderState>;
  activity: Record<string, Activity>;
  conversations: Record<string, conv.Conversation>;
  /** The channel on screen. Its messages don't count as unread. */
  visible: ChannelKey | null;
  /** Recently opened chats, newest first. */
  recent: ChannelKey[];
  drafts: Record<string, string>;
  replyTo: Record<string, string | null>;
  /** Each account's provider type, such as "discord", from the catalog. */
  types: Record<string, string>;
}

const RECENT_KEY = "qmsg.recent";

function loadRecent(): ChannelKey[] {
  try {
    return JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]") as ChannelKey[];
  } catch {
    return [];
  }
}

export const useStore = create<State>()(() => ({
  account: accounts.load(),
  demo: false,
  needsPairing: false,
  link: { kind: "idle" },
  providers: {},
  activity: {},
  conversations: {},
  visible: null,
  recent: accounts.load() ? loadRecent() : [],
  drafts: {},
  replyTo: {},
  types: {},
}));

const set = useStore.setState;
const get = useStore.getState;

let backend: Backend | null = null;
let watching: AbortController | null = null;
let epoch = "";
let sequence = 0;
/** The newest message marked read in each channel, and marks running. */
const readUpTo = new Map<string, string>();
const marking = new Set<string>();

export function getBackend(): Backend | null {
  return backend;
}

export class StoreError extends Error {
  override name = "StoreError";
}

// MARK: Conversations

export function conversationOf(state: State, key: ChannelKey): conv.Conversation | undefined {
  return state.conversations[keyString(key)];
}

function updateConversation(key: ChannelKey, change: (c: conv.Conversation) => conv.Conversation) {
  const k = keyString(key);
  set((s) => ({ conversations: { ...s.conversations, [k]: change(s.conversations[k] ?? conv.emptyConversation()) } }));
}

function noteLast(key: ChannelKey, message: Message, unread = 0) {
  const k = keyString(key);
  set((s) => {
    const entry = s.activity[k] ?? { last: null, unread: 0 };
    const last = (entry.last?.sent_at ?? 0) <= message.sent_at ? message : entry.last;
    return { activity: { ...s.activity, [k]: { last, unread: entry.unread + unread } } };
  });
}

// MARK: Lifecycle

/** Pairs with the server in a pairing payload, replacing any account. */
export async function pair(text: string, deviceName: string) {
  const payload = parsePairing(text);
  const client = new QmsgClient(apiBase(payload.endpoint));
  const credential = await client.pair(payload.code, deviceName);
  const account: Account = {
    endpoint: payload.endpoint,
    deviceId: credential.deviceId,
    token: credential.token,
    expiresAtMs: credential.expiresAtMs,
  };
  accounts.save(account);
  stop();
  reset();
  client.setToken(account.token);
  backend = client;
  set({ account, needsPairing: false, demo: false });
  startWatching();
}

/** Runs the app offline against made-up accounts. */
export async function startDemo() {
  const { DemoBackend } = await import("~/api/demo");
  stop();
  reset();
  backend = new DemoBackend();
  set({ demo: true, needsPairing: false, account: null });
  startWatching();
}

/** Connects with the saved account, if any. */
export function resume() {
  const { account, needsPairing, demo } = get();
  if (demo || watching || !account || needsPairing) return;
  backend ??= new QmsgClient(apiBase(account.endpoint), account.token);
  void maintainToken();
  startWatching();
}

/** Stops the event stream. The cursor is kept, so `resume` replays what was missed. */
export function suspend() {
  watching?.abort();
  watching = null;
  set({ link: { kind: "idle" } });
}

/** Revokes this device on the server, when reachable, and forgets it. */
export async function unpair() {
  const { account, needsPairing, demo } = get();
  if (account && !needsPairing && !demo) {
    await backend?.revoke(account.deviceId).catch(() => {});
  }
  stop();
  accounts.delete();
  localStorage.removeItem(RECENT_KEY);
  set({ account: null, needsPairing: false, demo: false });
  reset();
}

function stop() {
  suspend();
  backend = null;
}

function reset() {
  for (const c of Object.values(get().conversations)) {
    for (const p of c.pending) for (const url of Object.values(p.previews)) URL.revokeObjectURL(url);
  }
  epoch = "";
  sequence = 0;
  readUpTo.clear();
  marking.clear();
  set({ providers: {}, activity: {}, conversations: {}, visible: null, recent: [], drafts: {}, replyTo: {}, types: {} });
}

// MARK: Event stream

function startWatching() {
  if (watching) return;
  const controller = new AbortController();
  watching = controller;
  void watchLoop(controller.signal);
}

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => (clearTimeout(timer), resolve()), { once: true });
  });

async function watchLoop(signal: AbortSignal) {
  let failures = 0;
  while (!signal.aborted && backend) {
    if (failures === 0) set({ link: { kind: "connecting" } });
    try {
      for await (const event of backend.watch(epoch, sequence, signal)) {
        if (signal.aborted) return;
        failures = 0;
        if (get().link.kind !== "live") set({ link: { kind: "live" } });
        apply(event);
      }
    } catch (error) {
      if (signal.aborted) return;
      if (isUnauthenticated(error) && !(await recoverToken())) {
        set({ needsPairing: true, link: { kind: "idle" } });
        if (watching?.signal === signal) watching = null;
        return;
      }
      set({ link: { kind: "retrying", message: describe(error) } });
    }
    failures += 1;
    // Backoff: 0.5 s doubling to 30 s, with jitter.
    const delay = Math.min(30, 0.5 * 2 ** Math.min(failures, 6)) * (0.8 + Math.random() * 0.4);
    await sleep(delay * 1000, signal);
  }
}

function apply(event: WatchEvent) {
  switch (event.type) {
    case "snapshot":
      applySnapshot(event.snapshot);
      break;
    case "update":
      if (event.epoch !== epoch || event.sequence <= sequence) return;
      sequence = event.sequence;
      if (event.event) applyUpdate(event.event);
      break;
    case "heartbeat":
      break;
  }
}

function applySnapshot(snapshot: Snapshot) {
  if (snapshot.epoch === epoch && snapshot.sequence < sequence) return;
  epoch = snapshot.epoch;
  sequence = snapshot.sequence;
  const providers = Object.fromEntries(snapshot.providers.map((v) => [v.provider.name, providerState(v)]));
  // The snapshot has no messages: reload what is on screen, and the rest when shown.
  const conversations = Object.fromEntries(
    Object.entries(get().conversations).map(([k, c]) => [k, { ...c, stale: true }]),
  );
  set({ providers, conversations });
  const visible = get().visible;
  if (visible) void loadLatest(visible);
  void loadTypes();
}

/** Reads account types, which say how a network writes mentions. Servers without account management have none. */
async function loadTypes() {
  const catalog = await backend?.providerCatalog().catch(() => null);
  if (catalog) set({ types: Object.fromEntries(catalog.accounts.map((a) => [a.name, a.type])) });
}

function applyUpdate(event: ProviderEvent) {
  const [name, body] = tag(event) as [string, { provider: ProviderState["id"] }];
  const id = body.provider;
  let providers = get().providers;
  const current = providers[id.name];
  if (!current || current.id.instance !== id.instance) providers = { ...providers, [id.name]: newProvider(id) };
  switch (name) {
    case "Directory": {
      const { update } = body as Extract<ProviderEvent, { Directory: unknown }>["Directory"];
      set({ providers: { ...providers, [id.name]: applyDirectory(providers[id.name]!, update) } });
      return;
    }
    case "Status": {
      const { status } = body as Extract<ProviderEvent, { Status: unknown }>["Status"];
      set({ providers: { ...providers, [id.name]: { ...providers[id.name]!, status } } });
      return;
    }
    case "Exited": {
      const { result } = body as Extract<ProviderEvent, { Exited: unknown }>["Exited"];
      const error = "Err" in result ? result.Err : null;
      set({ providers: { ...providers, [id.name]: newProvider(id, { Failed: error ?? "The account stopped." }) } });
      return;
    }
    case "Message": {
      if (providers !== get().providers) set({ providers });
      const { event: message } = body as Extract<ProviderEvent, { Message: unknown }>["Message"];
      const channel = eventChannel(message);
      const key: ChannelKey = { provider: id.name, channel };
      const me = scopeOf(get().providers[id.name], channel.organization)?.me;
      const k = keyString(key);
      if (get().conversations[k]) updateConversation(key, (c) => conv.applyEvent(c, message, me));
      const visible = get().visible;
      const onScreen = sameKey(key, visible) && !document.hidden;
      if ("Received" in message) {
        const m = message.Received;
        // Mail sends flag changes as messages again, and its unread state is
        // the read flag, not a count of arrivals.
        const counts = m.author !== me && !onScreen && !m.nonce && !m.email;
        noteLast(key, m, counts ? 1 : 0);
        if (counts) notify(key, m);
        if (onScreen) void markRead(key);
      } else if ("Deleted" in message) {
        const a = get().activity[k];
        if (a?.last?.id === message.Deleted.id) set((s) => ({ activity: { ...s.activity, [k]: { ...a, last: null } } }));
      } else if ("Read" in message && message.Read.user === me) {
        const a = get().activity[k];
        if (a) set((s) => ({ activity: { ...s.activity, [k]: { ...a, unread: 0 } } }));
      } else if ("Gap" in message && sameKey(key, visible)) {
        void loadLatest(key);
      }
      return;
    }
  }
}

function notify(key: ChannelKey, message: Message) {
  if (!document.hidden || typeof Notification === "undefined" || Notification.permission !== "granted") return;
  if (localStorage.getItem("qmsg.notifications") !== "on") return;
  const provider = get().providers[key.provider];
  const scope = scopeOf(provider, key.channel.organization);
  const channel = channelOf(provider, key.channel);
  const author = userName(scope, message.author);
  const where = channel ? channelTitle(scope, channel) : key.channel.channel;
  const title = channel && (channel.kind === "Direct" || where === author) ? author : `${author} · ${where}`;
  const n = new Notification(title, { body: plainText(message).slice(0, 200), tag: keyString(key) });
  n.onclick = () => {
    window.focus();
    show(key);
    n.close();
  };
}

// MARK: Token maintenance

/** Rotates a token that expires within 30 days, persisting the new secret first so a lost reply can be recovered. */
async function maintainToken() {
  const account = get().account;
  if (!account || !backend) return;
  if (account.pendingSecret) {
    await recoverToken();
    return;
  }
  if (account.expiresAtMs - Date.now() > 30 * 24 * 3600 * 1000) return;
  const pending = { ...account, pendingSecret: randomSecret() };
  accounts.save(pending);
  set({ account: pending });
  try {
    const credential = await backend.rotate(pending.pendingSecret);
    adopt(credential.token, credential.expiresAtMs);
  } catch {
    await recoverToken();
  }
}

/** After a rejected token or a lost rotation reply: tries the pending rotation's token. */
async function recoverToken(): Promise<boolean> {
  const account = get().account;
  if (!account || !backend) return false;
  const proposed = proposedToken(account);
  if (!proposed) return false;
  try {
    await backend.snapshot(proposed);
    // The rotation went through. Its expiry is 90 days from then.
    adopt(proposed, Date.now() + 89 * 24 * 3600 * 1000);
    return true;
  } catch (error) {
    if (!isUnauthenticated(error)) return true; // Unreachable; decide later.
    const { pendingSecret: _drop, ...rest } = account;
    accounts.save(rest);
    set({ account: rest });
    try {
      await backend.snapshot();
      return true;
    } catch (e) {
      return !isUnauthenticated(e);
    }
  }
}

function adopt(token: string, expiresAtMs: number) {
  const account = get().account;
  if (!account) return;
  const { pendingSecret: _drop, ...rest } = account;
  const next = { ...rest, token, expiresAtMs };
  accounts.save(next);
  set({ account: next });
  backend?.setToken(token);
}

// MARK: Commands

function isRetryable(error: unknown): boolean {
  if (!(error instanceof RpcError)) return false;
  if (error.code === Code.UNAVAILABLE || error.code === Code.RESOURCE_EXHAUSTED) return true;
  return error.code === Code.FAILED_PRECONDITION && error.message === "provider instance changed; refresh snapshot";
}

/** Runs a command on a provider's current instance, retrying reads and journaled actions after transient failures. */
export async function execute(command: Command, provider: string, operation?: ActionOperation, timeoutMs?: number): Promise<ExecuteResult> {
  for (let attempt = 0; ; attempt++) {
    if (!backend) throw new StoreError("Not connected to the server.");
    const p = get().providers[provider];
    if (!p) throw new StoreError(`The account “${provider}” is no longer available.`);
    try {
      return await backend.execute(command, p.id, operation, timeoutMs);
    } catch (error) {
      if (attempt >= 3 || !isRetryable(error)) throw error;
      if (error instanceof RpcError && error.code === Code.FAILED_PRECONDITION) {
        // The provider restarted; refresh its instance, then retry.
        const snapshot = await backend.snapshot().catch(() => null);
        if (snapshot) applySnapshot(snapshot);
      } else {
        await new Promise((r) => setTimeout(r, 1000 * (attempt + 1)));
      }
    }
  }
}

async function reply(command: Command, provider: string, operation?: ActionOperation, timeoutMs?: number) {
  const result = await execute(command, provider, operation, timeoutMs);
  if (!result.ok) throw result.error;
  return result.reply;
}

// MARK: History

/** Opens a channel. */
export function show(key: ChannelKey | null) {
  const previous = get().visible;
  set({ visible: key });
  if (!key) return;
  const k = keyString(key);
  if (!sameKey(previous, key)) {
    const recent = [key, ...get().recent.filter((r) => !sameKey(r, key))].slice(0, 32);
    localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
    set({ recent });
  }
  const a = get().activity[k];
  if (a?.unread) set((s) => ({ activity: { ...s.activity, [k]: { ...a, unread: 0 } } }));
  const c = get().conversations[k];
  if (!c || !c.loaded || c.stale) void loadLatest(key);
  else void markRead(key);
}

/**
 * Loads the newest page of a channel, reconciling it with what was loaded. A
 * call while one is running loads again once that one is done, since its page
 * may predate whatever prompted the call.
 */
export async function loadLatest(key: ChannelKey) {
  const k = keyString(key);
  if (get().conversations[k]?.isLoading) {
    updateConversation(key, (c) => ({ ...c, reloadQueued: true }));
    return;
  }
  updateConversation(key, (c) => ({ ...c, isLoading: true }));
  try {
    do {
      updateConversation(key, conv.beginLoad);
      try {
        const r = await reply({ History: { channel: key.channel, cursor: null, limit: 50 } }, key.provider);
        if (!("History" in r)) {
          updateConversation(key, (c) => conv.endLoad(c));
          return;
        }
        updateConversation(key, (c) => conv.replace(c, r.History));
        const last = r.History.messages.at(-1);
        if (last) noteLast(key, last);
      } catch (error) {
        updateConversation(key, (c) => conv.endLoad(c, describe(error)));
        return;
      }
    } while (get().conversations[k]?.reloadQueued);
  } finally {
    updateConversation(key, (c) => ({ ...c, isLoading: false }));
  }
  if (sameKey(get().visible, key)) void markRead(key);
}

/** Loads the page before the oldest loaded message. */
export async function loadOlder(key: ChannelKey) {
  const c = conversationOf(get(), key);
  if (!c?.olderCursor || c.isLoadingOlder || c.isLoading) return;
  const generation = c.generation;
  updateConversation(key, (c) => ({ ...c, isLoadingOlder: true }));
  try {
    const r = await reply({ History: { channel: key.channel, cursor: c.olderCursor, limit: 50 } }, key.provider);
    // Dropped if a refresh replaced the history it continues.
    if ("History" in r) updateConversation(key, (c) => conv.prepend(c, r.History, generation));
  } catch (error) {
    updateConversation(key, (c) => ({ ...c, loadError: describe(error) }));
  } finally {
    updateConversation(key, (c) => ({ ...c, isLoadingOlder: false }));
  }
}

/** Fetches one message in full, such as a mail body. */
export async function loadMessage(key: ChannelKey, id: string) {
  const r = await reply({ GetMessage: { channel: key.channel, id } }, key.provider);
  if (!("Message" in r)) throw new StoreError("Couldn't load this message.");
  updateConversation(key, (c) => conv.merge(c, [r.Message]));
  return r.Message;
}

// MARK: Sending

// 128 random bits in 22 URL-safe characters, within Discord's 25-character limit.
function sendNonce(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Sends text and files to a channel. The send shows as pending until the provider reports the message. */
export function send(key: ChannelKey, text: string, files: File[] = [], replyTo: string | null = null) {
  const trimmed = text.trim();
  const content: Content[] = [];
  if (trimmed) content.push({ Text: trimmed });
  const previews: Record<number, string> = {};
  for (const file of files) {
    if (file.type.startsWith("image/") || file.type.startsWith("video/")) previews[content.length] = URL.createObjectURL(file);
    // A placeholder until the upload finishes.
    content.push(contentForFile(file, { name: file.name, mime: file.type, size: file.size, source: { Bytes: [] } }));
  }
  if (!content.length) return;
  const pending: conv.PendingSend = {
    id: sendNonce(),
    content,
    replyTo,
    operation: newOperation(),
    state: { kind: "sending" },
    previews,
    files,
  };
  updateConversation(key, (c) => ({ ...c, pending: [...c.pending, pending] }));
  void deliver(pending, key);
}

/** Tries a failed or uncertain send again, as a new operation. */
export function retry(key: ChannelKey, pending: conv.PendingSend) {
  const again: conv.PendingSend = {
    ...pending,
    id: sendNonce(),
    operation: newOperation(),
    state: { kind: "sending" },
    sentId: undefined,
  };
  updateConversation(key, (c) => ({ ...c, pending: [...c.pending.filter((p) => p.id !== pending.id), again] }));
  void deliver(again, key);
}

export function discard(key: ChannelKey, pending: conv.PendingSend) {
  for (const url of Object.values(pending.previews)) URL.revokeObjectURL(url);
  updateConversation(key, (c) => ({ ...c, pending: c.pending.filter((p) => p.id !== pending.id) }));
}

const isUnsentUpload = (c: Content) => {
  const [, media] = tag(c) as [string, { source?: object }];
  return !!media?.source && "Bytes" in media.source;
};

async function deliver(pending: conv.PendingSend, key: ChannelKey) {
  let content = pending.content;
  let submitted = false;
  try {
    // Upload files first; the media handles are tied to this provider instance.
    const files = [...pending.files];
    const uploaded: Content[] = [];
    for (const part of content) {
      if (!isUnsentUpload(part)) {
        uploaded.push(part);
        continue;
      }
      const file = files.shift();
      if (!file) continue;
      const provider = get().providers[key.provider];
      if (!backend || !provider) throw new StoreError("Not connected to the server.");
      uploaded.push(contentForFile(file, await backend.upload(provider.id, file)));
    }
    content = uploaded;
    updateConversation(key, (c) => conv.updatePending(c, pending.id, (p) => ({ ...p, content })));
    submitted = true;
    const result = await execute(
      { Send: { channel: key.channel, reply_to: pending.replyTo, content, nonce: pending.id } },
      key.provider,
      pending.operation,
    );
    if (result.ok && "Sent" in result.reply) {
      const id = result.reply.Sent.id;
      updateConversation(key, (c) => conv.markSent(c, pending.id, id));
    } else {
      const state: conv.PendingState = result.ok
        ? { kind: "unknown", message: "Unexpected reply" }
        : result.error.deliveryUnknown
          ? { kind: "unknown", message: result.error.message }
          : { kind: "failed", message: result.error.message };
      updateConversation(key, (c) => conv.updatePending(c, pending.id, (p) => ({ ...p, state })));
    }
  } catch (error) {
    // Without an answer, the journal can't say whether it was sent. The
    // server refuses invalid requests and exhausted quotas before running anything.
    const message = describe(error);
    const refused =
      error instanceof RpcError && (error.code === Code.INVALID_ARGUMENT || error.code === Code.RESOURCE_EXHAUSTED);
    updateConversation(key, (c) =>
      conv.updatePending(c, pending.id, (p) => ({
        ...p,
        state:
          !submitted || error instanceof StoreError || refused || p.content.some(isUnsentUpload)
            ? { kind: "failed", message }
            : { kind: "unknown", message },
      })),
    );
  }
}

/** Uploads a file for a function's content input, such as a mail attachment. */
export async function uploadFor(provider: string, file: File): Promise<Content> {
  const p = get().providers[provider];
  if (!backend || !p) throw new StoreError("Not connected to the server.");
  return contentForFile(file, await backend.upload(p.id, file));
}

// MARK: Functions

/** Functions available in a context, such as a message's actions. */
export async function functions(context: FunctionContext, provider: string): Promise<QFunction[]> {
  const r = await reply({ Functions: { context } }, provider);
  return "Functions" in r ? r.Functions : [];
}

/** Calls a discovered function. Actions are journaled with `operation`; pass the same one to retry the same call. */
export async function call(
  fn: QFunction,
  context: FunctionContext,
  args: Arguments,
  provider: string,
  operation?: ActionOperation,
  timeoutMs?: number,
): Promise<Value> {
  const op = actionOf(fn) ? (operation ?? newOperation()) : undefined;
  const r = await reply({ Call: { function: fn.id, context, arguments: args } }, provider, op, timeoutMs);
  return "Value" in r ? r.Value : "Null";
}

/** Suggestions for a function input. */
export async function complete(
  fn: QFunction,
  input: FunctionInput,
  context: FunctionContext,
  args: Arguments,
  query: string,
  provider: string,
): Promise<CompletionItem[]> {
  if (!input.completion) return [];
  const r = await reply(
    {
      Complete: {
        call: { function: input.completion, context, arguments: args },
        input: { function: fn.id, input: input.id },
        query,
        cursor: null,
        limit: 20,
      },
    },
    provider,
  );
  return "Completed" in r ? r.Completed.items : [];
}

/** Checks one input with its verification helper. Returns the problem, if any. */
export async function verify(
  fn: QFunction,
  input: FunctionInput,
  context: FunctionContext,
  args: Arguments,
  value: Value,
  provider: string,
): Promise<string | null> {
  if (!input.verification) return null;
  const r = await reply(
    {
      Verify: {
        call: { function: input.verification, context, arguments: args },
        input: { function: fn.id, input: input.id },
        value,
      },
    },
    provider,
  );
  if (!("Verified" in r) || r.Verified === "Valid") return null;
  return r.Verified.Invalid.map((i) => i.message).join("\n") || "Invalid";
}

/** Opens a conversation with members, or finds the one already open. */
export async function openChannel(provider: string, organization: string | null, members: string[]): Promise<ChannelKey> {
  const r = await reply({ OpenChannel: { organization, members } }, provider, newOperation());
  if (!("Opened" in r)) throw new StoreError("Unexpected reply");
  return { provider, channel: r.Opened };
}

/**
 * Marks a channel read up to its newest message, when the provider offers a
 * read action there. Each mark is a journaled action, and the server keeps a
 * limited number per device, so a message is marked once, the account's own
 * messages not at all, and a burst of arrivals while one mark runs becomes a
 * single mark of the newest.
 */
export async function markRead(key: ChannelKey): Promise<void> {
  const k = keyString(key);
  const last = get().conversations[k]?.messages.at(-1);
  if (marking.has(k) || !last || readUpTo.get(k) === last.id || last.email) return;
  if (last.author === scopeOf(get().providers[key.provider], key.channel.organization)?.me) {
    readUpTo.set(k, last.id);
    return;
  }
  marking.add(k);
  const context: FunctionContext = { Message: { channel: key.channel, id: last.id } };
  try {
    const fns = await functions(context, key.provider);
    const mark = fns.find((f) => actionOf(f) === "MarkRead");
    if (mark) await call(mark, context, {}, key.provider);
    readUpTo.set(k, last.id);
  } catch {
    // Try again with the next message.
  } finally {
    marking.delete(k);
  }
  const newest = get().conversations[k]?.messages.at(-1);
  if (readUpTo.get(k) === last.id && newest && newest.id !== last.id && sameKey(get().visible, key)) {
    await markRead(key);
  }
}

// MARK: Accounts and devices

export async function providerCatalog() {
  if (!backend) throw new StoreError("Not connected to the server.");
  return backend.providerCatalog();
}

export async function configureProvider(settings: Parameters<Backend["configureProvider"]>[0]) {
  if (!backend) throw new StoreError("Not connected to the server.");
  await backend.configureProvider(settings);
  applySnapshot(await backend.snapshot());
}

export async function removeProvider(name: string) {
  if (!backend) throw new StoreError("Not connected to the server.");
  await backend.removeProvider(name);
  applySnapshot(await backend.snapshot());
}

export async function devices() {
  if (!backend) throw new StoreError("Not connected to the server.");
  return backend.devices();
}

export async function revoke(deviceId: string) {
  if (!backend) throw new StoreError("Not connected to the server.");
  await backend.revoke(deviceId);
  if (deviceId === get().account?.deviceId) await unpair();
}

// MARK: Drafts

export function setDraft(key: ChannelKey, text: string) {
  const k = keyString(key);
  set((s) => ({ drafts: { ...s.drafts, [k]: text } }));
}

export function setReplyTo(key: ChannelKey, id: string | null) {
  const k = keyString(key);
  set((s) => ({ replyTo: { ...s.replyTo, [k]: id } }));
}

export { AppError };
