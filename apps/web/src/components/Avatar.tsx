import { memo } from "react";
import type { Media, Presence } from "~/api/types";
import { cn } from "~/lib/cn";
import { hue, initials } from "~/lib/content";
import { previewOf, useMedia } from "~/store/media";

const presenceColor: Record<Presence, string> = {
  Online: "bg-success",
  Idle: "bg-warning",
  Busy: "bg-destructive",
  Offline: "bg-muted-foreground/50",
};

/** A picture, or initials on a color derived from the name. */
export const Avatar = memo(function Avatar({
  name,
  seed,
  media,
  size = 32,
  square,
  presence,
  className,
}: {
  name: string;
  seed?: string;
  media?: Media | null;
  size?: number;
  square?: boolean;
  presence?: Presence | null;
  className?: string;
}) {
  const { url } = useMedia(media ? previewOf(media) : null);
  const h = hue(seed ?? name);
  return (
    <span className={cn("relative inline-flex shrink-0", className)} style={{ width: size, height: size }}>
      {url ? (
        <img
          src={url}
          alt=""
          draggable={false}
          className={cn("size-full object-cover", square ? "rounded-[28%]" : "rounded-full")}
        />
      ) : (
        <span
          aria-hidden
          className={cn(
            "flex size-full items-center justify-center font-semibold text-white select-none",
            square ? "rounded-[28%]" : "rounded-full",
          )}
          style={{
            fontSize: Math.max(9, size * 0.38),
            background: `linear-gradient(135deg, oklch(0.62 0.13 ${h}), oklch(0.5 0.14 ${(h + 40) % 360}))`,
          }}
        >
          {initials(name)}
        </span>
      )}
      {presence ? (
        <span
          title={presence}
          className={cn(
            "absolute -right-0.5 -bottom-0.5 rounded-full ring-2 ring-[var(--avatar-ring,var(--sidebar))]",
            presenceColor[presence],
          )}
          style={{ width: Math.max(8, size * 0.3), height: Math.max(8, size * 0.3) }}
        />
      ) : null}
    </span>
  );
});
