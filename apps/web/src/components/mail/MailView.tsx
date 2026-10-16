import DOMPurify from "dompurify";
import {
  ArrowLeft,
  ImageOff,
  Mail,
  MailOpen,
  Menu as MenuIcon,
  PenSquare,
  RefreshCw,
  Reply,
  Star,
  Trash2,
  FolderInput,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { describe } from "~/api/errors";
import { actionOf, type Channel, type Message, type QFunction, } from "~/api/types";
import { cn } from "~/lib/cn";
import { addressName, formatLong, formatStamp, htmlText, mediaOf, textOf } from "~/lib/content";
import { emptyConversation, mailNeedsBody } from "~/store/conversation";
import { keyString, mailboxName, mailboxRole, MailboxRole, type ChannelKey } from "~/store/directory";
import { mediaUrl } from "~/store/media";
import { call, functions, loadLatest, loadMessage, loadOlder, useStore } from "~/store/store";
import { toast, useUi } from "~/store/ui";
import { Avatar } from "../Avatar";
import { MediaPart } from "../chat/MediaPart";
import { FunctionDialog } from "../FunctionDialog";
import { Button, EmptyState, Spinner } from "../ui";

export function MailView({ channelKey, channel }: { channelKey: ChannelKey; channel: Channel }) {
  const k = keyString(channelKey);
  const conversation = useStore((s) => s.conversations[k]) ?? emptyConversation();
  const [selected, setSelected] = useState<string | null>(null);
  const [compose, setCompose] = useState<QFunction | null>(null);
  const messages = useMemo(() => [...conversation.messages].reverse(), [conversation.messages]);
  const message = selected ? conversation.messages.find((m) => m.id === selected) : undefined;
  const showRecipients = [MailboxRole.Sent, MailboxRole.Drafts].includes(mailboxRole(channel));
  const sentinel = useRef<HTMLDivElement>(null);

  useEffect(() => setSelected(null), [k]);
  useEffect(() => {
    const el = sentinel.current;
    if (!el) return;
    const observer = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && void loadOlder(channelKey), { rootMargin: "400px" });
    observer.observe(el);
    return () => observer.disconnect();
  }, [channelKey, conversation.olderCursor]);

  const startCompose = async () => {
    try {
      const fns = await functions({ Channel: channelKey.channel }, channelKey.provider);
      const fn = fns.find((f) => f.id === "compose") ?? fns.find((f) => actionOf(f) === "SendMessage");
      if (fn) setCompose(fn);
      else toast("This account can't send mail.", "error");
    } catch (error) {
      toast(describe(error), "error");
    }
  };

  return (
    <div className="flex h-full min-w-0">
      <div className={cn("flex w-full min-w-0 flex-col border-r border-border md:w-[22rem] md:shrink-0 lg:w-[26rem]", message && "hidden md:flex")}>
        <header className="flex h-13 shrink-0 items-center gap-2 border-b border-border px-3">
          <Button variant="ghost" size="icon-sm" className="md:hidden" onClick={() => useUi.setState({ sidebarOpen: true })} aria-label="Open sidebar">
            <MenuIcon />
          </Button>
          <h1 className="truncate text-[14px] font-semibold">{mailboxName(channel)}</h1>
          <span className="text-2xs text-muted-foreground">{channelKey.provider}</span>
          <div className="flex-1" />
          <Button variant="ghost" size="icon-sm" title="Refresh" aria-label="Refresh" onClick={() => void loadLatest(channelKey)}>
            <RefreshCw className={cn(conversation.isLoading && "animate-spin")} />
          </Button>
          <Button variant="ghost" size="icon-sm" title="New email" aria-label="New email" onClick={() => void startCompose()}>
            <PenSquare />
          </Button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {!conversation.loaded ? (
            <div className="flex justify-center p-8">{conversation.loadError ? <span className="text-xs text-destructive">{conversation.loadError}</span> : <Spinner />}</div>
          ) : messages.length === 0 ? (
            <EmptyState icon={<Mail />} title="No mail here" />
          ) : null}
          {messages.map((m) => (
            <MailRow key={m.id} message={m} active={m.id === selected} showRecipients={showRecipients} onClick={() => setSelected(m.id)} />
          ))}
          <div ref={sentinel} className="flex justify-center p-3">
            {conversation.isLoadingOlder ? <Spinner /> : null}
          </div>
        </div>
      </div>
      <div className={cn("min-w-0 flex-1", !message && "hidden md:block")}>
        {message ? (
          <Reader key={message.id} message={message} channelKey={channelKey} onBack={() => setSelected(null)} />
        ) : (
          <EmptyState icon={<MailOpen />} title="No message selected">
            Choose an email to read it.
          </EmptyState>
        )}
      </div>
      {compose ? (
        <FunctionDialog provider={channelKey.provider} context={{ Channel: channelKey.channel }} fn={compose} onClose={() => setCompose(null)} />
      ) : null}
    </div>
  );
}

function MailRow({ message, active, showRecipients, onClick }: { message: Message; active: boolean; showRecipients: boolean; onClick: () => void }) {
  const email = message.email;
  const people = (showRecipients ? email?.to : email?.from) ?? [];
  const who = people.map(addressName).join(", ") || "(unknown)";
  const unread = email ? !email.read : false;
  const preview = message.content.map(textOf).filter(Boolean).join(" ").replace(/\s+/g, " ").trim() || (email ? htmlText(email) : "");
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full gap-3 border-b border-border px-4 py-3 text-left transition-colors",
        active ? "bg-accent" : "hover:bg-accent/60",
      )}
    >
      <span className={cn("mt-1.5 size-2 shrink-0 rounded-full", unread ? "bg-primary" : "bg-transparent")} />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className={cn("truncate text-[13px]", unread ? "font-semibold" : "font-medium text-foreground/90")}>
            {showRecipients ? `To: ${who}` : who}
          </span>
          {email?.starred ? <Star className="size-3 shrink-0 fill-warning text-warning" /> : null}
          <span className="ml-auto shrink-0 text-2xs text-muted-foreground">{formatStamp(message.sent_at)}</span>
        </span>
        <span className={cn("block truncate text-[13px]", unread ? "text-foreground" : "text-foreground/80")}>{email?.subject || "(no subject)"}</span>
        <span className="line-clamp-2 text-xs text-muted-foreground">{preview}</span>
      </span>
    </button>
  );
}

const ICONS: Record<string, typeof Star> = {
  reply: Reply,
  star: Star,
  unstar: Star,
  mark_read: MailOpen,
  mark_unread: Mail,
  trash: Trash2,
  move: FolderInput,
};

function Reader({ message, channelKey, onBack }: { message: Message; channelKey: ChannelKey; onBack: () => void }) {
  const email = message.email;
  const [fns, setFns] = useState<QFunction[]>([]);
  const [dialog, setDialog] = useState<QFunction | null>(null);
  const [loading, setLoading] = useState(mailNeedsBody(message));
  const context = { Message: { channel: channelKey.channel, id: message.id } } as const;

  useEffect(() => {
    let live = true;
    if (mailNeedsBody(message)) {
      loadMessage(channelKey, message.id)
        .catch((e) => toast(describe(e), "error"))
        .finally(() => live && setLoading(false));
    }
    functions(context, channelKey.provider)
      .then((list) => {
        if (!live) return;
        setFns(list);
        const read = list.find((f) => f.id === "mark_read");
        if (read && email && !email.read) void call(read, context, {}, channelKey.provider).catch(() => {});
      })
      .catch(() => {});
    return () => {
      live = false;
    };
    // Once per opened message.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [message.id]);

  const run = async (fn: QFunction) => {
    if (fn.inputs.some((i) => i.required)) return setDialog(fn);
    try {
      await call(fn, context, {}, channelKey.provider);
      if (actionOf(fn) === "DeleteMessage") onBack();
    } catch (error) {
      toast(describe(error), "error");
    }
  };

  const visible = fns.filter((f) => {
    if (!("Action" in f.kind)) return false;
    if (f.id === "star" && email?.starred) return false;
    if (f.id === "unstar" && !email?.starred) return false;
    if (f.id === "mark_read" && email?.read) return false;
    if (f.id === "mark_unread" && !email?.read) return false;
    return true;
  });
  const from = email?.from[0];
  const attachments = useMemo(() => {
    const inline = new Set(Object.keys(email?.content_ids ?? {}).map(Number));
    return message.content
      .map((c, i) => ({ part: mediaOf(c), i }))
      .filter((x) => x.part && !(inline.has(x.i) && email?.html?.toLowerCase().includes(`cid:${email.content_ids[String(x.i)]?.toLowerCase()}`)));
  }, [message, email]);

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-13 shrink-0 items-center gap-1 border-b border-border px-3">
        <Button variant="ghost" size="icon-sm" className="md:hidden" onClick={onBack} aria-label="Back">
          <ArrowLeft />
        </Button>
        {visible.map((fn) => {
          const Icon = ICONS[fn.id] ?? (actionOf(fn) === "DeleteMessage" ? Trash2 : null);
          return (
            <Button key={fn.id} variant="ghost" size="sm" onClick={() => void run(fn)} title={fn.label} className="capitalize">
              {Icon ? <Icon className={cn(fn.id === "unstar" && "fill-warning text-warning")} /> : null}
              <span className="hidden xl:inline">{fn.label}</span>
            </Button>
          );
        })}
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-6 py-6">
          <h1 className="text-xl font-semibold">{email?.subject || "(no subject)"}</h1>
          <div className="mt-4 flex items-start gap-3">
            <Avatar name={from ? addressName(from) : "?"} seed={from?.address} size={40} />
            <div className="min-w-0 flex-1 text-[13px]">
              <div className="flex items-baseline gap-2">
                <span className="font-semibold">{from ? addressName(from) : "Unknown sender"}</span>
                {from?.name ? <span className="truncate text-muted-foreground">&lt;{from.address}&gt;</span> : null}
              </div>
              <div className="text-muted-foreground">
                To {(email?.to ?? []).map(addressName).join(", ") || "—"}
                {email?.cc.length ? ` · Cc ${email.cc.map(addressName).join(", ")}` : ""}
              </div>
            </div>
            <time className="shrink-0 text-xs text-muted-foreground">{formatLong(message.sent_at)}</time>
          </div>
          <div className="mt-6">
            {loading ? (
              <div className="flex justify-center py-10">
                <Spinner />
              </div>
            ) : email?.html ? (
              <HtmlBody message={message} />
            ) : (
              <div className="text-[14px] leading-relaxed break-words whitespace-pre-wrap">
                {message.content.map(textOf).filter(Boolean).join("\n") || <span className="text-muted-foreground">(empty)</span>}
              </div>
            )}
          </div>
          {attachments.length ? (
            <div className="mt-6 flex flex-wrap gap-2 border-t border-border pt-4">
              {attachments.map(({ part, i }) => (
                <MediaPart key={i} kind={part!.kind === "Image" ? "Image" : "File"} media={part!.media} />
              ))}
            </div>
          ) : null}
          <div className="mt-8">
            {fns.find((f) => f.id === "reply") ? (
              <Button variant="outline" onClick={() => setDialog(fns.find((f) => f.id === "reply")!)}>
                <Reply /> Reply
              </Button>
            ) : null}
          </div>
        </div>
      </div>
      {dialog ? <FunctionDialog provider={channelKey.provider} context={context} fn={dialog} onClose={() => setDialog(null)} /> : null}
    </div>
  );
}

/**
 * Mail HTML is untrusted. It is sanitized, shown in a sandboxed frame without
 * scripts, and remote images stay blocked until asked for. Inline `cid:`
 * images resolve to the message's own parts.
 */
function HtmlBody({ message }: { message: Message }) {
  const email = message.email!;
  const [remote, setRemote] = useState(false);
  const [cids, setCids] = useState<Record<string, string>>({});
  const frame = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(200);

  useEffect(() => {
    let live = true;
    const entries = Object.entries(email.content_ids);
    Promise.all(
      entries.map(async ([index, cid]) => {
        const part = message.content[Number(index)];
        const media = part ? mediaOf(part) : null;
        if (!media || media.kind !== "Image") return null;
        return [cid.toLowerCase(), await mediaUrl(media.media).catch(() => "")] as const;
      }),
    ).then((pairs) => live && setCids(Object.fromEntries(pairs.filter((p): p is readonly [string, string] => !!p && !!p[1]))));
    return () => {
      live = false;
    };
  }, [message, email.content_ids]);

  const { html, blocked } = useMemo(() => {
    let blocked = 0;
    const clean = DOMPurify.sanitize(email.html ?? "", {
      WHOLE_DOCUMENT: true,
      FORBID_TAGS: ["script", "iframe", "object", "embed", "form", "input", "button", "textarea", "select", "meta", "link", "base"],
      FORBID_ATTR: ["srcset"],
      RETURN_DOM: true,
    }) as unknown as HTMLElement;
    for (const img of clean.querySelectorAll("img")) {
      const src = img.getAttribute("src") ?? "";
      if (src.toLowerCase().startsWith("cid:")) {
        const url = cids[src.slice(4).toLowerCase()];
        if (url) img.setAttribute("src", url);
        else img.removeAttribute("src");
      } else if (/^https?:/i.test(src) && !remote) {
        img.removeAttribute("src");
        img.setAttribute("alt", img.getAttribute("alt") || "");
        blocked++;
      } else if (!/^(https?:|data:image\/|blob:)/i.test(src)) {
        img.removeAttribute("src");
      }
    }
    for (const a of clean.querySelectorAll("a")) {
      a.setAttribute("target", "_blank");
      a.setAttribute("rel", "noopener noreferrer");
    }
    // CSP also blocks CSS backgrounds, SVG references, media and stylesheet
    // imports. Removing only <img src> cannot enforce the privacy setting.
    const policy = `default-src 'none'; img-src data: blob:${remote ? " https: http:" : ""}; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'`;
    const style = `<style>html,body{margin:0;padding:0;font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,sans-serif;color:#222;background:#fff;word-wrap:break-word}img{max-width:100%;height:auto}table{max-width:100%}</style>`;
    // A stylesheet may contain hidden requests; offer the same opt-in even
    // when there are no ordinary remote <img> elements to count.
    if (!remote && clean.querySelector("style, [style], [background], image")) blocked++;
    return { html: `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}">${style}</head><body>${clean.innerHTML}</body></html>`, blocked };
  }, [email.html, cids, remote]);

  return (
    <div>
      {blocked ? (
        <div className="mb-3 flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          <ImageOff className="size-3.5" />
          <span className="flex-1">Remote images are blocked to protect your privacy.</span>
          <Button size="sm" variant="outline" onClick={() => setRemote(true)}>Load images</Button>
        </div>
      ) : null}
      <iframe
        ref={frame}
        title="Email"
        // Same-origin access lets the frame size itself; scripts stay off.
        sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
        srcDoc={html}
        onLoad={() => {
          const doc = frame.current?.contentDocument;
          if (doc) setHeight(Math.max(120, doc.documentElement.scrollHeight));
        }}
        style={{ height }}
        className="w-full rounded-lg border border-border bg-white"
      />
    </div>
  );
}
