import { describe, expect, it } from "vitest";
import type { Message } from "~/api/types";
import * as conv from "~/store/conversation";

const ref = { organization: null, channel: "c" };
const msg = (id: string, at: number, extra: Partial<Message> = {}): Message => ({
  id,
  channel: ref,
  author: "a",
  sent_at: at,
  content: [{ Text: id }],
  reactions: [],
  ...extra,
});
const ids = (c: conv.Conversation) => c.messages.map((m) => m.id);

describe("conversation", () => {
  it("keeps older history when the newest page joins up with it", () => {
    let c = conv.replace(conv.beginLoad(conv.emptyConversation()), { messages: [msg("3", 3), msg("4", 4)], next_cursor: "x" });
    c = conv.prepend(c, { messages: [msg("1", 1), msg("2", 2)], next_cursor: null }, c.generation);
    expect(ids(c)).toEqual(["1", "2", "3", "4"]);
    c = conv.replace(conv.beginLoad(c), { messages: [msg("4", 4), msg("5", 5)], next_cursor: "y" });
    expect(ids(c)).toEqual(["1", "2", "3", "4", "5"]);
    expect(c.reachedStart).toBe(true);
  });

  it("drops history that doesn't join up, and older pages asked for it", () => {
    let c = conv.replace(conv.emptyConversation(), { messages: [msg("1", 1)], next_cursor: "x" });
    const generation = c.generation;
    c = conv.replace(conv.beginLoad(c), { messages: [msg("9", 9)], next_cursor: "z" });
    expect(ids(c)).toEqual(["9"]);
    expect(conv.prepend(c, { messages: [msg("0", 0)], next_cursor: null }, generation)).toBe(c);
  });

  it("lets events during a load win over the page", () => {
    let c = conv.replace(conv.emptyConversation(), { messages: [msg("1", 1), msg("2", 2)], next_cursor: null });
    c = conv.beginLoad(c);
    c = conv.applyEvent(c, { Deleted: { channel: ref, id: "2" } }, "me");
    c = conv.applyEvent(c, { Edited: { channel: ref, id: "1", content: [{ Text: "new" }], edited_at: 5 } }, "me");
    c = conv.replace(c, { messages: [msg("1", 1), msg("2", 2)], next_cursor: null });
    expect(ids(c)).toEqual(["1"]);
    expect(c.messages[0]!.content).toEqual([{ Text: "new" }]);
  });

  it("settles pending sends by nonce and by id", () => {
    let c = conv.replace(conv.emptyConversation(), { messages: [], next_cursor: null });
    const pending = (id: string): conv.PendingSend => ({
      id,
      content: [],
      replyTo: null,
      operation: { id, createdAtMs: 0 },
      state: { kind: "sending" },
      previews: {},
      files: [],
    });
    c = { ...c, pending: [pending("n1"), pending("n2")] };
    c = conv.markSent(c, "n2", "m2");
    c = conv.applyEvent(c, { Received: msg("m1", 1, { nonce: "n1" }) }, "me");
    expect(c.pending.map((p) => p.id)).toEqual(["n2"]);
    c = conv.applyEvent(c, { Received: msg("m2", 2) }, "me");
    expect(c.pending).toEqual([]);
  });

  it("counts reactions and tracks the account's own", () => {
    let c = conv.replace(conv.emptyConversation(), { messages: [msg("1", 1)], next_cursor: null });
    c = conv.applyEvent(c, { Reacted: { channel: ref, id: "1", user: "me", key: "👍", added: true } }, "me");
    c = conv.applyEvent(c, { Reacted: { channel: ref, id: "1", user: "b", key: "👍", added: true } }, "me");
    expect(c.messages[0]!.reactions).toEqual([{ key: "👍", count: 2, me: true }]);
    c = conv.applyEvent(c, { Reacted: { channel: ref, id: "1", user: "me", key: "👍", added: false } }, "me");
    c = conv.applyEvent(c, { Reacted: { channel: ref, id: "1", user: "b", key: "👍", added: false } }, "me");
    expect(c.messages[0]!.reactions).toEqual([]);
  });

  it("orders mail with equal times by UID", () => {
    const email = { subject: "", from: [], to: [], cc: [], bcc: [], reply_to: [], references: [], content_ids: {}, read: true, starred: false, draft: false };
    const c = conv.merge(conv.emptyConversation(), [msg("1:10", 5, { email }), msg("1:9", 5, { email })]);
    expect(ids(c)).toEqual(["1:9", "1:10"]);
  });

  it("forgets typing after the timeout, and when the user sends", () => {
    let c = conv.replace(conv.emptyConversation(), { messages: [], next_cursor: null });
    c = conv.applyEvent(c, { Typing: { channel: ref, user: "b" } }, "me", 1000);
    expect(conv.typingUsers(c, 2000)).toEqual(["b"]);
    expect(conv.typingUsers(c, 20_000)).toEqual([]);
    c = conv.applyEvent(c, { Received: msg("x", 3, { author: "b" }) }, "me");
    expect(c.typing).toEqual({});
  });
});
