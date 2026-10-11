import type { BrowserContextOptions } from "playwright";

/** Host-owned recipes. A provider selects a name; it cannot run browser scripts. */
export interface LoginDefaults {
  url: string;
  context?: BrowserContextOptions;
  tokenResponse: { origin: string; path: RegExp; field: string };
  /** QR login returns encrypted credentials; capture a successful signed-in request. */
  authenticatedResponse?: { origin: string; path: RegExp };
}

export const providers: Record<string, LoginDefaults> = {
  discord: {
    url: "https://discord.com/login",
    context: { locale: "en-US", viewport: { width: 1024, height: 768 } },
    authenticatedResponse: { origin: "https://discord.com", path: /^\/api\/v\d+\/users\/@me(?:\/|$)/ },
    tokenResponse: {
      origin: "https://discord.com",
      path: /^\/api\/v\d+\/auth\/(login|mfa\/(totp|sms))$/,
      field: "token",
    },
  },
};
