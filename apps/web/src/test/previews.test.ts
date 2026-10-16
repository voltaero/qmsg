import { expect, it } from "vitest";
import type { Message } from "~/api/types";
import { plainText, textOf } from "~/lib/content";
const message: Message = { id: "1", channel: { organization: null, channel: "c" }, author: "a", sent_at: 1, content: [{ Text: "before ||secret **bold**|| after `||literal||`" }], reactions: [] };

it("hides Discord spoilers in display summaries without changing editing text", () => {
  expect(plainText(message, "discord")).toBe("before [spoiler] after ||literal||");
  expect(textOf(message.content[0]!)).toBe("before ||secret **bold**|| after `||literal||`");
  expect(plainText(message, "echo")).toContain("||secret **bold**||");
});
