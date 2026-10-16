import {
  AtSign,
  ChevronRight,
  Hash,
  Inbox,
  KeyRound,
  Mail,
  Megaphone,
  MessagesSquare,
  Search,
  Send,
  Settings,
  Star,
  Trash2,
  Volume2,
  Archive,
  FileText,
  ShieldAlert,
  Folder,
  type LucideIcon,
} from "lucide-react";
import { memo, useMemo, useState, type ReactNode } from "react";
import { tag, type Channel, type OrganizationEntry, type Scope } from "~/api/types";
import { cn } from "~/lib/cn";
import { formatStamp, plainText } from "~/lib/content";
import {
  accountIdentity,
  channelTitle,
  channelTree,
  isContainer,
  isConversation,
  isMailbox,
  keyString,
  mailboxes,
  mailboxName,
  mailboxRole,
  MailboxRole,
  sameKey,
  sortedOrganizations,
  statusLabel,
  statusTone,
  type ChannelKey,
  type ProviderState,
} from "~/store/directory";
import { show, useStore, type Activity } from "~/store/store";
import { toggleCollapsed, useUi } from "~/store/ui";
import { Avatar } from "./Avatar";
import { ConnectionDot } from "./ConnectionDot";
import { Kbd } from "./ui";

export function channelIcon(channel: Channel): LucideIcon {
  if (isMailbox(channel)) {
    switch (mailboxRole(channel)) {
      case MailboxRole.Inbox:
      case MailboxRole.Tab:
        return Inbox;
      case MailboxRole.Sent:
        return Send;
      case MailboxRole.Drafts:
        return FileText;
      case MailboxRole.Starred:
      case MailboxRole.Important:
        return Star;
      case MailboxRole.Trash:
        return Trash2;
      case MailboxRole.Junk:
        return ShieldAlert;
      case MailboxRole.Archive:
      case MailboxRole.All:
        return Archive;
      default:
        return Folder;
    }
  }
  const [kind] = tag(channel.kind);
  switch (kind) {
    case "Voice":
    case "Video":
      return Volume2;
    case "Announcement":
      return Megaphone;
    case "Forum":
      return MessagesSquare;
    case "Direct":
    case "Group":
      return AtSign;
    default:
      return Hash;
  }
}

const toneDot = {
  ok: "bg-success",
  busy: "bg-warning animate-pulse",
  attention: "bg-warning",
  error: "bg-destructive",
};

export function Sidebar() {
  const providers = useStore((s) => s.providers);
  const link = useStore((s) => s.link);
  const demo = useStore((s) => s.demo);
  const sorted = useMemo(() => Object.values(providers).sort((a, b) => a.id.name.localeCompare(b.id.name)), [providers]);
  return (
    <aside className="flex h-full w-full flex-col bg-sidebar text-sidebar-foreground">
      <div className="flex h-13 shrink-0 items-center gap-2 px-3">
        <div className="flex items-center gap-2 pl-1">
          <img src="/favicon.svg" alt="" className="size-5" />
          <span className="text-[15px] font-semibold tracking-tight">qmsg</span>
          {demo ? (
            <span className="rounded-md bg-primary/15 px-1.5 py-0.5 text-2xs font-medium text-primary">Demo</span>
          ) : null}
        </div>
        <div className="flex-1" />
        <ConnectionDot link={link} />
      </div>
      <div className="px-2 pb-2">
        <button
          onClick={() => useUi.setState({ paletteOpen: true })}
          className="flex h-8 w-full items-center gap-2 rounded-lg border border-border bg-background/60 px-2.5 text-[13px] text-muted-foreground transition-colors hover:bg-sidebar-row-hover"
        >
          <Search className="size-3.5" />
          <span className="flex-1 text-left">Jump to…</span>
          <Kbd>⌘K</Kbd>
        </button>
      </div>
      <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {sorted.length === 0 ? (
          <div className="px-2 py-6 text-center text-[13px] text-muted-foreground">
            {link.kind === "live" ? "No accounts yet. Add one in Settings." : "Loading accounts…"}
          </div>
        ) : null}
        {sorted.map((p) => (
          <ProviderSection key={p.id.name} provider={p} />
        ))}
      </nav>
      <div className="flex shrink-0 items-center gap-1 border-t border-sidebar-border p-2">
        <SidebarButton icon={Settings} label="Settings" onClick={() => useUi.setState({ settings: "accounts" })} />
      </div>
    </aside>
  );
}

function SidebarButton({ icon: Icon, label, onClick }: { icon: LucideIcon; label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className="flex h-8 flex-1 items-center gap-2 rounded-lg px-2 text-[13px] text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
    >
      <Icon className="size-4" />
      {label}
    </button>
  );
}

const ProviderSection = memo(function ProviderSection({ provider }: { provider: ProviderState }) {
  const name = provider.id.name;
  const collapsed = useUi((s) => !!s.collapsed[name]);
  const page = useUi((s) => s.providerPage === name);
  const identity = accountIdentity(provider);
  const tone = statusTone(provider.status);
  const [status] = tag(provider.status);
  const orgs = sortedOrganizations(provider);
  const mail = mailboxes(provider.standalone);
  return (
    <section className="mt-2 first:mt-0">
      <div
        className={cn(
          "group flex h-8 items-center gap-1.5 rounded-lg pr-1 pl-1.5 hover:bg-sidebar-row-hover",
          page && "bg-sidebar-row-active",
        )}
      >
        <button onClick={() => toggleCollapsed(name)} className="flex size-5 items-center justify-center text-sidebar-muted-foreground" aria-label={collapsed ? "Expand" : "Collapse"}>
          <ChevronRight className={cn("size-3.5 transition-transform", !collapsed && "rotate-90")} />
        </button>
        <button
          onClick={() => {
            useUi.setState({ providerPage: name, sidebarOpen: false });
            show(null);
          }}
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
          title={statusLabel(provider.status)}
        >
          <span className="truncate text-xs font-semibold tracking-wide text-sidebar-muted-foreground uppercase">{name}</span>
          {identity ? <span className="truncate text-xs text-sidebar-muted-foreground/70">{identity}</span> : null}
          <span className={cn("ml-auto size-1.5 shrink-0 rounded-full", toneDot[tone])} />
        </button>
      </div>
      {collapsed ? null : (
        <div className="mt-0.5">
          {status === "LoginRequired" ? (
            <Row
              icon={<KeyRound className="size-4 text-warning" />}
              label="Sign in to continue"
              active={page}
              onClick={() => {
                useUi.setState({ providerPage: name, sidebarOpen: false });
                show(null);
              }}
            />
          ) : null}
          {mail.length ? <MailboxList provider={provider} channels={mail} /> : null}
          <Conversations provider={provider} scope={provider.standalone} />
          {orgs.map((org) => (
            <OrganizationSection key={org.id} provider={provider} org={org} />
          ))}
        </div>
      )}
    </section>
  );
});

function Row({
  icon,
  label,
  detail,
  active,
  unread,
  depth = 0,
  onClick,
  muted,
  stamp,
}: {
  icon: ReactNode;
  label: ReactNode;
  detail?: ReactNode;
  active?: boolean;
  unread?: number;
  depth?: number;
  onClick: () => void;
  muted?: boolean;
  stamp?: string;
}) {
  return (
    <button
      onClick={onClick}
      style={{ paddingLeft: 8 + depth * 12 }}
      className={cn(
        "group flex w-full items-center gap-2 rounded-lg py-1 pr-2 text-left text-[13px] transition-colors",
        detail ? "min-h-11" : "min-h-8",
        active
          ? "bg-sidebar-row-active text-sidebar-foreground shadow-[0_0_0_1px_var(--border)]"
          : "text-sidebar-foreground/85 hover:bg-sidebar-row-hover",
        muted && !active && "text-sidebar-muted-foreground",
      )}
    >
      {icon}
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className={cn("truncate", unread ? "font-semibold text-sidebar-foreground" : "")}>{label}</span>
          {stamp ? <span className="ml-auto shrink-0 text-2xs text-sidebar-muted-foreground">{stamp}</span> : null}
        </span>
        {detail ? <span className="block truncate text-xs text-sidebar-muted-foreground">{detail}</span> : null}
      </span>
      {unread ? (
        <span className="min-w-5 shrink-0 rounded-full bg-primary px-1.5 text-center text-2xs leading-5 font-semibold text-primary-foreground">
          {unread > 99 ? "99+" : unread}
        </span>
      ) : null}
    </button>
  );
}

function open(provider: string, channel: Channel, organization: string | null) {
  useUi.setState({ providerPage: null, sidebarOpen: false });
  show({ provider, channel: { organization, channel: channel.id } });
}

function useIsVisible(key: ChannelKey) {
  return useStore((s) => sameKey(s.visible, key));
}

/** Direct and group conversations outside any organization, newest first. */
function Conversations({ provider, scope }: { provider: ProviderState; scope: Scope }) {
  const activity = useStore((s) => s.activity);
  const name = provider.id.name;
  const [all, setAll] = useState(false);
  const channels = useMemo(() => {
    const list = Object.values(scope.channels).filter((c) => !isMailbox(c) && !isContainer(c));
    const last = (c: Channel) => activity[keyString({ provider: name, channel: { organization: null, channel: c.id } })]?.last?.sent_at ?? 0;
    return list.sort((a, b) => last(b) - last(a) || channelTitle(scope, a).localeCompare(channelTitle(scope, b)));
  }, [scope, activity, name]);
  if (!channels.length) return null;
  const shown = all ? channels : channels.slice(0, 12);
  return (
    <div className="mt-1">
      <div className="px-2 pt-1.5 pb-1 text-2xs font-medium tracking-wide text-sidebar-muted-foreground/80">Direct messages</div>
      {shown.map((c) => (
        <ConversationRow key={c.id} provider={name} scope={scope} channel={c} activity={activity[keyString({ provider: name, channel: { organization: null, channel: c.id } })]} />
      ))}
      {channels.length > shown.length ? (
        <button onClick={() => setAll(true)} className="w-full rounded-lg px-2 py-1 text-left text-xs text-sidebar-muted-foreground hover:bg-sidebar-row-hover">
          Show {channels.length - shown.length} more
        </button>
      ) : null}
    </div>
  );
}

const ConversationRow = memo(function ConversationRow({
  provider,
  scope,
  channel,
  activity,
}: {
  provider: string;
  scope: Scope;
  channel: Channel;
  activity?: Activity;
}) {
  const key = { provider, channel: { organization: null, channel: channel.id } };
  const active = useIsVisible(key);
  const title = channelTitle(scope, channel);
  const others = (channel.members ?? []).filter((m) => m !== scope.me);
  const single = isConversation(channel) && others.length === 1 ? others[0]! : null;
  const user = single ? scope.users[single] : undefined;
  const last = activity?.last;
  const preview = last ? `${last.author === scope.me ? "You: " : ""}${plainText(last)}` : undefined;
  return (
    <Row
      active={active}
      unread={activity?.unread}
      onClick={() => open(provider, channel, null)}
      label={title}
      detail={preview}
      stamp={last ? formatStamp(last.sent_at) : undefined}
      icon={
        <Avatar
          name={title}
          seed={channel.id}
          media={user?.avatar ?? channel.icon}
          size={28}
          presence={single ? scope.presence[single] : undefined}
        />
      }
    />
  );
});

function MailboxList({ provider, channels }: { provider: ProviderState; channels: Channel[] }) {
  const activity = useStore((s) => s.activity);
  return (
    <div className="mt-1">
      <div className="flex items-center gap-1.5 px-2 pt-1.5 pb-1 text-2xs font-medium tracking-wide text-sidebar-muted-foreground/80">
        <Mail className="size-3" /> Mail
      </div>
      {channels.map((c) => {
        const key = { provider: provider.id.name, channel: { organization: null, channel: c.id } };
        return <ChannelRow key={c.id} channelKey={key} channel={c} label={mailboxName(c)} unread={activity[keyString(key)]?.unread} />;
      })}
    </div>
  );
}

function ChannelRow({
  channelKey,
  channel,
  label,
  depth,
  unread,
}: {
  channelKey: ChannelKey;
  channel: Channel;
  label: string;
  depth?: number;
  unread?: number;
}) {
  const active = useIsVisible(channelKey);
  const Icon = channelIcon(channel);
  return (
    <Row
      active={active}
      depth={depth}
      unread={unread}
      muted={!unread}
      onClick={() => open(channelKey.provider, channel, channelKey.channel.organization)}
      label={label}
      icon={<Icon className="size-4 shrink-0 text-sidebar-muted-foreground" />}
    />
  );
}

const OrganizationSection = memo(function OrganizationSection({ provider, org }: { provider: ProviderState; org: OrganizationEntry }) {
  const name = provider.id.name;
  const id = `${name}/${org.id}`;
  const collapsedMap = useUi((s) => s.collapsed);
  const collapsed = !!collapsedMap[id];
  const activity = useStore((s) => s.activity);
  const tree = useMemo(() => channelTree(org.scope), [org.scope]);
  const hidden = new Set<string>();
  const unread = tree.reduce(
    (n, { channel }) => n + (activity[keyString({ provider: name, channel: { organization: org.id, channel: channel.id } })]?.unread ?? 0),
    0,
  );
  return (
    <div className="mt-1">
      <button
        onClick={() => toggleCollapsed(id)}
        className="flex h-8 w-full items-center gap-2 rounded-lg px-2 text-left text-[13px] font-medium hover:bg-sidebar-row-hover"
      >
        <Avatar name={org.name} seed={org.id} media={org.icon} size={20} square />
        <span className="min-w-0 flex-1 truncate">{org.name}</span>
        {collapsed && unread ? <span className="size-2 rounded-full bg-primary" /> : null}
        <ChevronRight className={cn("size-3.5 text-sidebar-muted-foreground transition-transform", !collapsed && "rotate-90")} />
      </button>
      {collapsed
        ? null
        : tree.map(({ channel, depth }) => {
            if (channel.parent && hidden.has(channel.parent)) {
              hidden.add(channel.id);
              return null;
            }
            const key = { provider: name, channel: { organization: org.id, channel: channel.id } };
            if (channel.kind === "Category") {
              const cid = `${id}/${channel.id}`;
              if (collapsedMap[cid]) hidden.add(channel.id);
              return <CategoryRow key={channel.id} id={cid} name={channel.name} depth={depth} />;
            }
            return (
              <ChannelRow
                key={channel.id}
                channelKey={key}
                channel={channel}
                label={channelTitle(org.scope, channel)}
                depth={depth}
                unread={activity[keyString(key)]?.unread}
              />
            );
          })}
    </div>
  );
});

function CategoryRow({ id, name, depth }: { id: string; name: string; depth: number }) {
  const collapsed = useUi((s) => !!s.collapsed[id]);
  return (
    <button
      onClick={() => toggleCollapsed(id)}
      style={{ paddingLeft: 8 + depth * 12 }}
      className="mt-1.5 flex h-6 w-full items-center gap-1 pr-2 text-left text-2xs font-semibold tracking-wide text-sidebar-muted-foreground uppercase hover:text-sidebar-foreground"
    >
      <ChevronRight className={cn("size-3 transition-transform", !collapsed && "rotate-90")} />
      <span className="truncate">{name}</span>
    </button>
  );
}
