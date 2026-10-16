// The loaded messages of one channel, and how history pages and live events
// reconcile. Mirrors the iOS app's Conversation; every function returns a
// new state.

import type { ActionOperation } from "~/api/client";
import { describeCommand } from "~/api/errors";
import type { Content, HistoryPage, Message, MessageEvent, Reaction } from "~/api/types";

export type PendingState =
  | { kind: "sending" }
  /** Definitely not sent; safe to try again. */
  | { kind: "failed"; message: string }
  /** Maybe sent. Check the conversation before sending again. */
  | { kind: "unknown"; message: string };

/** A send the app is waiting on, shown in its channel until the provider reports the message. */
export interface PendingSend {
  /** The send's nonce. */
  id: string;
  content: Content[];
  replyTo: string | null;
  operation: ActionOperation;
  state: PendingState;
  /** The id the provider answered with, once it has. */
  sentId?: string;
  /** Local previews of attachments, as object URLs, by content index. */
  previews: Record<number, string>;
  /** Original files, kept until the send settles so failed uploads can retry. */
  files: File[];
}

export interface Conversation {
  /** Oldest first, unique by id. */
  messages: Message[];
  pending: PendingSend[];
  olderCursor: string | null;
  reachedStart: boolean;
  isLoading: boolean;
  isLoadingOlder: boolean;
  loadError: string | null;
  loaded: boolean;
  /** Users typing, until when (ms). */
  typing: Record<string, number>;
  /** The account's own read marker, if the provider reports one. */
  readUpTo: string | null;
  /** Needs a reload: after a gap or a fresh snapshot. */
  stale: boolean;
  /** Bumped when loaded history is dropped for a page it doesn't join up with. */
  generation: number;
  /** Ids changed and deleted by events while a newest page is on its way; they win over it. */
  during: { changed: Set<string>; deleted: Set<string> } | null;
  /** Another newest page is wanted once the running load finishes. */
  reloadQueued: boolean;
}

export const emptyConversation = (): Conversation => ({
  messages: [],
  pending: [],
  olderCursor: null,
  reachedStart: false,
  isLoading: false,
  isLoadingOlder: false,
  loadError: null,
  loaded: false,
  typing: {},
  readUpTo: null,
  stale: false,
  generation: 0,
  during: null,
  reloadQueued: false,
});

function mailId(m: Message): number | null {
  const last = m.id.split(":").at(-1);
  const n = last ? Number(last) : NaN;
  return Number.isSafeInteger(n) ? n : null;
}

export function precedes(a: Message, b: Message): boolean {
  if (a.sent_at === b.sent_at && a.email && b.email) {
    const left = mailId(a);
    const right = mailId(b);
    if (left !== null && right !== null && left !== right) return left < right;
  }
  return a.sent_at !== b.sent_at ? a.sent_at < b.sent_at : a.id < b.id;
}

const sorted = (messages: Message[]) => [...messages].sort((a, b) => (precedes(a, b) ? -1 : precedes(b, a) ? 1 : 0));

function unique(messages: Message[]): Message[] {
  const seen = new Set<string>();
  return messages.filter((m) => !seen.has(m.id) && (seen.add(m.id), true));
}

export function mailNeedsBody(m: Message): boolean {
  return m.content.some((c) => "Custom" in c && c.Custom.kind === "email-summary");
}

/** IMAP bodies are immutable for an identity; refreshing flags and headers must not discard a body already fetched. */
function keepingMailBody(next: Message, previous: Message | undefined): Message {
  if (!mailNeedsBody(next) || !previous?.email || mailNeedsBody(previous) || !next.email) return next;
  return {
    ...next,
    content: previous.content,
    email: { ...next.email, html: previous.email.html, content_ids: previous.email.content_ids },
  };
}

/** Drops pending sends the provider has reported. */
function settlePending(c: Conversation): Conversation {
  const nonces = new Set(c.messages.map((m) => m.nonce).filter(Boolean));
  const ids = new Set(c.messages.map((m) => m.id));
  const pending = c.pending.filter((p) => !nonces.has(p.id) && !(p.sentId && ids.has(p.sentId)));
  return pending.length === c.pending.length ? c : { ...c, pending };
}

/** Call before requesting the newest page that `replace` will take. */
export function beginLoad(c: Conversation): Conversation {
  return { ...c, during: { changed: new Set(), deleted: new Set() }, reloadQueued: false };
}

/** Call when a load ends without a page. */
export function endLoad(c: Conversation, error: string | null = null): Conversation {
  return { ...c, during: null, loadError: error ?? c.loadError };
}

/**
 * Takes the newest page. Messages older than it are kept when the page joins
 * up with them; within the page's span the page is the truth, so messages
 * missing from it were deleted.
 */
export function replace(c: Conversation, page: HistoryPage): Conversation {
  const changed = c.during?.changed ?? new Set<string>();
  const deleted = c.during?.deleted ?? new Set<string>();
  const incoming = sorted(page.messages);
  const pageIds = new Set(incoming.map((m) => m.id));
  const current = new Map(c.messages.map((m) => [m.id, m]));
  let result: Message[] = [];
  let { olderCursor, reachedStart, generation } = c;
  const oldest = incoming[0];
  if (oldest && page.next_cursor && c.messages.some((m) => pageIds.has(m.id))) {
    // Joins up: keep older history and its cursor.
    result = c.messages.filter((m) => precedes(m, oldest));
  } else {
    if (c.messages.length) generation += 1;
    olderCursor = page.next_cursor ?? null;
    reachedStart = !page.next_cursor;
  }
  // Prefer what events said since the request went out.
  result.push(...incoming.map((m) => (changed.has(m.id) ? (current.get(m.id) ?? m) : keepingMailBody(m, current.get(m.id)))));
  result.push(...c.messages.filter((m) => changed.has(m.id) && !pageIds.has(m.id)));
  return settlePending({
    ...c,
    messages: sorted(unique(result.filter((m) => !deleted.has(m.id)))),
    olderCursor,
    reachedStart,
    generation,
    during: null,
    loaded: true,
    stale: false,
    loadError: null,
  });
}

/** Adds an older page; changes nothing if the history it was requested for has since been replaced. */
export function prepend(c: Conversation, page: HistoryPage, generation: number): Conversation {
  if (generation !== c.generation) return c;
  return { ...merge(c, page.messages), olderCursor: page.next_cursor ?? null, reachedStart: !page.next_cursor };
}

export function merge(c: Conversation, incoming: Message[]): Conversation {
  const byId = new Map(c.messages.map((m) => [m.id, m]));
  for (const m of incoming) byId.set(m.id, keepingMailBody(m, byId.get(m.id)));
  return settlePending({ ...c, messages: sorted([...byId.values()]) });
}

function react(reactions: Reaction[], key: string, added: boolean, mine: boolean): Reaction[] {
  const i = reactions.findIndex((r) => r.key === key);
  if (i < 0) return added ? [...reactions, { key, count: 1, me: mine }] : reactions;
  const r = reactions[i]!;
  const next = added
    ? { ...r, count: r.count + 1, me: mine ? true : r.me }
    : { ...r, count: Math.max(0, r.count - 1), me: mine ? false : r.me };
  return next.count === 0 ? reactions.filter((_, j) => j !== i) : reactions.map((x, j) => (j === i ? next : x));
}

const TYPING_MS = 10_000;

export function applyEvent(c: Conversation, event: MessageEvent, me: string | null | undefined, now = Date.now()): Conversation {
  if (c.during) {
    const during = { changed: new Set(c.during.changed), deleted: new Set(c.during.deleted) };
    if ("Received" in event) {
      during.changed.add(event.Received.id);
      during.deleted.delete(event.Received.id);
    } else if ("Edited" in event) during.changed.add(event.Edited.id);
    else if ("Reacted" in event) during.changed.add(event.Reacted.id);
    else if ("Deleted" in event) during.deleted.add(event.Deleted.id);
    c = { ...c, during };
  }
  const update = (id: string, change: (m: Message) => Message): Conversation => {
    const i = c.messages.findIndex((m) => m.id === id);
    if (i < 0) return c;
    return { ...c, messages: c.messages.map((m, j) => (j === i ? change(m) : m)) };
  };
  if ("Received" in event) {
    const m = event.Received;
    const { [m.author]: _gone, ...typing } = c.typing;
    c = { ...c, typing };
    if (c.loaded || c.during) return merge(c, [m]);
    return { ...c, pending: c.pending.filter((p) => p.id !== m.nonce && p.sentId !== m.id) };
  }
  if ("Edited" in event) {
    const { id, content, edited_at } = event.Edited;
    return update(id, (m) => ({ ...m, content, edited_at }));
  }
  if ("Deleted" in event) {
    return { ...c, messages: c.messages.filter((m) => m.id !== event.Deleted.id) };
  }
  if ("Reacted" in event) {
    const { id, user, key, added } = event.Reacted;
    return update(id, (m) => ({ ...m, reactions: react(m.reactions, key, added, user === me) }));
  }
  if ("Read" in event) {
    return event.Read.user === me ? { ...c, readUpTo: event.Read.up_to ?? null } : c;
  }
  if ("Typing" in event) {
    if (event.Typing.user === me) return c;
    return { ...c, typing: { ...c.typing, [event.Typing.user]: now + TYPING_MS } };
  }
  if ("Gap" in event) return { ...c, stale: true };
  if ("NotSent" in event) {
    const { nonce, error } = event.NotSent;
    return updatePending(c, nonce, (p) => ({ ...p, state: { kind: "failed", message: describeCommand(error) } }));
  }
  return c;
}

export function updatePending(c: Conversation, nonce: string, change: (p: PendingSend) => PendingSend): Conversation {
  const i = c.pending.findIndex((p) => p.id === nonce);
  if (i < 0) return c;
  return { ...c, pending: c.pending.map((p, j) => (j === i ? change(p) : p)) };
}

export function markSent(c: Conversation, nonce: string, id: string): Conversation {
  if (c.messages.some((m) => m.id === id)) return { ...c, pending: c.pending.filter((p) => p.id !== nonce) };
  return updatePending(c, nonce, (p) => ({ ...p, sentId: id }));
}

export function typingUsers(c: Conversation, now = Date.now()): string[] {
  return Object.entries(c.typing)
    .filter(([, until]) => until > now)
    .map(([user]) => user)
    .sort();
}
