import { Download, EyeOff, FileIcon, Film, ImageOff, Music, Play, X } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Media, MediaKind } from "~/api/types";
import { cn } from "~/lib/cn";
import { formatBytes, formatDuration } from "~/lib/content";
import { useVisible } from "~/lib/useVisible";
import { download, previewOf, useMedia } from "~/store/media";
import { toast } from "~/store/ui";
import { describe } from "~/api/errors";
import { Spinner } from "../ui";

const MAX_W = 420;
const MAX_H = 320;

function fit(media: Media): { width: number; height: number } | null {
  const w = media.metadata?.width;
  const h = media.metadata?.height;
  if (!w || !h) return null;
  const scale = Math.min(1, MAX_W / w, MAX_H / h);
  return { width: Math.round(w * scale), height: Math.round(h * scale) };
}

export function MediaPart({ kind, media, local }: { kind: MediaKind; media: Media; local?: string }) {
  const [revealed, setRevealed] = useState(!media.metadata?.spoiler);
  if (!revealed) {
    return (
      <button
        onClick={() => setRevealed(true)}
        className="flex h-24 w-56 items-center justify-center gap-2 rounded-xl border border-border bg-muted text-[13px] text-muted-foreground hover:bg-accent"
      >
        <EyeOff className="size-4" /> Spoiler — show
      </button>
    );
  }
  switch (kind) {
    case "Image":
      return <ImagePart media={media} local={local} />;
    case "Video":
      return <VideoPart media={media} local={local} />;
    case "Audio":
      return <AudioPart media={media} />;
    default:
      return <FilePart media={media} />;
  }
}

function ImagePart({ media, local }: { media: Media; local?: string }) {
  const [ref, visible] = useVisible<HTMLDivElement>();
  const { url, error, loading } = useMedia(local ? null : previewOf(media), visible);
  const [open, setOpen] = useState(false);
  const size = fit(media);
  const src = local ?? url;
  return (
    <div
      ref={ref}
      className="relative max-w-full overflow-hidden rounded-xl border border-border bg-muted"
      style={size ? { width: size.width, aspectRatio: `${size.width} / ${size.height}` } : { maxWidth: MAX_W }}
    >
      {src ? (
        <button onClick={() => setOpen(true)} className="block size-full cursor-zoom-in">
          <img
            src={src}
            alt={media.metadata?.description ?? media.name ?? ""}
            draggable={false}
            className={cn("block size-full object-contain", !size && "max-h-80")}
          />
        </button>
      ) : (
        <div className={cn("flex items-center justify-center text-muted-foreground", size ? "size-full" : "h-40 w-64")}>
          {error ? (
            <span className="flex flex-col items-center gap-1.5 px-4 text-center text-xs">
              <ImageOff className="size-5" />
              {error}
            </span>
          ) : loading ? (
            <Spinner />
          ) : null}
        </div>
      )}
      {open && src ? <Lightbox media={media} preview={src} onClose={() => setOpen(false)} /> : null}
    </div>
  );
}

function Lightbox({ media, preview, onClose }: { media: Media; preview: string; onClose: () => void }) {
  // The preview may be smaller than the original.
  const { url } = useMedia(previewOf(media) === media ? null : media);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return createPortal(
    <div className="fixed inset-0 z-50 flex flex-col bg-black/85 backdrop-blur-sm animate-fade-in" onClick={onClose}>
      <div className="flex items-center gap-2 p-3 text-white/80" onClick={(e) => e.stopPropagation()}>
        <span className="min-w-0 flex-1 truncate text-[13px]">{media.name}</span>
        <button
          className="flex size-8 items-center justify-center rounded-lg hover:bg-white/10"
          onClick={() => download(media).catch((e) => toast(describe(e), "error"))}
          aria-label="Download"
        >
          <Download className="size-4" />
        </button>
        <button className="flex size-8 items-center justify-center rounded-lg hover:bg-white/10" onClick={onClose} aria-label="Close">
          <X className="size-4" />
        </button>
      </div>
      <div className="flex min-h-0 flex-1 items-center justify-center p-6 pt-0">
        <img src={url ?? preview} alt="" className="max-h-full max-w-full rounded-lg object-contain shadow-2xl" onClick={(e) => e.stopPropagation()} />
      </div>
    </div>,
    document.body,
  );
}

function VideoPart({ media, local }: { media: Media; local?: string }) {
  const [play, setPlay] = useState(!!local);
  const { url, error, loading } = useMedia(local ? null : media, play);
  const poster = media.metadata?.preview_url ? previewOf(media) : null;
  const { url: posterUrl } = useMedia(poster && poster !== media ? poster : null);
  const size = fit(media);
  const src = local ?? url;
  return (
    <div
      className="relative overflow-hidden rounded-xl border border-border bg-black"
      style={size ? { width: size.width, aspectRatio: `${size.width} / ${size.height}` } : { width: 360, aspectRatio: "16 / 9" }}
    >
      {src ? (
        <video src={src} controls autoPlay={!local} playsInline className="size-full" />
      ) : (
        <button onClick={() => setPlay(true)} className="group flex size-full items-center justify-center" aria-label="Play video">
          {posterUrl ? <img src={posterUrl} alt="" className="absolute inset-0 size-full object-cover opacity-80" /> : null}
          <span className="relative flex size-12 items-center justify-center rounded-full bg-black/60 text-white backdrop-blur transition-transform group-hover:scale-105">
            {loading ? <Spinner className="text-white" /> : <Play className="size-5 fill-white" />}
          </span>
          <span className="absolute bottom-2 left-2 flex items-center gap-1 rounded bg-black/60 px-1.5 py-0.5 text-2xs text-white">
            <Film className="size-3" /> {formatDuration(media.metadata?.duration_ms) || formatBytes(media.size)}
          </span>
          {error ? <span className="absolute inset-x-2 top-2 rounded bg-black/70 px-2 py-1 text-xs text-white">{error}</span> : null}
        </button>
      )}
    </div>
  );
}

function AudioPart({ media }: { media: Media }) {
  const [play, setPlay] = useState(false);
  const { url, error, loading } = useMedia(media, play);
  return (
    <div className="flex w-80 max-w-full items-center gap-3 rounded-xl border border-border bg-card p-2.5">
      <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/12 text-primary">
        <Music className="size-4" />
      </span>
      {url ? (
        <audio src={url} controls autoPlay className="h-8 min-w-0 flex-1" />
      ) : (
        <button onClick={() => setPlay(true)} className="min-w-0 flex-1 text-left">
          <div className="truncate text-[13px] font-medium">{media.name ?? "Audio"}</div>
          <div className="text-xs text-muted-foreground">
            {error ?? (loading ? "Loading…" : [formatDuration(media.metadata?.duration_ms), formatBytes(media.size)].filter(Boolean).join(" · "))}
          </div>
        </button>
      )}
    </div>
  );
}

function FilePart({ media }: { media: Media }) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      onClick={() => {
        setBusy(true);
        download(media)
          .catch((e) => toast(describe(e), "error"))
          .finally(() => setBusy(false));
      }}
      className="flex w-72 max-w-full items-center gap-3 rounded-xl border border-border bg-card p-2.5 text-left hover:bg-accent"
    >
      <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-accent text-muted-foreground">
        <FileIcon className="size-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-medium">{media.name ?? "File"}</span>
        <span className="block text-xs text-muted-foreground">{[media.mime, formatBytes(media.size)].filter(Boolean).join(" · ")}</span>
      </span>
      {busy ? <Spinner /> : <Download className="size-4 text-muted-foreground" />}
    </button>
  );
}
