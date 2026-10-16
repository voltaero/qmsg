import { SimpleMarkdown, rulesExtended, type parse } from "discord-markdown-parser";

// Use Discord's text rules, including masked links. Keep network-specific
// tokens that we don't render (timestamps, custom emoji, etc.) as literal text.
const parser = SimpleMarkdown.parserFor(Object.fromEntries([
  "blockQuote", "codeBlock", "newline", "escape", "autolink", "url", "link",
  "em", "strong", "underline", "strikethrough", "inlineCode", "text", "br",
  "spoiler", "heading", "subtext", "user", "channel", "role", "everyone", "here",
].map((name) => [name, rulesExtended[name]!]).concat([["list", SimpleMarkdown.defaultRules.list]])));

export type DiscordNode = ReturnType<typeof parse>[number];

export function parseDiscord(text: string): DiscordNode[] {
  // _list permits lists at line starts while preserving Discord's inline rules.
  return parser(text, { inline: true, _list: true });
}

/** A display summary must not reveal spoilers, including in notifications. */
export function discordPreview(text: string): string {
  const plain = (nodes: DiscordNode[]): string => nodes.map((node) => {
    if (node.type === "spoiler") return "[spoiler]";
    if (node.type === "br") return " ";
    if (node.type === "list") return node.items.map(plain).join(" ");
    if (node.type === "user") return `@${node.id}`;
    if (node.type === "channel") return `#${node.id}`;
    if (node.type === "role") return `@${node.id}`;
    if (node.type === "everyone" || node.type === "here") return `@${node.type}`;
    return Array.isArray(node.content) ? plain(node.content) : (node.content ?? "");
  }).join("");
  return plain(parseDiscord(text));
}

