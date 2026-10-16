import { AlertCircle, Menu as MenuIcon, RefreshCw, Users } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Channel, Scope, User } from "~/api/types";
import { cn } from "~/lib/cn";
import { channelOf, channelTitle, isConversation, isMailbox, keyString, scopeOf, userName, type ChannelKey } from "~/store/directory";
import { emptyConversation, typingUsers } from "~/store/conversation";
import { loadLatest, useStore } from "~/store/store";
import { toggleMembers, useUi } from "~/store/ui";
import { Avatar } from "../Avatar";
import { MailView } from "../mail/MailView";
import { channelIcon } from "../Sidebar";
import { Button, EmptyState, Spinner } from "../ui";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";

export function ChatView({ channelKey }: { channelKey: ChannelKey }) {
  const provider = useStore((s) => s.providers[channelKey.provider]);
  const scope = scopeOf(provider, channelKey.channel.organization);
  const channel = channelOf(provider, channelKey.channel);
  if (!provider || !scope || !channel) {
    return (
      <EmptyState icon={<AlertCircle />} title="This conversation isn't available">
        Its account may still be connecting, or the channel was removed.
      </EmptyState>
    );
  }
  if (isMailbox(channel)) return <MailView channelKey={channelKey} channel={channel} />;
  return <Chat channelKey={channelKey} channel={channel} scope={scope} />;
}

function Chat({ channelKey, channel, scope }: { channelKey: ChannelKey; channel: Channel; scope: Scope }) {
  const k = keyString(channelKey);
  const conversation = useStore((s) => s.conversations[k]) ?? emptyConversation();
  const type = useStore((s) => s.types[channelKey.provider]);
  const showMembers = useUi((s) => s.members);
  const title = channelTitle(scope, channel);
  const Icon = channelIcon(channel);
  const others = (channel.members ?? []).filter((m) => m !== scope.me);
  const direct = isConversation(channel) && others.length === 1 ? scope.users[others[0]!] : undefined;
  const hasMembers = !!channel.members?.length || !isConversation(channel);

  useEffect(() => {
    document.title = `${channel.kind === "Direct" ? "" : "#"}${title} · qmsg`;
    return () => {
      document.title = "qmsg";
    };
  }, [title, channel.kind]);

  return (
    <div className="flex h-full min-w-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-13 shrink-0 items-center gap-2.5 border-b border-border px-3 sm:px-4">
          <Button variant="ghost" size="icon-sm" className="md:hidden" onClick={() => useUi.setState({ sidebarOpen: true })} aria-label="Open sidebar">
            <MenuIcon />
          </Button>
          {direct ? (
            <Avatar name={title} seed={direct.id} media={direct.avatar} size={24} presence={scope.presence[direct.id]} className="[--avatar-ring:var(--background)]" />
          ) : (
            <Icon className="size-4 shrink-0 text-muted-foreground" />
          )}
          <h1 className="truncate text-[14px] font-semibold">{title}</h1>
          {channel.topic ? (
            <>
              <span className="hidden h-4 w-px bg-border sm:block" />
              <p className="hidden min-w-0 truncate text-[13px] text-muted-foreground sm:block" title={channel.topic}>
                {channel.topic}
              </p>
            </>
          ) : null}
          <div className="flex-1" />
          <span className="hidden text-2xs text-muted-foreground lg:inline">{channelKey.provider}</span>
          <Button variant="ghost" size="icon-sm" onClick={() => void loadLatest(channelKey)} title="Refresh" aria-label="Refresh">
            <RefreshCw className={cn(conversation.isLoading && "animate-spin")} />
          </Button>
          {hasMembers ? (
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={toggleMembers}
              title="Members"
              aria-label="Toggle members"
              className={cn("hidden lg:inline-flex", showMembers && "bg-accent text-foreground")}
            >
              <Users />
            </Button>
          ) : null}
        </header>
        {conversation.loadError && !conversation.loaded ? (
          <EmptyState icon={<AlertCircle />} title="Couldn't load messages">
            <p>{conversation.loadError}</p>
            <Button className="mt-3" onClick={() => void loadLatest(channelKey)}>Try again</Button>
          </EmptyState>
        ) : !conversation.loaded ? (
          <div className="flex flex-1 items-center justify-center">
            <Spinner className="size-5" />
          </div>
        ) : (
          <MessageList
            channelKey={channelKey}
            conversation={conversation}
            scope={scope}
            header={<ChannelStart title={title} channel={channel} direct={direct} />}
          />
        )}
        {conversation.loadError && conversation.loaded ? (
          <div className="mx-auto mb-1 flex max-w-4xl items-center gap-2 px-5 text-xs text-destructive">
            <AlertCircle className="size-3.5" /> {conversation.loadError}
          </div>
        ) : null}
        <TypingLine typing={conversation.typing} scope={scope} />
        <Composer channelKey={channelKey} channel={channel} scope={scope} messages={conversation.messages} providerType={type} />
      </div>
      {showMembers && hasMembers ? <MemberPanel channel={channel} scope={scope} /> : null}
    </div>
  );
}

function ChannelStart({ title, channel, direct }: { title: string; channel: Channel; direct?: User }) {
  const Icon = channelIcon(channel);
  return (
    <div className="mb-4 pt-8 pb-2">
      {direct ? (
        <Avatar name={title} seed={direct.id} media={direct.avatar} size={56} />
      ) : (
        <div className="flex size-14 items-center justify-center rounded-2xl border border-border bg-card">
          <Icon className="size-6 text-muted-foreground" />
        </div>
      )}
      <h2 className="mt-3 text-xl font-semibold">{direct ? title : `Welcome to #${title}`}</h2>
      <p className="mt-1 text-[13px] text-muted-foreground">
        {direct ? `This is the beginning of your conversation with ${title}.` : channel.topic || "This is the start of the channel."}
      </p>
    </div>
  );
}

function TypingLine({ typing, scope }: { typing: Record<string, number>; scope: Scope }) {
  const [now, setNow] = useState(Date.now());
  const users = typingUsers({ typing } as never, now);
  useEffect(() => {
    if (!Object.keys(typing).length) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [typing]);
  const names = users.map((u) => userName(scope, u));
  return (
    <div className="mx-auto h-5 w-full max-w-4xl px-5 text-2xs text-muted-foreground">
      {names.length ? (
        <span className="flex items-center gap-1.5">
          <span className="flex gap-0.5">
            {[0, 1, 2].map((i) => (
              <span key={i} className="size-1 rounded-full bg-muted-foreground" style={{ animation: `typing-dot 1.2s ${i * 0.15}s infinite` }} />
            ))}
          </span>
          <span>
            <strong className="font-medium text-foreground/80">{names.slice(0, 3).join(", ")}</strong>
            {names.length > 3 ? ` and ${names.length - 3} more` : ""} {names.length === 1 ? "is" : "are"} typing…
          </span>
        </span>
      ) : null}
    </div>
  );
}

const presenceOrder = { Online: 0, Busy: 1, Idle: 2, Offline: 3 } as const;

function MemberPanel({ channel, scope }: { channel: Channel; scope: Scope }) {
  const members = useMemo(() => {
    const ids = channel.members ?? Object.keys(scope.users);
    return ids
      .map((id) => scope.users[id])
      .filter((u): u is User => !!u)
      .sort(
        (a, b) =>
          presenceOrder[scope.presence[a.id] ?? "Offline"] - presenceOrder[scope.presence[b.id] ?? "Offline"] ||
          userName(scope, a.id).localeCompare(userName(scope, b.id)),
      );
  }, [channel.members, scope]);
  const shown = members.slice(0, 300);
  return (
    <aside className="hidden w-60 shrink-0 flex-col border-l border-border bg-sidebar lg:flex">
      <div className="flex h-13 shrink-0 items-center px-4 text-xs font-medium text-muted-foreground">Members — {members.length}</div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {shown.map((u) => {
          const presence = scope.presence[u.id];
          return (
            <div key={u.id} className={cn("flex items-center gap-2.5 rounded-lg px-2 py-1.5", (!presence || presence === "Offline") && "opacity-55")}>
              <Avatar name={userName(scope, u.id)} seed={u.id} media={u.avatar} size={28} presence={presence} />
              <div className="min-w-0">
                <div className="truncate text-[13px]">
                  {userName(scope, u.id)}
                  {u.id === scope.me ? <span className="ml-1 text-2xs text-muted-foreground">(you)</span> : null}
                </div>
                {u.username && u.username !== u.name ? <div className="truncate text-2xs text-muted-foreground">@{u.username}</div> : null}
              </div>
            </div>
          );
        })}
        {members.length > shown.length ? (
          <div className="px-2 py-2 text-2xs text-muted-foreground">and {members.length - shown.length} more</div>
        ) : null}
      </div>
    </aside>
  );
}
