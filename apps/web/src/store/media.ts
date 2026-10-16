// Loads media for display. Public URLs are used as they are; backend blobs
// are read in chunks, at most four at a time to match the server's read
// quota, and released as soon as they are read. Results are cached as object
// URLs, within a byte budget.

import { useEffect, useState } from "react";
import type { Media } from "~/api/types";
import { describe } from "~/api/errors";
import { getBackend } from "./store";

const MAX_CONCURRENT = 4;
const BUDGET = 192 * 1024 * 1024;
const MAX_IMAGE = 25 * 1024 * 1024;
const MAX_OTHER = 100 * 1024 * 1024;

interface Entry {
  url: string;
  size: number;
}

const cache = new Map<string, Entry>();
const loading = new Map<string, Promise<string>>();
let cached = 0;
let running = 0;
const queue: (() => void)[] = [];

function slot(): Promise<() => void> {
  return new Promise((resolve) => {
    const start = () => {
      running++;
      resolve(() => {
        running--;
        queue.shift()?.();
      });
    };
    if (running < MAX_CONCURRENT) start();
    else queue.push(start);
  });
}

function remember(key: string, blob: Blob): string {
  const url = URL.createObjectURL(blob);
  cache.set(key, { url, size: blob.size });
  cached += blob.size;
  // Map iteration is insertion order: evict the oldest.
  for (const [k, entry] of cache) {
    if (cached <= BUDGET || k === key) break;
    URL.revokeObjectURL(entry.url);
    cache.delete(k);
    cached -= entry.size;
  }
  return url;
}

function touch(key: string): string | null {
  const entry = cache.get(key);
  if (!entry) return null;
  cache.delete(key);
  cache.set(key, entry);
  return entry.url;
}

/** Only web links, never `javascript:` or other schemes from a provider. */
export function safeUrl(raw: string | null | undefined): string | null {
  if (!raw) return null;
  try {
    const url = new URL(raw);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}

function expired(media: Media): boolean {
  const at = media.metadata?.expires_at_ms;
  return !!at && at < Date.now();
}

/** A URL to show this media with. */
export async function mediaUrl(media: Media): Promise<string> {
  const source = media.source;
  if ("Url" in source) {
    const url = safeUrl(source.Url);
    if (!url) throw new Error("This link isn't a web address.");
    if (expired(media)) throw new Error("This link has expired; refresh the conversation.");
    return url;
  }
  const mime = media.mime ?? "application/octet-stream";
  if ("Bytes" in source) {
    const key = `bytes:${source.Bytes.length}:${source.Bytes.slice(0, 32).join(",")}`;
    return touch(key) ?? remember(key, new Blob([new Uint8Array(source.Bytes)], { type: mime }));
  }
  const id = source.Blob;
  const hit = touch(id);
  if (hit) return hit;
  const running = loading.get(id);
  if (running) return running;
  const task = (async () => {
    const backend = getBackend();
    if (!backend) throw new Error("Not connected to the server.");
    const done = await slot();
    try {
      const limit = mime.startsWith("image/") ? MAX_IMAGE : MAX_OTHER;
      const bytes = await backend.readBlob(id, limit);
      void backend.releaseBlob(id).catch(() => {});
      return remember(id, new Blob([bytes], { type: mime }));
    } finally {
      done();
      loading.delete(id);
    }
  })();
  loading.set(id, task);
  return task;
}

/** A smaller picture to show in a list, when the provider gave one. */
export function previewOf(media: Media): Media {
  const preview = safeUrl(media.metadata?.preview_url);
  if (preview && media.mime?.startsWith("image/")) return { ...media, source: { Url: preview } };
  return media;
}

export interface MediaState {
  url: string | null;
  error: string | null;
  loading: boolean;
}

/** Loads media while `enabled`, such as once it scrolls into view. */
export function useMedia(media: Media | null | undefined, enabled = true): MediaState {
  const [state, setState] = useState<MediaState>({ url: null, error: null, loading: false });
  const identity = media ? JSON.stringify(media.source) : "";
  useEffect(() => {
    if (!media || !enabled) return;
    let live = true;
    setState((s) => ({ ...s, loading: true, error: null }));
    mediaUrl(media).then(
      (url) => live && setState({ url, error: null, loading: false }),
      (error) => live && setState({ url: null, error: describe(error), loading: false }),
    );
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identity, enabled]);
  return state;
}

/** Saves media to the user's downloads. */
export async function download(media: Media) {
  const url = await mediaUrl(media);
  const a = document.createElement("a");
  a.href = url;
  a.download = media.name ?? "download";
  a.rel = "noopener";
  if (!url.startsWith("blob:")) a.target = "_blank";
  document.body.append(a);
  a.click();
  a.remove();
}
