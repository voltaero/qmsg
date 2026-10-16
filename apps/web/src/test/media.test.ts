import { expect, it } from "vitest";
import type { Media } from "~/api/types";
import { mediaUrl } from "~/store/media";

it("keeps inline files with the same header and size distinct", async () => {
  const media = (last: number, mime = "application/octet-stream"): Media => ({
    mime, source: { Bytes: [...Array(32).fill(65), last] },
  });
  const first = await mediaUrl(media(66));
  const second = await mediaUrl(media(67));
  expect(await (await fetch(second)).text()).toBe("A".repeat(32) + "C");
  expect(first).not.toBe(second);
  const otherMime = await mediaUrl(media(66, "text/plain"));
  expect((await fetch(otherMime)).headers.get("content-type")).toBe("text/plain");
});
