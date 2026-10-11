import { timingSafeEqual } from "node:crypto";
import { chromium, type Browser, type BrowserContext, type Page } from "playwright";
import { providers, type LoginDefaults } from "./defaults";
import { ensureBrowser } from "./setup";

type State = "starting" | "active" | "complete" | "cancelled" | "expired" | "failed";
interface Session {
  id: string;
  viewer: string;
  state: State;
  deadline: number;
  timer: ReturnType<typeof setTimeout>;
  context?: BrowserContext;
  page?: Page;
  token?: string;
  busy: boolean;
  frameBusy?: boolean;
  closing?: Promise<void>;
}

export interface Options {
  key: string;
  defaults?: Record<string, LoginDefaults>;
  limit?: number;
  lifetimeMs?: number;
  launch?: () => Promise<Browser>;
  /** Exact trusted public origin when /view/ is behind a reverse proxy. */
  viewerOrigin?: string;
}

const json = (value: unknown, status = 200) => Response.json(value, {
  status, headers: { "cache-control": "no-store" },
});
const same = (a: string, b: string) => {
  const left = Buffer.from(a), right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
};

/** Bound to loopback by the host. Forward only /view/ routes to clients. */
export class LoginService {
  private sessions = new Map<string, Session>();
  // A cancellation may arrive before its POST. Keep recent host-assigned
  // ids so a late creation cannot reopen that login.
  private cancelled = new Map<string, number>();
  private browser?: Promise<Browser>;
  private stopped = false;
  private defaults: Record<string, LoginDefaults>;

  constructor(private options: Options) {
    if (options.key.length < 32) throw new Error("The internal key must have at least 32 characters");
    this.defaults = options.defaults ?? providers;
    if (options.viewerOrigin) {
      const origin = new URL(options.viewerOrigin);
      if (!["http:", "https:"].includes(origin.protocol) || origin.origin !== options.viewerOrigin) {
        throw new Error("viewerOrigin must be an HTTP or HTTPS origin");
      }
    }
  }

  private getBrowser() {
    if (!this.browser) {
      this.browser = (async () => {
        if (this.options.launch) return this.options.launch();
        await ensureBrowser();
        return chromium.launch({ headless: true, timeout: 30_000 });
      })();
      this.browser.catch(() => { this.browser = undefined; });
    }
    return this.browser;
  }

  private async finish(session: Session, state: State, token?: string) {
    if (session.state !== "starting" && session.state !== "active") {
      await session.closing;
      return;
    }
    session.state = state;
    session.token = token;
    const context = session.context;
    session.context = undefined;
    session.page = undefined;
    session.closing = context?.close().catch(() => {});
    await session.closing;
  }

  private expire(session: Session) {
    session.token = undefined;
    this.sessions.delete(session.id);
    void this.finish(session, "expired");
  }

  private async start(session: Session, recipe: LoginDefaults) {
    try {
      const browser = await this.getBrowser();
      if (session.state !== "starting" || this.stopped) return;
      const context = await browser.newContext({
        ...recipe.context, acceptDownloads: false, serviceWorkers: "block",
      });
      if (session.state !== "starting" || this.stopped) {
        await context.close();
        return;
      }
      session.context = context;
      const page = await context.newPage();
      session.page = page;
      page.on("response", async response => {
        const url = new URL(response.url());
        const capture = recipe.tokenResponse;
        const authenticated = recipe.authenticatedResponse;
        const login = url.origin === capture.origin && capture.path.test(url.pathname)
          && response.request().method() === "POST";
        const signedIn = authenticated && url.origin === authenticated.origin
          && authenticated.path.test(url.pathname) && response.request().method() === "GET";
        if ((!login && !signedIn) || response.status() !== 200
          || (session.state !== "active" && session.state !== "starting")) return;
        try {
          let token: unknown;
          if (signedIn) {
            token = await response.request().headerValue("authorization");
          } else {
            if (Number(response.headers()["content-length"]) > 65_536) return;
            const body = await response.text();
            if (body.length > 65_536) return;
            token = JSON.parse(body)[capture.field];
          }
          if (typeof token === "string" && token.length > 0 && token.length <= 4096) {
            await this.finish(session, "complete", token);
          }
        } catch { /* A failed or non-JSON login response isn't a login. */ }
      });
      context.on("close", () => {
        if (session.state === "active" || session.state === "starting") {
          void this.finish(session, "failed");
        }
      });
      await page.goto(recipe.url, { waitUntil: "domcontentloaded", timeout: 30_000 });
      if (session.state === "starting") session.state = "active";
    } catch {
      await this.finish(session, "failed");
    }
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const parts = url.pathname.split("/").filter(Boolean);
    if (this.stopped) return json({ error: "Service is stopped" }, 503);
    const internal = parts[0] === "sessions";
    if (internal && !same(request.headers.get("authorization") ?? "", "Bearer " + this.options.key)) {
      return json({ error: "Unauthorized" }, 401);
    }
    try {
      if (internal && parts.length === 1 && request.method === "POST") {
        if (this.sessions.size >= (this.options.limit ?? 4)) return json({ error: "Login limit reached" }, 429);
        const body = await this.body(request);
        const recipe = typeof body.provider === "string" ? this.defaults[body.provider] : undefined;
        if (!recipe) return json({ error: "Unknown provider" }, 400);
        const id = body.id ?? crypto.randomUUID();
        if (typeof id !== "string" || !/^[a-zA-Z0-9-]{16,80}$/.test(id)) return json({ error: "Invalid session id" }, 400);
        for (const [old, until] of this.cancelled) {
          if (until <= Date.now()) this.cancelled.delete(old);
        }
        if (this.sessions.has(id) || this.cancelled.has(id)) return json({ error: "Session already used" }, 409);
        if (this.cancelled.size >= 256) return json({ error: "Login limit reached" }, 429);
        const lifetime = this.options.lifetimeMs ?? 10 * 60_000;
        const session: Session = {
          id, viewer: crypto.randomUUID() + crypto.randomUUID(), state: "starting",
          deadline: Date.now() + lifetime, busy: false,
          timer: setTimeout(() => this.expire(session), lifetime),
        };
        this.sessions.set(id, session);
        void this.start(session, recipe);
        return json({ id, path: "/view/" + session.viewer, expiresAt: session.deadline }, 201);
      }
      if (internal && parts.length >= 2) {
        const session = this.sessions.get(parts[1]);
        if (parts.length === 2 && request.method === "DELETE") {
          if (!/^[a-zA-Z0-9-]{16,80}$/.test(parts[1])) return json({ error: "Invalid session id" }, 400);
          // Keep live cancellations: when full, new creation stays blocked
          // until they expire, so delayed requests cannot reopen a login.
          if (this.cancelled.size < 256 || this.cancelled.has(parts[1])) {
            this.cancelled.set(parts[1], Date.now() + 60_000);
          }
          if (session) {
            await this.finish(session, "cancelled");
            session.token = undefined;
            clearTimeout(session.timer);
            this.sessions.delete(session.id);
          }
          return json({ state: "cancelled" });
        }
        if (!session) return json({ error: "No such session" }, 404);
        if (parts.length === 2 && request.method === "GET") return json({ state: session.state });
        if (parts.length === 3 && parts[2] === "claim" && request.method === "POST") {
          await session.closing;
          if (this.sessions.get(session.id) !== session) return json({ error: "Login ended" }, 409);
          if (session.state !== "complete" || !session.token) return json({ error: "No result" }, 409);
          const token = session.token;
          session.token = undefined;
          clearTimeout(session.timer);
          this.sessions.delete(session.id);
          return json({ token });
        }
      }
      if (parts[0] === "view" && parts.length >= 2) {
        const session = [...this.sessions.values()].find(s => same(s.viewer, parts[1]));
        if (!session) return json({ error: "Login expired or ended" }, 404);
        // Viewer writes must come from the page's origin, not another website.
        if (request.method !== "GET" && request.headers.get("origin") !== (this.options.viewerOrigin ?? url.origin)) {
          return json({ error: "Wrong origin" }, 403);
        }
        if (parts.length === 2 && request.method === "GET") {
          return new Response(await Bun.file(new URL("./viewer.html", import.meta.url)).text(), {
            headers: {
              "content-type": "text/html; charset=utf-8", "referrer-policy": "no-referrer",
              "cache-control": "no-store",
              "content-security-policy": "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' blob:; frame-ancestors 'none'",
            },
          });
        }
        if (parts[2] === "state" && request.method === "GET") return json({ state: session.state });
        if (parts[2] === "frame" && request.method === "GET") {
          if (!session.page || session.state !== "active") return json({ state: session.state }, 409);
          if (session.busy || session.frameBusy) return json({ error: "Busy" }, 429);
          session.frameBusy = true;
          try {
            const bytes = await session.page.screenshot({ type: "jpeg", quality: 65, timeout: 5000 });
            return new Response(new Uint8Array(bytes), { headers: {
              "content-type": "image/jpeg", "cache-control": "no-store",
            } });
          } finally { session.frameBusy = false; }
        }
        if (parts[2] === "input" && request.method === "POST") {
          if (!session.page || session.state !== "active") return json({ state: session.state }, 409);
          if (session.busy) return json({ error: "Busy" }, 429);
          const body = await this.body(request);
          if (session.busy) return json({ error: "Busy" }, 429);
          const page = session.page;
          session.busy = true;
          try {
            const size = page.viewportSize();
            if (body.kind === "click" && size && typeof body.x === "number" && typeof body.y === "number"
              && Number.isFinite(body.x) && Number.isFinite(body.y)
              && body.x >= 0 && body.y >= 0 && body.x < size.width && body.y < size.height) {
              await page.mouse.click(body.x, body.y);
            } else if (body.kind === "text" && typeof body.text === "string" && body.text.length <= 4096) {
              await page.keyboard.insertText(body.text);
            } else if (body.kind === "key" && typeof body.key === "string"
              && /^(Tab|Enter|Backspace|Delete|Escape|ArrowUp|ArrowDown|ArrowLeft|ArrowRight|Control\+A|Meta\+A|Shift\+Tab)$/.test(body.key)) {
              await page.keyboard.press(body.key);
            } else if (body.kind === "scroll" && typeof body.dy === "number" && Number.isFinite(body.dy) && Math.abs(body.dy) <= 2000) {
              await page.mouse.wheel(0, body.dy);
            } else return json({ error: "Invalid input" }, 400);
            return json({ state: session.state });
          } finally { session.busy = false; }
        }
      }
      return json({ error: "No such route" }, 404);
    } catch {
      return json({ error: "Request failed" }, 400);
    }
  }

  private async body(request: Request): Promise<Record<string, unknown>> {
    const reader = request.body?.getReader();
    if (!reader) throw new Error("Missing body");
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        size += value.length;
        if (size > 16_384) throw new Error("Body too large");
        chunks.push(value);
      }
    } finally { await reader.cancel().catch(() => {}); }
    const value: unknown = JSON.parse(Buffer.concat(chunks).toString());
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid body");
    return value as Record<string, unknown>;
  }

  async close() {
    this.stopped = true;
    await Promise.all([...this.sessions.values()].map(async session => {
      clearTimeout(session.timer);
      session.token = undefined;
      await this.finish(session, "cancelled");
    }));
    this.sessions.clear();
    if (this.browser) await (await this.browser.catch(() => undefined))?.close();
  }
}
