import { describe, expect, it } from "vitest";
import type { Channel } from "~/api/types";
import { applyDirectory, channelTitle, channelTree, mailboxName, mailboxes, newProvider } from "~/store/directory";

const channel = (id: string, extra: Partial<Channel> = {}): Channel => ({
  id,
  name: id,
  kind: "Text",
  accepted_content: [],
  limits: {},
  ...extra,
});

describe("directory", () => {
  it("applies organization and scope updates, ignoring unknown organizations", () => {
    let p = newProvider({ name: "d", instance: 1 });
    p = applyDirectory(p, { OrganizationUpserted: { id: "o", name: "Org", me: "u", users: [{ id: "u", name: "U" }], channels: [channel("c")] } });
    p = applyDirectory(p, { ChannelUpserted: { organization: "missing", channel: channel("x") } });
    p = applyDirectory(p, { Presence: { organization: "o", user: "u", presence: "Online" } });
    p = applyDirectory(p, { UserRemoved: { organization: "o", id: "u" } });
    expect(Object.keys(p.organizations)).toEqual(["o"]);
    expect(p.organizations.o!.scope.me).toBeNull();
    expect(p.organizations.o!.scope.presence).toEqual({});
    p = applyDirectory(p, "Reset");
    expect(p.organizations).toEqual({});
  });

  it("nests channels under their parents in position order", () => {
    const scope = {
      users: {},
      presence: {},
      channels: {
        cat: channel("cat", { kind: "Category", position: 1 }),
        b: channel("b", { parent: "cat", position: 2 }),
        a: channel("a", { parent: "cat", position: 1 }),
        top: channel("top", { position: 0 }),
        orphan: channel("orphan", { parent: "gone", position: 5 }),
      },
    };
    expect(channelTree(scope).map((n) => `${n.channel.id}:${n.depth}`)).toEqual(["top:0", "cat:0", "a:1", "b:1", "orphan:0"]);
  });

  it("names direct conversations after the other members", () => {
    const scope = {
      me: "me",
      users: { me: { id: "me", name: "Me" }, ada: { id: "ada", name: "Ada" }, x: { id: "x", name: "x", username: "xu" } },
      presence: {},
      channels: {},
    };
    expect(channelTitle(scope, channel("d", { kind: "Direct", name: "", members: ["me", "ada"] }))).toBe("Ada");
    expect(channelTitle(scope, channel("g", { kind: "Group", name: "", members: ["me", "ada", "x"] }))).toBe("Ada, xu");
  });

  it("orders mailboxes with special folders first", () => {
    const mb = (id: string, attributes: string[] = []) => channel(id, { kind: { Custom: "mailbox" }, mailbox: { delimiter: "/", attributes } });
    const scope = {
      users: {},
      presence: {},
      channels: { a: mb("Archive/2020"), t: mb("[Gmail]/Trash", ["\\Trash"]), i: mb("INBOX"), s: mb("[Gmail]/Sent Mail", ["\\Sent"]) },
    };
    expect(mailboxes(scope).map(mailboxName)).toEqual(["Inbox", "Sent Mail", "Trash", "2020"]);
  });
});
