import type { PairingPayload } from "./types";

/** A paired server and this browser's device credential. */
export interface Account {
  endpoint: string;
  deviceId: string;
  token: string;
  expiresAtMs: number;
  /**
   * A rotation's replacement secret, kept until the rotation is known to have
   * worked or failed. The proposed token is `<deviceId>.<pendingSecret>`.
   */
  pendingSecret?: string;
}

const KEY = "qmsg.account";

// Browsers have no keychain for a page. The token lives in this origin's
// local storage, so anything that can run script here can read it.
export const accounts = {
  load(): Account | null {
    try {
      const raw = localStorage.getItem(KEY);
      return raw ? (JSON.parse(raw) as Account) : null;
    } catch {
      return null;
    }
  },
  save(account: Account) {
    localStorage.setItem(KEY, JSON.stringify(account));
  },
  delete() {
    localStorage.removeItem(KEY);
  },
};

export function proposedToken(account: Account): string | null {
  return account.pendingSecret ? `${account.deviceId}.${account.pendingSecret}` : null;
}

export function randomSecret(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(32)), (b) => b.toString(16).padStart(2, "0")).join("");
}

export class PairingError extends Error {
  override name = "PairingError";
}

function isLoopback(host: string): boolean {
  const h = host.replace(/^\[|\]$/g, "");
  return h === "localhost" || h === "::1" || /^127\.\d+\.\d+\.\d+$/.test(h);
}

/** Parses and checks what `qmsg-orchestrator pair --json` printed. */
export function parsePairing(text: string, now = Date.now()): PairingPayload {
  let payload: PairingPayload;
  try {
    payload = JSON.parse(text.trim()) as PairingPayload;
  } catch {
    throw new PairingError("That isn't a qmsg pairing code. Paste the JSON from `qmsg-orchestrator pair --json`.");
  }
  if (!payload || typeof payload !== "object" || typeof payload.code !== "string" || typeof payload.endpoint !== "string") {
    throw new PairingError("That isn't a qmsg pairing code.");
  }
  if (payload.version !== 1) {
    throw new PairingError(`This pairing code is from a newer qmsg (version ${payload.version}). Update the app.`);
  }
  if (payload.expires_at_ms <= now) {
    throw new PairingError("This pairing code has expired. Run `qmsg-orchestrator pair` again.");
  }
  let url: URL;
  try {
    url = new URL(payload.endpoint);
  } catch {
    throw new PairingError(`The server address “${payload.endpoint}” isn't valid.`);
  }
  if (url.username || url.password || url.search || url.hash || (url.pathname !== "/" && url.pathname !== "")) {
    throw new PairingError(`The server address “${payload.endpoint}” isn't valid.`);
  }
  if (url.protocol === "http:" && !isLoopback(url.hostname)) {
    throw new PairingError("The server must use HTTPS unless it is reached through a local tunnel.");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new PairingError(`The server address “${payload.endpoint}” isn't valid.`);
  }
  return payload;
}

/**
 * Where to send API calls. The dev server proxies the API, and a page served
 * by the orchestrator itself (`app.web_dir`) shares its origin; otherwise use
 * the paired endpoint, which then needs this page's origin in `app.web_origins`.
 */
export function apiBase(endpoint: string): string {
  if (import.meta.env.DEV) return location.origin;
  const url = new URL(endpoint);
  return url.origin;
}

export function defaultDeviceName(): string {
  const ua = navigator.userAgent;
  const browser = /Firefox\//.test(ua)
    ? "Firefox"
    : /Edg\//.test(ua)
      ? "Edge"
      : /Chrome\//.test(ua)
        ? "Chrome"
        : /Safari\//.test(ua)
          ? "Safari"
          : "Browser";
  const os = /Mac OS X/.test(ua)
    ? "macOS"
    : /Windows/.test(ua)
      ? "Windows"
      : /Android/.test(ua)
        ? "Android"
        : /iPhone|iPad/.test(ua)
          ? "iOS"
          : /Linux/.test(ua)
            ? "Linux"
            : "";
  return `${browser}${os ? ` on ${os}` : ""} (web)`;
}
