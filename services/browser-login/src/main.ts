import { LoginService } from "./service";

const key = process.env.QMSG_BROWSER_KEY;
if (!key) throw new Error("Set QMSG_BROWSER_KEY to a private key of at least 32 characters");
const service = new LoginService({ key, viewerOrigin: process.env.QMSG_BROWSER_VIEWER_ORIGIN });
const server = Bun.serve({
  hostname: "127.0.0.1", port: Number(process.env.QMSG_BROWSER_PORT ?? 0),
  fetch: async request => {
    const response = await service.fetch(request);
    response.headers.set("cache-control", "no-store");
    response.headers.set("x-content-type-options", "nosniff");
    response.headers.set("content-security-policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' blob:; frame-ancestors 'none'");
    return response;
  },
});
// Only the address, never the management key or login result.
console.log(JSON.stringify({ url: server.url.toString() }));
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, async () => {
    await service.close();
    server.stop(true);
    process.exit(0);
  });
}
