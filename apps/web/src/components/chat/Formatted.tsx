import { Fragment, useState, type ReactNode } from "react";
import { tag, type FormattedText, type Mention, type Scope, type Style } from "~/api/types";
import { cn } from "~/lib/cn";
import { safeUrl } from "~/store/media";
import { userName } from "~/store/directory";

const URL_RE = /\bhttps?:\/\/[^\s<>"'`]+[^\s<>"'`.,;:!?)\]}]/g;

/** Plain text with web links made clickable. */
export function Linkified({ text }: { text: string }) {
  const parts: ReactNode[] = [];
  let last = 0;
  for (const match of text.matchAll(URL_RE)) {
    const url = safeUrl(match[0]);
    if (!url) continue;
    parts.push(text.slice(last, match.index));
    parts.push(<ExternalLink key={match.index} href={url}>{match[0]}</ExternalLink>);
    last = match.index! + match[0].length;
  }
  parts.push(text.slice(last));
  return <>{parts}</>;
}

export function ExternalLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a href={href} target="_blank" rel="noopener noreferrer nofollow" className="text-primary underline-offset-2 hover:underline">
      {children}
    </a>
  );
}

function Spoiler({ children }: { children: ReactNode }) {
  const [shown, setShown] = useState(false);
  return (
    <span
      onClick={() => setShown(true)}
      className={cn("rounded px-0.5 transition-colors", shown ? "bg-accent" : "cursor-pointer bg-foreground text-transparent select-none [&_*]:invisible")}
    >
      {children}
    </span>
  );
}

function mentionLabel(mention: Mention, scope: Scope | undefined): string {
  const [kind, id] = tag(mention) as [string, string];
  switch (kind) {
    case "User":
      return `@${userName(scope, id)}`;
    case "Channel": {
      const channel = scope?.channels[id];
      return `#${channel?.name ?? "channel"}`;
    }
    case "Role":
      return "@role";
    default:
      return "@everyone";
  }
}

/** Maps UTF-8 byte offsets to UTF-16 string indices. */
function byteIndex(text: string): (byte: number) => number {
  const offsets: number[] = [];
  let byte = 0;
  for (let i = 0; i < text.length; i++) {
    const code = text.codePointAt(i)!;
    const width = code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
    for (let b = 0; b < width; b++) offsets[byte + b] = i;
    byte += width;
    if (code >= 0x10000) i++;
  }
  return (b) => (b >= byte ? text.length : (offsets[b] ?? text.length));
}

function wrap(style: Style, node: ReactNode, key: number, scope: Scope | undefined): ReactNode {
  const [name, value] = tag(style);
  switch (name) {
    case "Bold":
      return <strong key={key} className="font-semibold">{node}</strong>;
    case "Italic":
      return <em key={key}>{node}</em>;
    case "Underline":
      return <u key={key}>{node}</u>;
    case "Strikethrough":
      return <s key={key}>{node}</s>;
    case "Code":
      return <code key={key} className="rounded bg-accent px-1 py-px font-mono text-[0.88em]">{node}</code>;
    case "CodeBlock":
      return (
        <code key={key} className="my-1 block overflow-x-auto rounded-lg border border-border bg-muted px-3 py-2 font-mono text-[12.5px] leading-relaxed whitespace-pre">
          {node}
        </code>
      );
    case "Quote":
      return <span key={key} className="my-0.5 block border-l-2 border-muted-foreground/40 pl-3 text-muted-foreground">{node}</span>;
    case "Spoiler":
      return <Spoiler key={key}>{node}</Spoiler>;
    case "Link": {
      const url = safeUrl(value as string);
      return url ? <ExternalLink key={key} href={url}>{node}</ExternalLink> : <Fragment key={key}>{node}</Fragment>;
    }
    case "Mention":
      return (
        <span key={key} className="rounded bg-mention px-0.5 font-medium text-primary" title={mentionLabel(value as Mention, scope)}>
          {node}
        </span>
      );
    default:
      return <Fragment key={key}>{node}</Fragment>;
  }
}

/** Text with styled ranges. Ranges are in UTF-8 bytes; the text reads well without them. */
export function Formatted({ value, scope }: { value: FormattedText; scope?: Scope }) {
  const { text, spans } = value;
  if (!spans.length) return <Linkified text={text} />;
  const at = byteIndex(text);
  const ranges = spans
    .map((s) => ({ start: at(s.start), end: at(s.end), style: s.style }))
    .filter((s) => s.end > s.start);
  const cuts = [...new Set([0, text.length, ...ranges.flatMap((r) => [r.start, r.end])])].sort((a, b) => a - b);
  const nodes: ReactNode[] = [];
  for (let i = 0; i < cuts.length - 1; i++) {
    const start = cuts[i]!;
    const end = cuts[i + 1]!;
    const active = ranges.filter((r) => r.start <= start && r.end >= end);
    const hasLink = active.some((r) => typeof r.style === "object" && "Link" in r.style);
    let node: ReactNode = hasLink ? text.slice(start, end) : <Linkified text={text.slice(start, end)} />;
    active.forEach((r, j) => (node = wrap(r.style, node, j, scope)));
    nodes.push(<Fragment key={start}>{node}</Fragment>);
  }
  return <>{nodes}</>;
}
