import { ArrowUp, FileIcon, Paperclip, X } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Channel, Message, Scope, User } from "~/api/types";
import { cn } from "~/lib/cn";
import { formatBytes, plainText } from "~/lib/content";
import { channelTitle, isWritable, keyString, userName, type ChannelKey } from "~/store/directory";
import { send, setDraft, setReplyTo, useStore } from "~/store/store";
import { toast } from "~/store/ui";
import { Avatar } from "../Avatar";

/** How a network writes a mention, or null when the app doesn't know. */
function mentionStyle(type: string | undefined): ((u: User) => { shown: string; sent: string }) | null {
  if (type === "discord") return (u) => ({ shown: `@${u.name}`, sent: `<@${u.id}>` });
  if (type === "echo") return (u) => ({ shown: `@${u.name}`, sent: `@${u.name}` });
  return null;
}

/** Converts names picked in the composer to their network mention tokens. */
export function encodePicked(text: string, replacements: Record<string, string>): string {
  const names = Object.keys(replacements)
    .filter(Boolean)
    .sort((a, b) => b.length - a.length);
  if (!names.length) return text;
  const escaped = names.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const re = new RegExp(`(?<![\\p{L}\\p{N}_])(?:${escaped.join("|")})(?![\\p{L}\\p{N}_])`, "gu");
  return text.replace(re, (shown) => replacements[shown] ?? shown);
}

/** Whether a file fits one of the channel's rules for its kind. */
function accepts(channel: Channel, file: File): boolean {
  const kind = file.type.split("/")[0];
  const wanted = kind === "image" ? "Image" : kind === "video" ? "Video" : kind === "audio" ? "Audio" : "File";
  return channel.accepted_content.some((r) => {
    if (r.kind !== wanted && r.kind !== "File") return false;
    if (r.max_size != null && file.size > r.max_size) return false;
    if (!r.mime_types.length) return true;
    return r.mime_types.some((p) => (p.endsWith("/*") ? file.type.startsWith(p.slice(0, -1)) : p === file.type));
  });
}

function measure(text: string, unit: string): number {
  if (unit === "Utf16") return text.length;
  if (unit === "Bytes") return new TextEncoder().encode(text).length;
  return [...text].length;
}

export function Composer({
  channelKey,
  channel,
  scope,
  messages,
  providerType,
}: {
  channelKey: ChannelKey;
  channel: Channel;
  scope: Scope | undefined;
  messages: Message[];
  providerType?: string;
}) {
  const k = keyString(channelKey);
  const draft = useStore((s) => s.drafts[k] ?? "");
  const replyId = useStore((s) => s.replyTo[k] ?? null);
  const replied = replyId ? messages.find((m) => m.id === replyId) : undefined;
  const [files, setFiles] = useState<File[]>([]);
  const [mentions, setMentions] = useState<Record<string, string>>({});
  const [caret, setCaret] = useState(0);
  const [active, setActive] = useState(0);
  const [dragging, setDragging] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const writable = isWritable(channel);
  const textRule = channel.accepted_content.find((r) => r.kind === "Text" || r.kind === "Formatted");
  const style = mentionStyle(providerType);
  const title = channelTitle(scope, channel);

  useEffect(() => {
    setFiles([]);
    setMentions({});
    ref.current?.focus();
  }, [k]);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "0px";
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`;
  }, [draft]);

  useEffect(() => {
    if (replyId) ref.current?.focus();
  }, [replyId]);

  // The word being typed after an "@", if any.
  const query = useMemo(() => {
    if (!style) return null;
    const before = draft.slice(0, caret);
    const match = /(?:^|\s)@([\p{L}\p{N}_.-]*)$/u.exec(before);
    return match ? match[1]! : null;
  }, [draft, caret, style]);

  const suggestions = useMemo(() => {
    if (query === null || !scope) return [];
    const recent = [...new Set(messages.slice(-50).map((m) => m.author))];
    const ids = channel.members ?? (query ? Object.keys(scope.users) : recent);
    const q = query.toLowerCase();
    return ids
      .map((id) => scope.users[id])
      .filter((u): u is User => !!u && u.id !== scope.me && !!u.name && u.name !== u.id)
      .filter((u) => !q || u.name.toLowerCase().includes(q) || u.username?.toLowerCase().includes(q))
      .sort((a, b) => a.name.localeCompare(b.name))
      .slice(0, 6);
  }, [query, scope, channel.members, messages]);

  const pick = (user: User) => {
    if (!style) return;
    const { shown, sent } = style(user);
    const before = draft.slice(0, caret).replace(/@[\p{L}\p{N}_.-]*$/u, `${shown} `);
    const text = before + draft.slice(caret);
    setDraft(channelKey, text);
    setMentions((m) => ({ ...m, [shown]: sent }));
    setActive(0);
    requestAnimationFrame(() => {
      ref.current?.setSelectionRange(before.length, before.length);
      setCaret(before.length);
    });
  };

  const addFiles = (list: FileList | File[]) => {
    const added: File[] = [];
    for (const file of Array.from(list)) {
      if (!accepts(channel, file)) toast(`${title} doesn't take ${file.name} (${file.type || "unknown type"}, ${formatBytes(file.size)}).`, "error");
      else added.push(file);
    }
    const max = channel.limits.max_attachments;
    setFiles((f) => {
      const next = [...f, ...added];
      if (max != null && next.length > max) {
        toast(`At most ${max} attachments per message.`, "error");
        return next.slice(0, max);
      }
      return next;
    });
  };

  const length = textRule ? measure(draft.trim(), textRule.text_unit) : 0;
  const max = textRule?.max_size ?? null;
  const tooLong = max != null && length > max;
  const canSend = writable && !tooLong && (!!draft.trim() || files.length > 0);

  const submit = () => {
    if (!canSend) return;
    send(channelKey, encodePicked(draft, mentions), files, replyId);
    setDraft(channelKey, "");
    setReplyTo(channelKey, null);
    setFiles([]);
    setMentions({});
  };

  if (!writable) {
    return (
      <div className="mx-auto w-full max-w-4xl px-3 pb-4 sm:px-5">
        <div className="rounded-2xl border border-border bg-muted px-4 py-3 text-center text-[13px] text-muted-foreground">
          You can't send messages here.
        </div>
      </div>
    );
  }

  return (
    <div className="mx-auto w-full max-w-4xl px-3 pb-4 sm:px-5">
      <div
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes("Files")) {
            e.preventDefault();
            setDragging(true);
          }
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          addFiles(e.dataTransfer.files);
        }}
        className={cn(
          "relative rounded-2xl border bg-card shadow-composer transition-colors focus-within:border-ring/40",
          dragging ? "border-primary border-dashed bg-primary/5" : "border-border",
        )}
      >
        {suggestions.length ? (
          <div className="absolute inset-x-2 bottom-full mb-2 rounded-xl border border-border bg-popover p-1 shadow-xl animate-pop-in">
            {suggestions.map((u, i) => (
              <button
                key={u.id}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(u)}
                className={cn("flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[13px]", i === active ? "bg-accent" : "hover:bg-accent")}
              >
                <Avatar name={u.name} seed={u.id} media={u.avatar} size={22} />
                <span className="font-medium">{u.name}</span>
                {u.username ? <span className="text-xs text-muted-foreground">@{u.username}</span> : null}
              </button>
            ))}
          </div>
        ) : null}
        {replyId ? (
          <div className="flex items-center gap-2 border-b border-border px-3.5 py-2 text-xs text-muted-foreground">
            <span className="shrink-0">Replying to</span>
            <span className="shrink-0 font-medium text-foreground">{replied ? userName(scope, replied.author) : "message"}</span>
            <span className="min-w-0 flex-1 truncate">{replied ? plainText(replied, providerType) : ""}</span>
            <button onClick={() => setReplyTo(channelKey, null)} className="rounded p-0.5 hover:bg-accent" aria-label="Cancel reply">
              <X className="size-3.5" />
            </button>
          </div>
        ) : null}
        {files.length ? (
          <div className="flex flex-wrap gap-2 px-3 pt-3">
            {files.map((file, i) => (
              <FileChip key={`${file.name}-${i}`} file={file} onRemove={() => setFiles((f) => f.filter((_, j) => j !== i))} />
            ))}
          </div>
        ) : null}
        <textarea
          ref={ref}
          value={draft}
          rows={1}
          placeholder={`Message ${channel.kind === "Direct" ? "" : "#"}${title}`}
          onChange={(e) => {
            setDraft(channelKey, e.target.value);
            setCaret(e.target.selectionStart);
          }}
          onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
          onPaste={(e) => {
            if (e.clipboardData.files.length) {
              e.preventDefault();
              addFiles(e.clipboardData.files);
            }
          }}
          onKeyDown={(e) => {
            if (suggestions.length) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                setActive((a) => (a + (e.key === "ArrowDown" ? 1 : suggestions.length - 1)) % suggestions.length);
                return;
              }
              if (e.key === "Enter" || e.key === "Tab") {
                e.preventDefault();
                pick(suggestions[active] ?? suggestions[0]!);
                return;
              }
            }
            if (e.key === "Escape" && replyId) setReplyTo(channelKey, null);
            if (e.key === "Enter" && !e.shiftKey && !e.altKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            }
          }}
          className="block max-h-60 w-full resize-none bg-transparent px-4 pt-3.5 pb-1 text-[14px] leading-relaxed outline-none placeholder:text-muted-foreground/60"
        />
        <div className="flex items-center gap-1 px-2 pb-2">
          <label className="flex size-8 cursor-pointer items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground" title="Attach files">
            <Paperclip className="size-4" />
            <input
              type="file"
              multiple
              hidden
              onChange={(e) => {
                if (e.target.files) addFiles(e.target.files);
                e.target.value = "";
              }}
            />
          </label>
          <div className="flex-1" />
          {max != null && length > max * 0.8 ? (
            <span className={cn("text-2xs tabular-nums", tooLong ? "text-destructive" : "text-muted-foreground")}>
              {length}/{max}
            </span>
          ) : null}
          <button
            onClick={submit}
            disabled={!canSend}
            aria-label="Send"
            className="flex size-8 items-center justify-center rounded-full bg-primary text-primary-foreground transition-opacity disabled:opacity-30"
          >
            <ArrowUp className="size-4" />
          </button>
        </div>
      </div>
      <div className="mt-1.5 hidden px-2 text-2xs text-muted-foreground/70 sm:block">
        Enter to send · Shift+Enter for a new line · paste or drop files to attach
      </div>
    </div>
  );
}

function FileChip({ file, onRemove }: { file: File; onRemove: () => void }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!file.type.startsWith("image/")) return;
    const u = URL.createObjectURL(file);
    setUrl(u);
    return () => URL.revokeObjectURL(u);
  }, [file]);
  return (
    <div className="group relative flex h-14 items-center gap-2 rounded-xl border border-border bg-background pr-3 pl-1">
      {url ? (
        <img src={url} alt="" className="size-12 rounded-lg object-cover" />
      ) : (
        <span className="flex size-12 items-center justify-center rounded-lg bg-accent text-muted-foreground">
          <FileIcon className="size-5" />
        </span>
      )}
      <span className="max-w-40 min-w-0">
        <span className="block truncate text-xs font-medium">{file.name}</span>
        <span className="block text-2xs text-muted-foreground">{formatBytes(file.size)}</span>
      </span>
      <button
        onClick={onRemove}
        aria-label={`Remove ${file.name}`}
        className="absolute -top-1.5 -right-1.5 flex size-5 items-center justify-center rounded-full border border-border bg-popover text-muted-foreground opacity-0 shadow group-hover:opacity-100"
      >
        <X className="size-3" />
      </button>
    </div>
  );
}
