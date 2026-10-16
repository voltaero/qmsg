import { Fragment, type ReactNode } from "react";
import { parseDiscord, type DiscordNode } from "~/lib/discord";
import type { Scope } from "~/api/types";
import { safeUrl } from "~/store/media";
import { ExternalLink, Spoiler } from "./Formatted";

function render(nodes: DiscordNode[], scope?: Scope): ReactNode {
  return nodes.map((node, key) => {
    const children = Array.isArray(node.content) ? render(node.content, scope) : node.content;
    switch (node.type) {
      case "strong": return <strong key={key} className="font-semibold">{children}</strong>;
      case "em": return <em key={key}>{children}</em>;
      case "underline": return <u key={key}>{children}</u>;
      case "strikethrough": return <s key={key}>{children}</s>;
      case "spoiler": return <Spoiler key={key}>{children}</Spoiler>;
      case "inlineCode": return <code key={key} className="rounded bg-accent px-1 py-px font-mono text-[0.88em]">{node.content}</code>;
      case "codeBlock": return <pre key={key} className="my-1 overflow-x-auto rounded-lg border border-border bg-muted px-3 py-2 font-mono text-[12.5px] leading-relaxed whitespace-pre"><code>{node.content}</code></pre>;
      case "blockQuote": return <blockquote key={key} className="my-0.5 border-l-2 border-muted-foreground/40 pl-3 text-muted-foreground">{children}</blockquote>;
      case "heading": return <div key={key} role="heading" aria-level={node.level} className="my-1 text-lg font-semibold">{children}</div>;
      case "subtext": return <small key={key} className="block text-muted-foreground">{children}</small>;
      case "list": {
        const items = node.items.map((item: DiscordNode[], i: number) => <li key={i}>{render(item, scope)}</li>);
        return node.ordered
          ? <ol key={key} start={node.start} className="my-1 list-decimal pl-6 whitespace-normal">{items}</ol>
          : <ul key={key} className="my-1 list-disc pl-6 whitespace-normal">{items}</ul>;
      }
      case "link": case "url": case "autolink": {
        const href = safeUrl(node.target);
        return href ? <ExternalLink key={key} href={href}>{children}</ExternalLink> : <Fragment key={key}>{children}</Fragment>;
      }
      case "user": return <span key={key} className="rounded bg-mention px-0.5 text-primary">@{scope?.users[node.id]?.name ?? node.id}</span>;
      case "channel": return <span key={key} className="rounded bg-mention px-0.5 text-primary">#{scope?.channels[node.id]?.name ?? node.id}</span>;
      case "role": return <span key={key} className="rounded bg-mention px-0.5 text-primary">{`<@&${node.id}>`}</span>;
      case "everyone": case "here": return <span key={key} className="rounded bg-mention px-0.5 text-primary">@{node.type}</span>;
      case "br": return <br key={key} />;
      default: return <Fragment key={key}>{children}</Fragment>;
    }
  });
}

export function DiscordText({ text, scope }: { text: string; scope?: Scope }) {
  return <>{render(parseDiscord(text), scope)}</>;
}
