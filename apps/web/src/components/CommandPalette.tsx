import { CornerDownLeft, KeyRound, Monitor, Moon, Search, Settings, Sun, User as UserIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { cn } from "~/lib/cn";
import {
  channelTitle,
  isContainer,
  isMailbox,
  keyString,
  mailboxName,
  sameKey,
  sortedOrganizations,
  type ChannelKey,
} from "~/store/directory";
import { show, useStore } from "~/store/store";
import { setTheme, useUi } from "~/store/ui";
import { channelIcon } from "./Sidebar";
import { Kbd } from "./ui";

interface Entry {
  id: string;
  label: string;
  detail: string;
  icon: ReactNode;
  run: () => void;
  /** Lower sorts first when the query is empty. */
  rank: number;
}

/** Scores a label against a query: prefix beats word start beats substring beats scattered letters. */
export function score(label: string, query: string): number {
  const l = label.toLowerCase();
  const q = query.toLowerCase().trim();
  if (!q) return 1;
  if (l.startsWith(q)) return 100 - l.length / 100;
  const word = l.search(new RegExp(`(^|[\\s#@/_.-])${q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`));
  if (word >= 0) return 80 - word / 100;
  const at = l.indexOf(q);
  if (at >= 0) return 60 - at / 100;
  let i = 0;
  for (const c of l) if (c === q[i]) i++;
  return i === q.length ? 20 - l.length / 100 : 0;
}

export function CommandPalette() {
  const open = useUi((s) => s.paletteOpen);
  if (!open) return null;
  return <Palette onClose={() => useUi.setState({ paletteOpen: false })} />;
}

function Palette({ onClose }: { onClose: () => void }) {
  const providers = useStore((s) => s.providers);
  const recent = useStore((s) => s.recent);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  const entries = useMemo(() => {
    const result: Entry[] = [];
    const openKey = (key: ChannelKey) => () => {
      useUi.setState({ providerPage: null, sidebarOpen: false });
      show(key);
    };
    for (const p of Object.values(providers)) {
      const name = p.id.name;
      const scopes = [
        { org: null as string | null, orgName: null as string | null, scope: p.standalone },
        ...sortedOrganizations(p).map((o) => ({ org: o.id, orgName: o.name, scope: o.scope })),
      ];
      for (const { org, orgName, scope } of scopes) {
        for (const channel of Object.values(scope.channels)) {
          if (isContainer(channel)) continue;
          const key = { provider: name, channel: { organization: org, channel: channel.id } };
          const Icon = channelIcon(channel);
          const recentIndex = recent.findIndex((r) => sameKey(r, key));
          result.push({
            id: keyString(key),
            label: isMailbox(channel) ? mailboxName(channel) : channelTitle(scope, channel),
            detail: [name, orgName].filter(Boolean).join(" · "),
            icon: <Icon className="size-4" />,
            run: openKey(key),
            rank: recentIndex >= 0 ? recentIndex : 1000,
          });
        }
      }
      result.push({
        id: `account:${name}`,
        label: name,
        detail: "Account",
        icon: "LoginRequired" in (typeof p.status === "object" ? p.status : {}) ? <KeyRound className="size-4" /> : <UserIcon className="size-4" />,
        run: () => {
          useUi.setState({ providerPage: name, sidebarOpen: false });
          show(null);
        },
        rank: 2000,
      });
    }
    const commands: [string, ReactNode, () => void][] = [
      ["Settings", <Settings key="s" className="size-4" />, () => useUi.setState({ settings: "accounts" })],
      ["Paired devices", <Monitor key="d" className="size-4" />, () => useUi.setState({ settings: "devices" })],
      ["Theme: light", <Sun key="l" className="size-4" />, () => setTheme("light")],
      ["Theme: dark", <Moon key="k" className="size-4" />, () => setTheme("dark")],
      ["Theme: system", <Monitor key="y" className="size-4" />, () => setTheme("system")],
    ];
    commands.forEach(([label, icon, run], i) => result.push({ id: `cmd:${label}`, label, detail: "Command", icon, run, rank: 3000 + i }));
    return result;
  }, [providers, recent]);

  const results = useMemo(() => {
    if (!query.trim()) return [...entries].sort((a, b) => a.rank - b.rank || a.label.localeCompare(b.label)).slice(0, 60);
    return entries
      .map((e) => ({ e, s: Math.max(score(e.label, query), score(`${e.detail} ${e.label}`, query) * 0.9) }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s || a.e.rank - b.e.rank)
      .slice(0, 60)
      .map((x) => x.e);
  }, [entries, query]);

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const choose = (entry: Entry | undefined) => {
    if (!entry) return;
    onClose();
    entry.run();
  };

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 p-4 pt-[14vh] animate-fade-in" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="w-full max-w-xl overflow-hidden rounded-xl border border-border bg-popover shadow-2xl animate-pop-in">
        <div className="flex items-center gap-2.5 border-b border-border px-4">
          <Search className="size-4 text-muted-foreground" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              else if (e.key === "ArrowDown") {
                e.preventDefault();
                setActive((a) => Math.min(a + 1, results.length - 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setActive((a) => Math.max(a - 1, 0));
              } else if (e.key === "Enter") {
                e.preventDefault();
                choose(results[active]);
              }
            }}
            placeholder="Jump to a conversation, account or command…"
            className="h-12 flex-1 bg-transparent text-[14px] outline-none placeholder:text-muted-foreground/70"
          />
          <Kbd>Esc</Kbd>
        </div>
        <div ref={list} className="max-h-[50vh] overflow-y-auto p-1.5">
          {results.length === 0 ? <div className="px-3 py-8 text-center text-[13px] text-muted-foreground">No matches</div> : null}
          {results.map((entry, i) => (
            <button
              key={entry.id}
              data-index={i}
              onMouseMove={() => setActive(i)}
              onClick={() => choose(entry)}
              className={cn("flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left", i === active && "bg-accent")}
            >
              <span className="text-muted-foreground">{entry.icon}</span>
              <span className="min-w-0 flex-1 truncate text-[13px]">{entry.label}</span>
              <span className="truncate text-xs text-muted-foreground">{entry.detail}</span>
              {i === active ? <CornerDownLeft className="size-3.5 text-muted-foreground" /> : null}
            </button>
          ))}
        </div>
      </div>
    </div>,
    document.body,
  );
}
