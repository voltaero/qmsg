import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { MessageBody } from "~/components/chat/MessageList";

vi.hoisted(() => {
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {} }));
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  });
});

function render(text: string, providerType = "discord") {
  return renderToStaticMarkup(<MessageBody content={[{ Text: text }]} scope={undefined} {...{ providerType }} />);
}

describe("Discord message rendering", () => {
  it("renders Markdown from the provider's plain Text content", () => {
    const html = render("**bold** *italic* __underline__ ~~strike~~\n`const inline = 42;`\n```ts\nconst greeting = \"hi\";\n```\n> quoted text\n[Example link](https://example.com)\n- first item\n- second item");
    for (const tag of ["strong", "em", "u", "s", "code", "blockquote", "ul", "li"]) {
      expect(html).toContain(`<${tag}`);
    }
    expect(html).toContain('href="https://example.com/"');
    expect(html).not.toContain("**bold**");
    expect(html).not.toContain("```ts");
  });

  it("hides spoilers and gives keyboard users a reveal control", () => {
    const html = render("before ||secret **bold**|| after");
    expect(html).toContain('aria-label="Reveal spoiler"');
    expect(html).toContain('aria-hidden="true"');
    expect(html).not.toContain("||secret");
  });

  it("keeps code literal and never turns raw HTML or unsafe URLs into markup", () => {
    const html = render('`**literal**` <script>alert(1)</script> [bad](javascript:alert(1))');
    expect(html).toContain("**literal**");
    expect(html).not.toContain("<script>");
    expect(html).not.toContain('href="javascript:');
    expect(html).not.toContain("<strong>");
  });

  it("keeps other providers' plain Text literal", () => {
    expect(render("**literal** ||literal||", "echo")).toContain("**literal** ||literal||");
  });
});
