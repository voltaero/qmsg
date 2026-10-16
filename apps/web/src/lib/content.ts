import { tag, type Content, type Email, type Media, type MediaKind, type Message } from "~/api/types";

/** The kind of content a file is sent as, from its MIME type. */
export function fileKind(mime: string): MediaKind {
  switch (mime.split("/")[0]) {
    case "image":
      return "Image";
    case "video":
      return "Video";
    case "audio":
      return "Audio";
    default:
      return "File";
  }
}

export function contentForFile(file: File, media: Media): Content {
  switch (fileKind(file.type)) {
    case "Image":
      return { Image: media };
    case "Video":
      return { Video: media };
    case "Audio":
      return { Audio: media };
    default:
      return { File: media };
  }
}

export function mediaOf(content: Content): { kind: MediaKind; media: Media } | null {
  const [kind, value] = tag(content);
  if (kind === "Image" || kind === "Video" || kind === "Audio" || kind === "File") return { kind, media: value as Media };
  return null;
}

/** The plain text of a text part. */
export function textOf(content: Content): string | null {
  if ("Text" in content) return content.Text;
  if ("Formatted" in content) return content.Formatted.text;
  return null;
}

/** A one-line summary of a message, for lists and notifications. */
export function plainText(message: Message): string {
  if (message.email) return message.email.subject || "(no subject)";
  const text = message.content.map(textOf).filter(Boolean).join(" ").replace(/\s+/g, " ").trim();
  if (text) return text;
  for (const part of message.content) {
    const m = mediaOf(part);
    if (m) return m.kind === "Image" ? "Photo" : m.kind === "Video" ? "Video" : m.kind === "Audio" ? "Audio" : (m.media.name ?? "File");
    if ("Custom" in part) return customLabel(part.Custom.kind);
  }
  return "";
}

export function customLabel(kind: string): string {
  switch (kind) {
    case "discord-embed":
    case "embed":
      return "Embed";
    case "discord-sticker":
    case "sticker":
      return "Sticker";
    case "email-summary":
      return "Email";
    default:
      return kind.replace(/[-_]/g, " ");
  }
}

/** Custom content as JSON, when it is JSON. */
export function customJson(data: number[]): unknown {
  try {
    return JSON.parse(new TextDecoder().decode(new Uint8Array(data)));
  } catch {
    return null;
  }
}

export function formatBytes(n: number | null | undefined): string {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB"];
  let value = n / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export function formatDuration(ms: number | null | undefined): string {
  if (!ms) return "";
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

const time = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const date = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
const fullDate = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" });
const long = new Intl.DateTimeFormat(undefined, { dateStyle: "full", timeStyle: "short" });
const dayHeader = new Intl.DateTimeFormat(undefined, { weekday: "long", month: "long", day: "numeric" });

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

export const formatTime = (ms: number) => time.format(ms);
export const formatLong = (ms: number) => long.format(ms);

/** Short relative stamp for lists: time today, weekday this week, date otherwise. */
export function formatStamp(ms: number, now = Date.now()): string {
  const days = (startOfDay(now) - startOfDay(ms)) / 86_400_000;
  if (days <= 0) return time.format(ms);
  if (days < 7) return weekday.format(ms);
  if (new Date(ms).getFullYear() === new Date(now).getFullYear()) return date.format(ms);
  return fullDate.format(ms);
}

export function formatDay(ms: number, now = Date.now()): string {
  const days = (startOfDay(now) - startOfDay(ms)) / 86_400_000;
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (new Date(ms).getFullYear() === new Date(now).getFullYear()) return dayHeader.format(ms);
  return fullDate.format(ms);
}

export function sameDay(a: number, b: number): boolean {
  return startOfDay(a) === startOfDay(b);
}

export function addressName(a: { address: string; name?: string | null }): string {
  return a.name?.trim() || a.address;
}

/** The HTML body as plain text, for previews of mail without a text part. */
export function htmlText(email: Email): string {
  if (!email.html) return "";
  const doc = new DOMParser().parseFromString(email.html, "text/html");
  doc.querySelectorAll("style,script,head,title").forEach((n) => n.remove());
  return (doc.body.textContent ?? "").replace(/[ ‌͏]/g, " ").replace(/\s+/g, " ").trim();
}

export function initials(name: string): string {
  const parts = name.replace(/[^\p{L}\p{N}\s]/gu, "").trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return name.slice(0, 1).toUpperCase() || "?";
  return ((parts[0]![0] ?? "") + (parts.length > 1 ? (parts.at(-1)![0] ?? "") : "")).toUpperCase();
}

/** A stable hue for a name, for avatars without a picture. */
export function hue(seed: string): number {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return h % 360;
}
