import { afterAll, beforeAll, expect, test } from "bun:test";
import { chromium, type Browser } from "playwright";
import { LoginService } from "../src/service";
import { ensureBrowser } from "../src/setup";
import { providers, type LoginDefaults } from "../src/defaults";

const key = "test-internal-key-not-a-real-secret";
let browser: Browser;
const site = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: request => {
  if (new URL(request.url).pathname === "/auth/login") {
    return Response.json({ token: "test-session-result" });
  }
  return new Response('<input style="position:absolute;left:20px;top:20px;width:200px;height:40px" id="password" type="password"><button style="position:absolute;left:20px;top:90px;width:200px;height:40px" onclick="fetch(\'/auth/login\',{method:\'POST\'}).then(r=>r.json()).then(()=>document.body.textContent=\'Done\')">Log in</button>', { headers: { "content-type": "text/html" } });
}});
const recipe: LoginDefaults = {
  url: site.url.toString(), context: { viewport: { width: 400, height: 200 } },
  tokenResponse: { origin: site.url.origin, path: /^\/auth\/login$/, field: "token" },
};
beforeAll(async () => { await ensureBrowser(); browser = await chromium.launch(); }, 120_000);
afterAll(async () => { await browser?.close(); site.stop(true); });

function call(service: LoginService, path: string, method = "GET", body?: unknown, auth = true, origin = "http://qmsg.test") {
  return service.fetch(new Request("http://qmsg.test" + path, {
    method, headers: { ...(auth ? { authorization: "Bearer " + key } : {}), origin, "content-type": "application/json" },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  }));
}
async function ready(service: LoginService, id: string) {
  for (let i = 0; i < 100; i++) {
    const response = await call(service, "/sessions/" + id);
    const state = (await response.json()).state;
    if (state === "active" || state === "complete" || state === "failed") return state;
    await Bun.sleep(20);
  }
  throw new Error("Browser did not start");
}

test("real browser input completes login; viewer cannot claim its result", async () => {
  const service = new LoginService({ key, defaults: { fake: recipe }, launch: async () => browser });
  try {
    const response = await call(service, "/sessions", "POST", { provider: "fake" });
    expect(response.status).toBe(201);
    const { id, path } = await response.json();
    expect(await ready(service, id)).toBe("active");
    const screenshot = await call(service, path + "/frame", "GET", undefined, false);
    expect(screenshot.headers.get("content-type")).toBe("image/jpeg");
    expect((await screenshot.arrayBuffer()).byteLength).toBeGreaterThan(100);
    expect((await call(service, path + "/input", "POST", { kind: "click", x: 40, y: 40 }, false)).status).toBe(200);
    expect((await call(service, path + "/input", "POST", { kind: "text", text: "test-password" }, false)).status).toBe(200);
    expect(await browser.contexts()[0].pages()[0].locator("#password").inputValue()).toBe("test-password");
    expect((await call(service, path + "/input", "POST", { kind: "click", x: 40, y: 110 }, false)).status).toBe(200);
    for (let i = 0; i < 100; i++) {
      if ((await (await call(service, "/sessions/" + id)).json()).state === "complete") break;
      await Bun.sleep(20);
    }
    expect(await (await call(service, path + "/state", "GET", undefined, false)).json()).toEqual({ state: "complete" });
    expect((await call(service, "/sessions/" + id + "/claim", "POST", {}, false)).status).toBe(401);
    expect(await (await call(service, "/sessions/" + id + "/claim", "POST", {})).json()).toEqual({ token: "test-session-result" });
    expect((await call(service, "/sessions/" + id + "/claim", "POST", {})).status).toBe(404);
    expect((await call(service, path + "/state", "GET", undefined, false)).status).toBe(404);
    expect(browser.contexts().length).toBe(0);
  } finally {
    // This service owns the shared browser; use a new browser for subsequent tests.
    await service.close();
    browser = await chromium.launch();
  }
}, 20_000);

test("sessions are isolated, bounded and cancelled with contexts closed", async () => {
  const service = new LoginService({ key, limit: 2, defaults: { fake: recipe }, launch: async () => browser });
  try {
    expect((await call(service, "/sessions", "POST", { provider: "fake" }, false)).status).toBe(401);
    const a = await (await call(service, "/sessions", "POST", { provider: "fake" })).json();
    const b = await (await call(service, "/sessions", "POST", { provider: "fake" })).json();
    expect(a.path).not.toBe(b.path);
    expect(await ready(service, a.id)).toBe("active");
    expect(await ready(service, b.id)).toBe("active");
    expect(browser.contexts().length).toBe(2);
    expect((await call(service, "/sessions", "POST", { provider: "fake" })).status).toBe(429);
    expect((await call(service, a.path + "/input", "POST", { kind: "text", text: "value" }, false, "https://other.test")).status).toBe(403);
    expect((await call(service, a.path + "/input", "POST", { kind: "click", x: Infinity, y: 10 }, false)).status).toBe(400);
    expect((await call(service, "/sessions/" + a.id, "DELETE")).status).toBe(200);
    expect(browser.contexts().length).toBe(1);
    expect((await call(service, a.path + "/frame", "GET", undefined, false)).status).toBe(404);
    expect((await call(service, b.path + "/frame", "GET", undefined, false)).status).toBe(200);
  } finally { await service.close(); browser = await chromium.launch(); }
}, 20_000);

test("startup failure and expiry do not retain browser contexts", async () => {
  const failed = new LoginService({ key, defaults: { fake: recipe }, launch: async () => { throw new Error("missing browser"); } });
  const { id } = await (await call(failed, "/sessions", "POST", { provider: "fake" })).json();
  expect(await ready(failed, id)).toBe("failed");
  await failed.close();
  const expires = new LoginService({ key, lifetimeMs: 80, defaults: { fake: recipe }, launch: async () => browser });
  const session = await (await call(expires, "/sessions", "POST", { provider: "fake" })).json();
  await Bun.sleep(200);
  expect((await call(expires, session.path + "/state", "GET", undefined, false)).status).toBe(404);
  expect(browser.contexts().length).toBe(0);
  await expires.close();
  browser = await chromium.launch();
}, 20_000);

test("Discord defaults capture only official successful auth responses", () => {
  expect(providers.discord.url).toBe("https://discord.com/login");
  expect(providers.discord.tokenResponse.origin).toBe("https://discord.com");
  expect(providers.discord.tokenResponse.path.test("/api/v9/auth/mfa/totp")).toBe(true);
  expect(providers.discord.tokenResponse.path.test("/api/v9/users/@me")).toBe(false);
});

test("host-assigned ids can be cancelled before or during creation", async () => {
  const service = new LoginService({ key, defaults: { fake: recipe }, launch: async () => browser });
  try {
    const cancelled = "11111111111111111111111111111111";
    expect((await call(service, "/sessions/" + cancelled, "DELETE")).status).toBe(200);
    expect((await call(service, "/sessions", "POST", { provider: "fake", id: cancelled })).status).toBe(409);
    const id = "22222222222222222222222222222222";
    const started = await call(service, "/sessions", "POST", { provider: "fake", id });
    expect((await started.json()).id).toBe(id);
    expect((await call(service, "/sessions/" + id, "DELETE")).status).toBe(200);
    await Bun.sleep(100);
    expect(browser.contexts().length).toBe(0);
    expect((await call(service, "/sessions", "POST", { provider: "fake", id })).status).toBe(409);
  } finally { await service.close(); browser = await chromium.launch(); }
}, 20_000);

test("reverse proxy uses its configured origin, never forwarded headers", async () => {
  const service = new LoginService({
    key, defaults: { fake: recipe }, viewerOrigin: "https://public.qmsg.test",
    launch: async () => browser,
  });
  try {
    const { id, path } = await (await call(service, "/sessions", "POST", { provider: "fake" })).json();
    expect(await ready(service, id)).toBe("active");
    const body = { kind: "click", x: 40, y: 40 };
    expect((await call(service, path + "/input", "POST", body, false, "https://public.qmsg.test")).status).toBe(200);
    expect((await call(service, path + "/input", "POST", body, false)).status).toBe(403);
    expect((await call(service, path + "/input", "POST", body, false, "https://other.test")).status).toBe(403);
    const spoofed = new Request("http://qmsg.test" + path + "/input", {
      method: "POST", headers: { origin: "https://other.test", "x-forwarded-host": "other.test", "x-forwarded-proto": "https" },
      body: JSON.stringify(body),
    });
    expect((await service.fetch(spoofed)).status).toBe(403);
  } finally { await service.close(); browser = await chromium.launch(); }
}, 20_000);

test("the viewer page sends clicks and text through the real HTTP service", async () => {
  const service = new LoginService({ key, defaults: { fake: recipe }, launch: async () => browser });
  const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: request => service.fetch(request) });
  const client = await chromium.launch();
  try {
    const response = await fetch(new URL("/sessions", server.url), {
      method: "POST", headers: { authorization: "Bearer " + key, "content-type": "application/json" },
      body: JSON.stringify({ provider: "fake" }),
    });
    const { id, path } = await response.json();
    const page = await client.newPage();
    await page.goto(new URL(path, server.url).toString());
    await page.locator("#frame").waitFor({ state: "visible", timeout: 5000 });
    await page.waitForFunction(() => {
      const image = document.querySelector<HTMLImageElement>("#frame");
      return image?.complete && image.naturalWidth > 0;
    }, undefined, { timeout: 5000 });
    const image = page.locator("#frame");
    const box = await image.boundingBox();
    if (!box) throw new Error("No browser image");
    // The 400 x 200 browser scales to the viewer's displayed width.
    await page.mouse.click(box.x + 40 * box.width / 400, box.y + 40 * box.height / 200);
    await Bun.sleep(100);
    await page.locator("#text").fill("test-password");
    await page.locator("#send").click();
    await Bun.sleep(100);
    expect(await browser.contexts()[0].pages()[0].locator("#password").inputValue()).toBe("test-password");
    await page.mouse.click(box.x + 40 * box.width / 400, box.y + 110 * box.height / 200);
    await page.locator("#state").filter({ hasText: "Logged in" }).waitFor({ timeout: 5000 });
    expect(await page.locator("#text").inputValue()).toBe("");
    expect(await page.locator("body").textContent()).not.toContain("test-session-result");
    const result = await fetch(new URL("/sessions/" + id + "/claim", server.url), {
      method: "POST", headers: { authorization: "Bearer " + key },
    });
    expect(await result.json()).toEqual({ token: "test-session-result" });
  } finally {
    await client.close();
    await service.close();
    server.stop(true);
    browser = await chromium.launch();
  }
}, 20_000);

test("cancellation bookkeeping stays bounded without forgetting live cancellations", async () => {
  const service = new LoginService({ key, defaults: { fake: recipe }, launch: async () => { throw new Error("must not launch"); } });
  try {
    for (let n = 0; n < 257; n++) {
      const response = await service.fetch(new Request("http://qmsg.test/sessions/cancelled-session-" + n, {
        method: "DELETE", headers: { authorization: "Bearer " + key },
      }));
      expect(response.status).toBe(200);
    }
    for (const [id, status] of [["cancelled-session-0", 409], ["cancelled-session-256", 429]] as const) {
      const response = await service.fetch(new Request("http://qmsg.test/sessions", {
        method: "POST", headers: { authorization: "Bearer " + key },
        body: JSON.stringify({ provider: "fake", id }),
      }));
      expect(response.status).toBe(status);
    }
  } finally { await service.close(); }
});

test("QR login captures only a successful authenticated response", async () => {
  const signedIn = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: request => {
    if (new URL(request.url).pathname === "/me") {
      return Response.json({ id: "fake-user" }, { status: request.headers.get("authorization") === "test-qr-result" ? 200 : 403 });
    }
    return new Response("Sign in with QR", { headers: { "content-type": "text/html" } });
  }});
  const service = new LoginService({ key, launch: async () => browser, defaults: { fake: {
    url: signedIn.url.toString(),
    tokenResponse: { origin: signedIn.url.origin, path: /^\/auth\/login$/, field: "token" },
    authenticatedResponse: { origin: signedIn.url.origin, path: /^\/me$/ },
  } } });
  try {
    const { id, path } = await (await call(service, "/sessions", "POST", { provider: "fake" })).json();
    expect(await ready(service, id)).toBe("active");
    const page = browser.contexts()[0].pages()[0];
    await page.evaluate(async () => { await fetch("/me", { headers: { authorization: "rejected" } }); });
    await Bun.sleep(50);
    expect(await ready(service, id)).toBe("active");
    await page.evaluate(async () => { await fetch("/wrong-path", { headers: { authorization: "test-qr-result" } }); });
    await Bun.sleep(50);
    expect(await ready(service, id)).toBe("active");
    await page.evaluate(() => { void fetch("/me", { headers: { authorization: "test-qr-result" } }); });
    for (let i = 0; i < 100; i++) {
      if ((await (await call(service, "/sessions/" + id)).json()).state === "complete") break;
      await Bun.sleep(20);
    }
    expect(await ready(service, id)).toBe("complete");
    expect(await (await call(service, path + "/state", "GET", undefined, false)).json()).toEqual({ state: "complete" });
    expect(await (await call(service, "/sessions/" + id + "/claim", "POST", {})).json()).toEqual({ token: "test-qr-result" });
    expect(browser.contexts().length).toBe(0);
  } finally {
    await service.close(); signedIn.stop(true); browser = await chromium.launch();
  }
}, 20_000);
