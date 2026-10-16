import { Command, Menu as MenuIcon, MessagesSquare } from "lucide-react";
import { useEffect, useRef } from "react";
import { cn } from "~/lib/cn";
import { formatStamp, plainText } from "~/lib/content";
import { channelOf, channelTitle, keyString, scopeOf, type ChannelKey } from "~/store/directory";
import { resume, show, suspend, useStore } from "~/store/store";
import { applyTheme, useUi } from "~/store/ui";
import { Avatar } from "./components/Avatar";
import { ChatView } from "./components/chat/ChatView";
import { CommandPalette } from "./components/CommandPalette";
import { Pairing } from "./components/Pairing";
import { OAuthCallback, ProviderPage } from "./components/ProviderPage";
import { SettingsDialog } from "./components/Settings";
import { Sidebar } from "./components/Sidebar";
import { Button, EmptyState, Kbd, Toaster } from "./components/ui";

// The open conversation lives in the URL hash, so reloads and the back
// button keep it: #/c/<provider>/<organization or ~>/<channel>.
function keyFromHash(): ChannelKey | null {
  const match = /^#\/c\/([^/]+)\/([^/]+)\/(.+)$/.exec(location.hash);
  if (!match) return null;
  const [, provider, org, channel] = match.map((p) => decodeURIComponent(p ?? ""));
  return { provider: provider!, channel: { organization: org === "~" ? null : org!, channel: channel! } };
}

function hashFor(key: ChannelKey | null): string {
  if (!key) return "#/";
  const e = encodeURIComponent;
  return `#/c/${e(key.provider)}/${key.channel.organization == null ? "~" : e(key.channel.organization)}/${e(key.channel.channel)}`;
}

function useHashRouting() {
  const visible = useStore((s) => s.visible);
  const live = useStore((s) => s.link.kind === "live");
  useEffect(() => {
    const onHash = () => {
      const key = keyFromHash();
      if (keyString(key ?? { provider: "", channel: { organization: null, channel: "" } }) !== keyString(useStore.getState().visible ?? { provider: "", channel: { organization: null, channel: "" } })) {
        useUi.setState({ providerPage: null });
        show(key);
      }
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);
  // Open the linked conversation once the directory has arrived.
  useEffect(() => {
    if (live && !useStore.getState().visible) {
      const key = keyFromHash();
      if (key) show(key);
    }
  }, [live]);
  const shown = useRef(false);
  useEffect(() => {
    // Before anything was shown, keep the linked hash for the effect above.
    if (!visible && !shown.current) return;
    shown.current = true;
    const hash = hashFor(visible);
    if (location.hash !== hash) history.pushState(null, "", hash);
  }, [visible]);
}

function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        useUi.setState((s) => ({ paletteOpen: !s.paletteOpen }));
      } else if (mod && e.key === ",") {
        e.preventDefault();
        useUi.setState({ settings: "accounts" });
      } else if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        // Step through recent conversations.
        const { recent, visible } = useStore.getState();
        if (!recent.length) return;
        e.preventDefault();
        const i = visible ? recent.findIndex((r) => keyString(r) === keyString(visible)) : -1;
        const next = recent[Math.min(Math.max(i + (e.key === "ArrowDown" ? 1 : -1), 0), recent.length - 1)];
        if (next) show(next);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** Keeps the stream running only while the page is shown, and the unread count in the title. */
function useLifecycle() {
  useEffect(() => {
    applyTheme();
    resume();
    const onShow = () => {
      if (document.visibilityState === "visible") {
        resume();
        const visible = useStore.getState().visible;
        if (visible) show(visible);
      }
    };
    const onOnline = () => {
      suspend();
      resume();
    };
    document.addEventListener("visibilitychange", onShow);
    window.addEventListener("online", onOnline);
    return () => {
      document.removeEventListener("visibilitychange", onShow);
      window.removeEventListener("online", onOnline);
    };
  }, []);
  const unread = useStore((s) => Object.values(s.activity).reduce((n, a) => n + a.unread, 0));
  useEffect(() => {
    const base = document.title.replace(/^\(\d+\+?\) /, "");
    document.title = unread ? `(${unread > 99 ? "99+" : unread}) ${base}` : base;
  }, [unread]);
}

export function App() {
  if (location.pathname === "/oauth/callback") return <OAuthCallback />;
  return <Main />;
}

function Main() {
  const account = useStore((s) => s.account);
  const demo = useStore((s) => s.demo);
  const needsPairing = useStore((s) => s.needsPairing);
  useLifecycle();
  if ((!account && !demo) || needsPairing) {
    return (
      <>
        <Pairing />
        <Toaster />
      </>
    );
  }
  return <Shell />;
}

function Shell() {
  const visible = useStore((s) => s.visible);
  const page = useUi((s) => s.providerPage);
  const sidebarOpen = useUi((s) => s.sidebarOpen);
  useHashRouting();
  useShortcuts();
  return (
    <div className="flex h-full">
      <div
        className={cn(
          "fixed inset-y-0 left-0 z-30 w-72 border-r border-sidebar-border transition-transform duration-300 ease-drawer md:static md:z-auto md:w-64 md:translate-x-0 lg:w-72",
          sidebarOpen ? "translate-x-0 shadow-2xl" : "-translate-x-full",
        )}
      >
        <Sidebar />
      </div>
      {sidebarOpen ? <div className="fixed inset-0 z-20 bg-black/40 md:hidden" onClick={() => useUi.setState({ sidebarOpen: false })} /> : null}
      <main className="min-w-0 flex-1 bg-background">
        {page ? <ProviderPage name={page} /> : visible ? <ChatView key={keyString(visible)} channelKey={visible} /> : <Home />}
      </main>
      <CommandPalette />
      <SettingsDialog />
      <Toaster />
    </div>
  );
}

function Home() {
  const recent = useStore((s) => s.recent);
  const types = useStore((s) => s.types);
  const providers = useStore((s) => s.providers);
  const activity = useStore((s) => s.activity);
  const chats = recent
    .map((key) => {
      const p = providers[key.provider];
      const channel = channelOf(p, key.channel);
      const scope = scopeOf(p, key.channel.organization);
      return channel && scope ? { key, channel, scope, last: activity[keyString(key)]?.last } : null;
    })
    .filter((x) => !!x)
    .slice(0, 8);
  return (
    <div className="flex h-full flex-col">
      <header className="flex h-13 shrink-0 items-center px-3 md:hidden">
        <Button variant="ghost" size="icon-sm" onClick={() => useUi.setState({ sidebarOpen: true })} aria-label="Open sidebar">
          <MenuIcon />
        </Button>
      </header>
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-6">
        <div className="w-full max-w-md">
          <EmptyState icon={<MessagesSquare />} title="Pick up where you left off">
            Choose a conversation in the sidebar, or press <Kbd className="mx-0.5"><Command className="size-2.5" />K</Kbd> to jump anywhere.
          </EmptyState>
          {chats.length ? (
            <div className="mt-2 divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
              {chats.map(({ key, channel, scope, last }) => {
                const title = channelTitle(scope, channel);
                return (
                  <button key={keyString(key)} onClick={() => show(key)} className="flex w-full items-center gap-3 px-3 py-2.5 text-left hover:bg-accent">
                    <Avatar name={title} seed={channel.id} media={channel.icon} size={32} square={channel.kind !== "Direct"} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px] font-medium">{title}</span>
                      <span className="block truncate text-xs text-muted-foreground">{last ? plainText(last, types[key.provider]) : key.provider}</span>
                    </span>
                    {last ? <span className="text-2xs text-muted-foreground">{formatStamp(last.sent_at)}</span> : null}
                  </button>
                );
              })}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
