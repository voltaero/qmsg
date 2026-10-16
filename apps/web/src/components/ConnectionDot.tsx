import { cn } from "~/lib/cn";
import type { Link } from "~/store/store";

export function linkLabel(link: Link): string {
  switch (link.kind) {
    case "live":
      return "Connected";
    case "connecting":
      return "Connecting…";
    case "retrying":
      return `Reconnecting: ${link.message}`;
    default:
      return "Offline";
  }
}

export function ConnectionDot({ link }: { link: Link }) {
  return (
    <span title={linkLabel(link)} className="flex items-center gap-1.5 rounded-md px-1.5 py-1 text-2xs text-sidebar-muted-foreground">
      <span
        className={cn(
          "size-2 rounded-full",
          link.kind === "live" && "bg-success",
          link.kind === "connecting" && "animate-pulse bg-warning",
          link.kind === "retrying" && "bg-destructive",
          link.kind === "idle" && "bg-muted-foreground/40",
        )}
      />
      {link.kind === "live" ? null : <span>{link.kind === "retrying" ? "Offline" : linkLabel(link)}</span>}
    </span>
  );
}
