import {
  tag,
  type Channel,
  type ChannelRef,
  type DirectoryUpdate,
  type OrganizationEntry,
  type ProviderId,
  type ProviderStatus,
  type ProviderView,
  type Scope,
  type User,
} from "~/api/types";

/** One provider's account as the app knows it. */
export interface ProviderState {
  id: ProviderId;
  status: ProviderStatus;
  organizations: Record<string, OrganizationEntry>;
  standalone: Scope;
}

export const emptyScope = (): Scope => ({ me: null, users: {}, channels: {}, presence: {} });

export function providerState(view: ProviderView): ProviderState {
  return {
    id: view.provider,
    status: view.status,
    organizations: Object.fromEntries(view.organizations.map((o) => [o.id, o])),
    standalone: view.standalone ?? emptyScope(),
  };
}

export function newProvider(id: ProviderId, status: ProviderStatus = "Connecting"): ProviderState {
  return { id, status, organizations: {}, standalone: emptyScope() };
}

export function scopeOf(p: ProviderState | undefined, organization: string | null | undefined): Scope | undefined {
  if (!p) return undefined;
  return organization == null ? p.standalone : p.organizations[organization]?.scope;
}

export function channelOf(p: ProviderState | undefined, ref: ChannelRef): Channel | undefined {
  return scopeOf(p, ref.organization)?.channels[ref.channel];
}

const byName = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

export function sortedOrganizations(p: ProviderState): OrganizationEntry[] {
  return Object.values(p.organizations).sort((a, b) => byName.compare(a.name, b.name) || a.id.localeCompare(b.id));
}

/** Applies a directory update the way the orchestrator does. Updates for unknown organizations change nothing. */
export function applyDirectory(p: ProviderState, update: DirectoryUpdate): ProviderState {
  const modify = (organization: string | null | undefined, change: (s: Scope) => Scope): ProviderState => {
    if (organization == null) return { ...p, standalone: change(p.standalone) };
    const entry = p.organizations[organization];
    if (!entry) return p;
    return { ...p, organizations: { ...p.organizations, [organization]: { ...entry, scope: change(entry.scope) } } };
  };
  const [name, value] = tag(update);
  switch (name) {
    case "Reset":
      return { ...p, organizations: {}, standalone: emptyScope() };
    case "OrganizationUpserted": {
      const o = value as Extract<DirectoryUpdate, { OrganizationUpserted: unknown }>["OrganizationUpserted"];
      const entry: OrganizationEntry = {
        id: o.id,
        name: o.name,
        icon: o.icon,
        scope: {
          me: o.me,
          users: Object.fromEntries(o.users.map((u) => [u.id, u])),
          channels: Object.fromEntries(o.channels.map((c) => [c.id, c])),
          presence: {},
        },
      };
      return { ...p, organizations: { ...p.organizations, [o.id]: entry } };
    }
    case "OrganizationUpdated": {
      const { id, name, icon } = value as { id: string; name: string; icon?: OrganizationEntry["icon"] };
      const entry = p.organizations[id];
      if (!entry) return p;
      return { ...p, organizations: { ...p.organizations, [id]: { ...entry, name, icon } } };
    }
    case "OrganizationRemoved": {
      const { id } = value as { id: string };
      const { [id]: _removed, ...rest } = p.organizations;
      return { ...p, organizations: rest };
    }
    case "UserUpserted": {
      const { organization, user } = value as { organization?: string | null; user: User };
      return modify(organization, (s) => ({ ...s, users: { ...s.users, [user.id]: user } }));
    }
    case "UserRemoved": {
      const { organization, id } = value as { organization?: string | null; id: string };
      return modify(organization, (s) => {
        const { [id]: _u, ...users } = s.users;
        const { [id]: _p, ...presence } = s.presence;
        return { ...s, users, presence, me: s.me === id ? null : s.me };
      });
    }
    case "ChannelUpserted": {
      const { organization, channel } = value as { organization?: string | null; channel: Channel };
      return modify(organization, (s) => ({ ...s, channels: { ...s.channels, [channel.id]: channel } }));
    }
    case "ChannelRemoved": {
      const ref = value as ChannelRef;
      return modify(ref.organization, (s) => {
        const { [ref.channel]: _c, ...channels } = s.channels;
        return { ...s, channels };
      });
    }
    case "Me": {
      const { organization, id } = value as { organization?: string | null; id?: string | null };
      return modify(organization, (s) => ({ ...s, me: id }));
    }
    case "Presence": {
      const { organization, user, presence } = value as Extract<DirectoryUpdate, { Presence: unknown }>["Presence"];
      return modify(organization, (s) => ({ ...s, presence: { ...s.presence, [user]: presence } }));
    }
    default:
      return p;
  }
}

// MARK: Names

/** A name suitable for chat labels, even before a profile arrives. */
export function userName(scope: Scope | undefined, id: string): string {
  const user = scope?.users[id];
  if (!user) return "Unknown user";
  const name = user.name.trim();
  if (name && name !== id) return name;
  if (user.username) return user.username;
  return "Unknown user";
}

/** A display name for a channel: its name, or for a direct or group conversation without one, its other members. */
export function channelTitle(scope: Scope | undefined, channel: Channel): string {
  const others = (channel.members ?? []).filter((m) => m !== scope?.me);
  if (channel.kind === "Direct") {
    const names = others.filter((id) => scope?.users[id]).map((id) => userName(scope, id)).filter((n) => n !== "Unknown user");
    if (names.length) return names.join(", ");
  }
  if (channel.name) return channel.name;
  return others.length ? others.map((id) => userName(scope, id)).join(", ") : channel.id;
}

/** The identity reported by this account's network, rather than its local label. */
export function accountIdentity(p: ProviderState): string | null {
  for (const scope of [p.standalone, ...sortedOrganizations(p).map((o) => o.scope)]) {
    const id = scope.me;
    if (!id) continue;
    const user = scope.users[id];
    if (user) {
      if (user.username) {
        if (user.username.includes("@")) return user.username;
        return user.name === user.username ? `@${user.username}` : `${user.name} · @${user.username}`;
      }
      if (user.name && user.name !== id) return user.name;
    }
    if (id.includes("@")) return id;
  }
  return null;
}

export function statusLabel(status: ProviderStatus): string {
  const [name, value] = tag(status);
  switch (name) {
    case "Connecting":
      return "Connecting";
    case "Syncing":
      return "Syncing";
    case "Ready":
      return "Connected";
    case "LoginRequired":
      return "Sign-in required";
    case "Disconnected": {
      const reason = (value as { reason: string }).reason;
      return reason ? `Disconnected: ${reason}` : "Disconnected";
    }
    case "Failed":
      return `Failed: ${value}`;
    default:
      return name;
  }
}

export type StatusTone = "ok" | "busy" | "attention" | "error";

export function statusTone(status: ProviderStatus): StatusTone {
  const [name] = tag(status);
  if (name === "Ready") return "ok";
  if (name === "LoginRequired") return "attention";
  if (name === "Failed" || name === "Disconnected") return "error";
  return "busy";
}

// MARK: Channel trees

export function isContainer(channel: Channel): boolean {
  return channel.kind === "Category" || channel.kind === "Forum";
}

export function isWritable(channel: Channel): boolean {
  return channel.accepted_content.length > 0;
}

export function isConversation(channel: Channel): boolean {
  return channel.kind === "Direct" || channel.kind === "Group";
}

export function isMailbox(channel: Channel): boolean {
  return !!channel.mailbox || (typeof channel.kind === "object" && channel.kind.Custom === "mailbox");
}

function sortKey(a: Channel, b: Channel): number {
  const pa = a.position ?? Number.MAX_SAFE_INTEGER;
  const pb = b.position ?? Number.MAX_SAFE_INTEGER;
  return pa - pb || byName.compare(a.name, b.name) || a.id.localeCompare(b.id);
}

export interface ChannelNode {
  channel: Channel;
  depth: number;
}

/** The channels of a scope as a tree: top-level channels and categories, each followed by its children. */
export function channelTree(scope: Scope): ChannelNode[] {
  const children = new Map<string | null, Channel[]>();
  for (const channel of Object.values(scope.channels)) {
    const parent = channel.parent && scope.channels[channel.parent] ? channel.parent : null;
    const list = children.get(parent) ?? [];
    list.push(channel);
    children.set(parent, list);
  }
  const result: ChannelNode[] = [];
  const visit = (parent: string | null, depth: number) => {
    for (const channel of (children.get(parent) ?? []).sort(sortKey)) {
      result.push({ channel, depth });
      if (depth < 4) visit(channel.id, depth + 1);
    }
  };
  visit(null, 0);
  return result;
}

// MARK: Mail

export enum MailboxRole {
  Inbox,
  Tab,
  Starred,
  Important,
  Drafts,
  Sent,
  Archive,
  All,
  Junk,
  Trash,
  Other,
}

/** The folder's own name, without its parents: "Sent Mail" for "[Gmail]/Sent Mail". */
export function mailboxName(channel: Channel): string {
  if (channel.id.toUpperCase() === "INBOX") return "Inbox";
  const delimiter = channel.mailbox?.delimiter;
  if (!delimiter) return channel.name;
  return channel.name.split(delimiter).filter(Boolean).at(-1) ?? channel.name;
}

/** What a mail folder is for, from its IMAP special-use attributes, or the usual names. */
export function mailboxRole(channel: Channel): MailboxRole {
  const attributes = new Set((channel.mailbox?.attributes ?? []).map((a) => a.toLowerCase()));
  if (channel.id.toUpperCase() === "INBOX") return MailboxRole.Inbox;
  if (attributes.has("$gmailtab")) return MailboxRole.Tab;
  if (attributes.has("\\flagged")) return MailboxRole.Starred;
  if (attributes.has("\\important")) return MailboxRole.Important;
  if (attributes.has("\\drafts")) return MailboxRole.Drafts;
  if (attributes.has("\\sent")) return MailboxRole.Sent;
  if (attributes.has("\\archive")) return MailboxRole.Archive;
  if (attributes.has("\\all")) return MailboxRole.All;
  if (attributes.has("\\junk")) return MailboxRole.Junk;
  if (attributes.has("\\trash")) return MailboxRole.Trash;
  switch (mailboxName(channel).toLowerCase()) {
    case "drafts":
    case "draft":
      return MailboxRole.Drafts;
    case "sent":
    case "sent mail":
    case "sent items":
    case "sent messages":
      return MailboxRole.Sent;
    case "archive":
      return MailboxRole.Archive;
    case "junk":
    case "spam":
    case "junk e-mail":
    case "junk email":
      return MailboxRole.Junk;
    case "trash":
    case "deleted items":
    case "deleted messages":
    case "bin":
      return MailboxRole.Trash;
    default:
      return MailboxRole.Other;
  }
}

/** Mail folders in the order Mail shows them: Inbox and special folders first, then the rest by name. */
export function mailboxes(scope: Scope): Channel[] {
  return Object.values(scope.channels)
    .filter(isMailbox)
    .sort((a, b) => mailboxRole(a) - mailboxRole(b) || byName.compare(a.name, b.name) || a.id.localeCompare(b.id));
}

// MARK: Keys

/** Identifies a channel across providers. */
export interface ChannelKey {
  provider: string;
  channel: ChannelRef;
}

export function keyString(key: ChannelKey): string {
  return `${key.provider}\u001f${key.channel.organization ?? ""}\u001f${key.channel.channel}`;
}

export function sameKey(a: ChannelKey | null | undefined, b: ChannelKey | null | undefined): boolean {
  return !!a && !!b && keyString(a) === keyString(b);
}
