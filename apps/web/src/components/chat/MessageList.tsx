import {
  AlertTriangle,
  Copy,
  CornerUpLeft,
  EyeOff,
  Loader2,
  MoreHorizontal,
  Pencil,
  RotateCcw,
  SmilePlus,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { describe } from "~/api/errors";
import { actionOf, type Content, type FunctionContext, type Message, type QFunction, type Scope } from "~/api/types";
import { cn } from "~/lib/cn";
import { customJson, customLabel, formatDay, formatLong, formatTime, mediaOf, plainText, sameDay, textOf } from "~/lib/content";
import type { Conversation, PendingSend } from "~/store/conversation";
import { userName, type ChannelKey } from "~/store/directory";
import { call, discard, functions, loadOlder, retry, setReplyTo, useStore } from "~/store/store";
import { toast } from "~/store/ui";
import { Avatar } from "../Avatar";
import { FunctionDialog } from "../FunctionDialog";
import { Button, Menu, Spinner, Textarea, type MenuItem } from "../ui";
import { safeUrl } from "~/store/media";
import { ExternalLink, Formatted, Linkified } from "./Formatted";
import { DiscordText } from "./DiscordText";
import { MediaPart } from "./MediaPart";

const QUICK = ["👍", "❤️", "😂", "🎉", "😮", "😢", "🙏", "🔥"];
const EMOJI = [
  ...QUICK,
  "👀", "✅", "❌", "💯", "🚀", "🤔", "😅", "😍", "🥲", "😎", "🙌", "👏", "💀", "😭", "🤝", "⭐",
  "👋", "🤯", "😴", "🫡", "💡", "📌", "⚡", "🍕",
];

/** A message's functions, fetched on first use. */
function useMessageFunctions(key: ChannelKey, id: string) {
  const [fns, setFns] = useState<QFunction[] | null>(null);
  const load = useCallback(async () => {
    if (fns) return fns;
    const result = await functions({ Message: { channel: key.channel, id } }, key.provider);
    setFns(result);
    return result;
  }, [fns, key, id]);
  return load;
}

export function MessageList({
  channelKey,
  conversation,
  scope,
  header,
}: {
  channelKey: ChannelKey;
  conversation: Conversation;
  scope: Scope | undefined;
  header: ReactNode;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const top = useRef<HTMLDivElement>(null);
  const { messages, pending } = conversation;
  const byId = useMemo(() => new Map(messages.map((m) => [m.id, m])), [messages]);

  // Older pages load as the top comes into view. The list is laid out from
  // the bottom (column-reverse), so prepending keeps the reader's place.
  useEffect(() => {
    const el = top.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      (entries) => entries.some((e) => e.isIntersecting) && void loadOlder(channelKey),
      { root: scroller.current, rootMargin: "600px 0px 0px 0px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [channelKey, conversation.olderCursor]);

  // A send of one's own scrolls back to the newest message.
  const pendingCount = pending.length;
  useLayoutEffect(() => {
    if (pendingCount) scroller.current?.scrollTo({ top: 0 });
  }, [pendingCount]);

  return (
    <div ref={scroller} className="flex min-h-0 flex-1 flex-col-reverse overflow-y-auto overscroll-contain [overflow-anchor:none]">
      <div className="mx-auto w-full max-w-4xl px-3 pt-4 pb-3 sm:px-5">
        <div ref={top} />
        {conversation.reachedStart ? (
          header
        ) : conversation.olderCursor ? (
          <div className="flex justify-center py-4">
            {conversation.isLoadingOlder ? <Spinner /> : <span className="h-4" />}
          </div>
        ) : null}
        {messages.map((m, i) => (
          <MessageRow
            key={m.id}
            message={m}
            previous={messages[i - 1]}
            scope={scope}
            channelKey={channelKey}
            replied={m.reply_to ? (byId.get(m.reply_to) ?? null) : undefined}
          />
        ))}
        {pending.map((p) => (
          <PendingRow key={p.id} pending={p} channelKey={channelKey} scope={scope} />
        ))}
      </div>
    </div>
  );
}

function grouped(message: Message, previous: Message | undefined): boolean {
  return (
    !!previous &&
    previous.author === message.author &&
    !message.reply_to &&
    message.sent_at - previous.sent_at < 7 * 60_000 &&
    sameDay(message.sent_at, previous.sent_at)
  );
}

function DayDivider({ at }: { at: number }) {
  return (
    <div className="my-4 flex items-center gap-3 text-2xs font-medium text-muted-foreground">
      <div className="h-px flex-1 bg-border" />
      {formatDay(at)}
      <div className="h-px flex-1 bg-border" />
    </div>
  );
}

const MessageRow = memo(function MessageRow({
  message,
  previous,
  scope,
  channelKey,
  replied,
}: {
  message: Message;
  previous?: Message;
  scope: Scope | undefined;
  channelKey: ChannelKey;
  /** The message this replies to: null when it isn't loaded. */
  replied?: Message | null;
}) {
  const providerType = useStore((s) => s.types[channelKey.provider]);
  const compact = grouped(message, previous);
  const newDay = !previous || !sameDay(previous.sent_at, message.sent_at);
  const author = scope?.users[message.author];
  const name = userName(scope, message.author);
  const mine = message.author === scope?.me;
  const loadFunctions = useMessageFunctions(channelKey, message.id);
  const [menu, setMenu] = useState<{ rect: DOMRect; items: (MenuItem | "separator")[]; header?: ReactNode } | null>(null);
  const [dialog, setDialog] = useState<QFunction | null>(null);
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const context: FunctionContext = { Message: { channel: channelKey.channel, id: message.id } };

  const run = async (fn: QFunction, args: Record<string, never> | Parameters<typeof call>[2] = {}) => {
    try {
      await call(fn, context, args, channelKey.provider);
    } catch (error) {
      toast(describe(error), "error");
    }
  };

  const react = async (key: string, remove = false) => {
    try {
      const fns = await loadFunctions();
      const fn = fns.find((f) => actionOf(f) === (remove ? "RemoveReaction" : "AddReaction"));
      if (!fn) return toast("This message can't take reactions.", "error");
      await run(fn, { reaction: { Text: key } });
    } catch (error) {
      toast(describe(error), "error");
    }
  };

  const openReactions = async (rect: DOMRect) => {
    setMenu({
      rect,
      items: [],
      header: (
        <div className="grid grid-cols-8 gap-0.5 p-1">
          {EMOJI.map((e) => (
            <button
              key={e}
              onClick={() => {
                setMenu(null);
                void react(e);
              }}
              className="flex size-8 items-center justify-center rounded-md text-lg hover:bg-accent"
            >
              {e}
            </button>
          ))}
        </div>
      ),
    });
  };

  const openMore = async (rect: DOMRect) => {
    setBusy(true);
    let fns: QFunction[] = [];
    try {
      fns = await loadFunctions();
    } catch (error) {
      toast(describe(error), "error");
    } finally {
      setBusy(false);
    }
    const text = message.content.map(textOf).filter(Boolean).join("\n");
    const items: (MenuItem | "separator")[] = [
      { label: "Reply", icon: <CornerUpLeft />, onSelect: () => setReplyTo(channelKey, message.id) },
      { label: "Copy text", icon: <Copy />, disabled: !text, onSelect: () => void navigator.clipboard.writeText(text) },
    ];
    const rest: MenuItem[] = [];
    for (const fn of fns) {
      const action = actionOf(fn);
      if (action === "AddReaction" || action === "RemoveReaction" || action === "MarkRead") continue;
      if (action === "EditMessage") {
        rest.push({ label: fn.label || "Edit", icon: <Pencil />, onSelect: () => setEditing(true) });
      } else if (action === "DeleteMessage") {
        rest.push({
          label: fn.label || "Delete",
          icon: <Trash2 />,
          destructive: true,
          onSelect: () => {
            if (confirm("Delete this message? This can't be undone.")) void run(fn);
          },
        });
      } else if (action === "MarkUnread") {
        rest.push({ label: fn.label || "Mark unread", icon: <EyeOff />, onSelect: () => void run(fn) });
      } else if (fn.inputs.length === 0 && action) {
        rest.push({ label: fn.label, icon: <Sparkles />, onSelect: () => void run(fn) });
      } else if (action || "Lookup" in fn.kind) {
        rest.push({ label: `${fn.label}…`, icon: <Sparkles />, onSelect: () => setDialog(fn) });
      }
    }
    if (rest.length) items.push("separator", ...rest);
    setMenu({ rect, items });
  };

  return (
    <>
      {newDay ? <DayDivider at={message.sent_at} /> : null}
      <div
        className={cn(
          "group relative -mx-2 flex gap-3 rounded-lg px-2 hover:bg-accent/60",
          compact ? "py-0.5" : "mt-3 pt-1 pb-0.5",
          menu && "bg-accent/60",
        )}
      >
        <div className="w-9 shrink-0">
          {compact ? (
            <span className="invisible block pt-1 text-right text-[10px] text-muted-foreground group-hover:visible" title={formatLong(message.sent_at)}>
              {formatTime(message.sent_at)}
            </span>
          ) : (
            <Avatar name={name} seed={message.author} media={author?.avatar} size={36} className="mt-0.5" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          {replied !== undefined ? <ReplyQuote message={replied} scope={scope} providerType={providerType} /> : null}
          {compact ? null : (
            <div className="flex items-baseline gap-2">
              <span className={cn("truncate text-[13.5px] font-semibold", mine && "text-primary")}>{name}</span>
              <time className="shrink-0 text-2xs text-muted-foreground" title={formatLong(message.sent_at)}>
                {formatTime(message.sent_at)}
              </time>
            </div>
          )}
          {editing ? (
            <EditMessage
              message={message}
              onCancel={() => setEditing(false)}
              onSave={async (text) => {
                const fns = await loadFunctions();
                const fn = fns.find((f) => actionOf(f) === "EditMessage");
                if (!fn) return;
                await call(fn, context, { content: { Content: [{ Text: text }] } }, channelKey.provider);
                setEditing(false);
              }}
            />
          ) : (
            <MessageBody content={message.content} scope={scope} providerType={providerType} edited={!!message.edited_at} />
          )}
          {message.reactions.length ? (
            <div className="mt-1 flex flex-wrap gap-1">
              {message.reactions.map((r) => (
                <button
                  key={r.key}
                  onClick={() => void react(r.key, r.me)}
                  className={cn(
                    "flex h-6 items-center gap-1 rounded-full border px-2 text-xs transition-colors",
                    r.me ? "border-primary/40 bg-primary/12 text-primary" : "border-border bg-card hover:bg-accent",
                  )}
                >
                  <span className="text-[13px]">{r.key}</span>
                  <span className="font-medium tabular-nums">{r.count}</span>
                </button>
              ))}
            </div>
          ) : null}
        </div>
        <div
          className={cn(
            "absolute -top-3 right-2 hidden items-center rounded-lg border border-border bg-popover p-0.5 shadow-md group-hover:flex",
            (menu || busy) && "flex",
          )}
        >
          {QUICK.slice(0, 3).map((e) => (
            <button key={e} onClick={() => void react(e)} className="flex size-7 items-center justify-center rounded-md text-[15px] hover:bg-accent" aria-label={`React ${e}`}>
              {e}
            </button>
          ))}
          <ToolButton label="Add reaction" onClick={(r) => void openReactions(r)}>
            <SmilePlus />
          </ToolButton>
          <ToolButton label="Reply" onClick={() => setReplyTo(channelKey, message.id)}>
            <CornerUpLeft />
          </ToolButton>
          <ToolButton label="More" onClick={(r) => void openMore(r)}>
            {busy ? <Loader2 className="animate-spin" /> : <MoreHorizontal />}
          </ToolButton>
        </div>
      </div>
      {menu ? <Menu anchor={menu.rect} items={menu.items} header={menu.header} onClose={() => setMenu(null)} align="end" /> : null}
      {dialog ? <FunctionDialog provider={channelKey.provider} context={context} fn={dialog} onClose={() => setDialog(null)} /> : null}
    </>
  );
});

function ToolButton({ label, onClick, children }: { label: string; onClick: (rect: DOMRect) => void; children: ReactNode }) {
  return (
    <button
      aria-label={label}
      title={label}
      onClick={(e) => onClick(e.currentTarget.getBoundingClientRect())}
      className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground [&_svg]:size-4"
    >
      {children}
    </button>
  );
}

function EditMessage({ message, onSave, onCancel }: { message: Message; onSave: (text: string) => Promise<void>; onCancel: () => void }) {
  const [text, setText] = useState(message.content.map(textOf).filter(Boolean).join("\n"));
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try {
      await onSave(text);
    } catch (error) {
      toast(describe(error), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="mt-1">
      <Textarea
        autoFocus
        onFocus={(e) => e.currentTarget.setSelectionRange(e.currentTarget.value.length, e.currentTarget.value.length)}
        value={text}
        rows={Math.min(8, text.split("\n").length + 1)}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void save();
          }
        }}
      />
      <div className="mt-1.5 flex items-center gap-2 text-xs text-muted-foreground">
        <Button size="sm" variant="primary" loading={busy} onClick={() => void save()}>Save</Button>
        <Button size="sm" variant="ghost" onClick={onCancel}>Cancel</Button>
        <span>Enter to save · Esc to cancel</span>
      </div>
    </div>
  );
}

function ReplyQuote({ message, scope, providerType }: { message: Message | null; scope: Scope | undefined; providerType?: string }) {
  return (
    <div className="mb-0.5 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
      <CornerUpLeft className="size-3 shrink-0 -scale-x-100" />
      {message ? (
        <>
          <span className="shrink-0 font-medium text-foreground/80">{userName(scope, message.author)}</span>
          <span className="truncate">{plainText(message, providerType) || "Attachment"}</span>
        </>
      ) : (
        <span className="italic">Replying to an earlier message</span>
      )}
    </div>
  );
}

export function MessageBody({ content, scope, edited, previews, providerType }: { content: Content[]; scope: Scope | undefined; providerType?: string; edited?: boolean; previews?: Record<number, string> }) {
  const textParts: ReactNode[] = [];
  const blocks: ReactNode[] = [];
  content.forEach((part, i) => {
    if ("Text" in part) textParts.push(providerType === "discord" ? <DiscordText key={i} text={part.Text} scope={scope} /> : <Linkified key={i} text={part.Text} />);
    else if ("Formatted" in part) textParts.push(<Formatted key={i} value={part.Formatted} scope={scope} />);
    else if ("Custom" in part) blocks.push(<CustomPart key={i} kind={part.Custom.kind} data={part.Custom.data} />);
    else {
      const m = mediaOf(part);
      if (m) blocks.push(<MediaPart key={i} kind={m.kind} media={m.media} local={previews?.[i]} />);
    }
  });
  return (
    <>
      {textParts.length ? (
        <div className="text-[14px] leading-[1.45] break-words whitespace-pre-wrap text-foreground/95">
          {textParts.map((p, i) => (
            <div key={i}>
              {p}
              {edited && i === textParts.length - 1 ? <span className="ml-1 text-2xs text-muted-foreground">(edited)</span> : null}
            </div>
          ))}
        </div>
      ) : edited ? (
        <span className="text-2xs text-muted-foreground">(edited)</span>
      ) : null}
      {blocks.length ? <div className="mt-1.5 flex flex-col items-start gap-1.5">{blocks}</div> : null}
    </>
  );
}

/** Platform content qmsg has no type for, such as an embed. */
function CustomPart({ kind, data }: { kind: string; data: number[] }) {
  const json = customJson(data) as Record<string, unknown> | null;
  const title = typeof json?.title === "string" ? json.title : null;
  const description = typeof json?.description === "string" ? json.description : null;
  const url = typeof json?.url === "string" ? json.url : null;
  const color = typeof json?.color === "number" ? `#${json.color.toString(16).padStart(6, "0")}` : undefined;
  if (title || description) {
    return (
      <div className="max-w-md rounded-lg border border-border border-l-4 bg-card px-3 py-2" style={{ borderLeftColor: color }}>
        {title ? (
          <div className="text-[13px] font-semibold">{safeUrl(url) ? <ExternalLink href={safeUrl(url)!}>{title}</ExternalLink> : title}</div>
        ) : null}
        {description ? <div className="mt-0.5 text-[13px] whitespace-pre-wrap text-muted-foreground">{description}</div> : null}
      </div>
    );
  }
  return (
    <span className="inline-flex items-center gap-1.5 rounded-md bg-muted px-2 py-1 text-xs text-muted-foreground">
      <Sparkles className="size-3" /> {customLabel(kind)}
    </span>
  );
}

function PendingRow({ pending, channelKey, scope }: { pending: PendingSend; channelKey: ChannelKey; scope: Scope | undefined }) {
  const providerType = useStore((s) => s.types[channelKey.provider]);
  const state = pending.state;
  return (
    <div className="group -mx-2 mt-1 flex gap-3 rounded-lg px-2 py-0.5">
      <div className="w-9 shrink-0" />
      <div className={cn("min-w-0 flex-1", state.kind === "sending" && "opacity-60")}>
        <MessageBody content={pending.content} scope={scope} providerType={providerType} previews={pending.previews} />
        {state.kind === "sending" ? (
          <div className="mt-0.5 flex items-center gap-1 text-2xs text-muted-foreground">
            <Loader2 className="size-3 animate-spin" /> Sending…
          </div>
        ) : (
          <div className={cn("mt-1 flex flex-wrap items-center gap-2 text-xs", state.kind === "failed" ? "text-destructive" : "text-warning")}>
            <AlertTriangle className="size-3.5" />
            <span>{state.kind === "failed" ? `Not sent: ${state.message}` : `Maybe sent: ${state.message}. Check before sending again.`}</span>
            <button onClick={() => retry(channelKey, pending)} className="flex items-center gap-1 rounded px-1.5 py-0.5 font-medium hover:bg-accent">
              <RotateCcw className="size-3" /> {state.kind === "failed" ? "Retry" : "Send again"}
            </button>
            <button onClick={() => discard(channelKey, pending)} className="flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-accent">
              <X className="size-3" /> Discard
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
