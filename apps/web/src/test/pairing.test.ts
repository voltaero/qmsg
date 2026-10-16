import { describe, expect, it } from "vitest";
import { parsePairing } from "~/api/account";

const payload = (endpoint: string, extra: object = {}) =>
  JSON.stringify({ version: 1, endpoint, code: "c", expires_at_ms: 2000, ...extra });

describe("pairing", () => {
  it("accepts HTTPS anywhere and HTTP only on loopback", () => {
    expect(parsePairing(payload("https://qmsg.local:50051"), 1000).endpoint).toBe("https://qmsg.local:50051");
    expect(parsePairing(payload("http://127.0.0.1:50051"), 1000).code).toBe("c");
    expect(parsePairing(payload("http://[::1]:50051"), 1000).code).toBe("c");
    expect(() => parsePairing(payload("http://192.168.1.2:50051"), 1000)).toThrow(/HTTPS/);
  });

  it("refuses expired, newer and malformed codes", () => {
    expect(() => parsePairing(payload("https://h"), 3000)).toThrow(/expired/);
    expect(() => parsePairing(payload("https://h", { version: 2 }), 1000)).toThrow(/newer/);
    expect(() => parsePairing(payload("https://h/path"), 1000)).toThrow(/isn't valid/);
    expect(() => parsePairing("not json", 1000)).toThrow(/pairing code/);
  });
});
