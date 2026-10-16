import { Bell, LogOut, Monitor, Moon, Palette, Plus, Smartphone, Sun, Trash2, Users, X } from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { describe } from "~/api/errors";
import type { ManagedProvider, PairedDevice, ProviderCatalog, ProviderType } from "~/api/types";
import { cn } from "~/lib/cn";
import { formatLong } from "~/lib/content";
import { statusLabel } from "~/store/directory";
import { configureProvider, devices, providerCatalog, removeProvider, revoke, unpair, useStore } from "~/store/store";
import { setTheme, toast, useUi, type Theme } from "~/store/ui";
import { Button, Dialog, Input, Label, Spinner, Switch } from "./ui";

type Tab = NonNullable<ReturnType<typeof useUi.getState>["settings"]>;

const tabs: { id: Tab; label: string; icon: ReactNode }[] = [
  { id: "accounts", label: "Accounts", icon: <Users /> },
  { id: "devices", label: "Devices", icon: <Smartphone /> },
  { id: "appearance", label: "Appearance", icon: <Palette /> },
];

export function SettingsDialog() {
  const tab = useUi((s) => s.settings);
  useEffect(() => {
    if (!tab) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && useUi.setState({ settings: null });
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab]);
  if (!tab) return null;
  const close = () => useUi.setState({ settings: null });
  return createPortal(
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/40 p-4 animate-fade-in" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="flex h-[min(640px,90vh)] w-full max-w-3xl overflow-hidden rounded-xl border border-border bg-popover shadow-2xl animate-pop-in">
        <nav className="hidden w-48 shrink-0 flex-col gap-0.5 border-r border-border bg-sidebar p-2 sm:flex">
          <div className="px-2 pt-2 pb-3 text-[13px] font-semibold">Settings</div>
          {tabs.map((t) => (
            <button
              key={t.id}
              onClick={() => useUi.setState({ settings: t.id })}
              className={cn(
                "flex h-8 items-center gap-2 rounded-lg px-2 text-[13px] [&_svg]:size-4",
                tab === t.id ? "bg-sidebar-row-active font-medium shadow-[0_0_0_1px_var(--border)]" : "text-sidebar-muted-foreground hover:bg-sidebar-row-hover",
              )}
            >
              {t.icon}
              {t.label}
            </button>
          ))}
        </nav>
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-13 shrink-0 items-center gap-2 border-b border-border px-5">
            <select
              value={tab}
              onChange={(e) => useUi.setState({ settings: e.target.value as Tab })}
              className="rounded-md bg-transparent text-[14px] font-semibold sm:hidden"
            >
              {tabs.map((t) => (
                <option key={t.id} value={t.id}>{t.label}</option>
              ))}
            </select>
            <h2 className="hidden text-[14px] font-semibold sm:block">{tabs.find((t) => t.id === tab)?.label}</h2>
            <div className="flex-1" />
            <Button variant="ghost" size="icon-sm" onClick={close} aria-label="Close">
              <X />
            </Button>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto p-5">
            {tab === "accounts" ? <Accounts /> : tab === "devices" ? <Devices /> : <Appearance />}
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function Section({ title, description, children, action }: { title: string; description?: string; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="mb-7">
      <div className="mb-2.5 flex items-end gap-2">
        <div className="flex-1">
          <h3 className="text-[13px] font-semibold">{title}</h3>
          {description ? <p className="mt-0.5 text-xs text-muted-foreground">{description}</p> : null}
        </div>
        {action}
      </div>
      {children}
    </section>
  );
}

function Accounts() {
  const providers = useStore((s) => s.providers);
  const [catalog, setCatalog] = useState<ProviderCatalog | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ account?: ManagedProvider; type?: ProviderType } | null>(null);
  const load = useCallback(() => {
    providerCatalog()
      .then((c) => {
        setCatalog(c);
        setError(null);
      })
      .catch((e) => setError(describe(e)));
  }, []);
  useEffect(load, [load]);

  if (error) {
    return <p className="rounded-lg bg-muted px-3 py-2 text-[13px] text-muted-foreground">{error}</p>;
  }
  if (!catalog) return <Spinner />;
  return (
    <>
      <Section
        title="Chat accounts"
        description="Accounts run on your qmsg server. Turning one off stops it and keeps its sign-in."
        action={
          catalog.types.length ? (
            <Button size="sm" variant="outline" onClick={() => setEditing({})}>
              <Plus /> Add account
            </Button>
          ) : null
        }
      >
        <div className="divide-y divide-border rounded-xl border border-border">
          {catalog.accounts.length === 0 ? <div className="px-4 py-6 text-center text-[13px] text-muted-foreground">No accounts yet.</div> : null}
          {catalog.accounts.map((account) => {
            const provider = providers[account.name];
            const type = catalog.types.find((t) => t.id === account.type);
            return (
              <div key={account.name} className="flex items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline gap-2">
                    <span className="truncate text-[13px] font-medium">{account.label || account.name}</span>
                    <span className="text-xs text-muted-foreground">{type?.label ?? account.type}</span>
                  </div>
                  <div className="truncate text-xs text-muted-foreground">
                    {account.enabled ? (provider ? statusLabel(provider.status) : "Starting…") : "Off"}
                  </div>
                </div>
                <Button size="sm" variant="ghost" onClick={() => setEditing({ account, type })} disabled={!type}>
                  Edit
                </Button>
                <Switch
                  checked={account.enabled}
                  onChange={(enabled) =>
                    configureProvider({ name: account.name, type: account.type, label: account.label, enabled, settings: account.settings })
                      .then(load)
                      .catch((e) => toast(describe(e), "error"))
                  }
                />
              </div>
            );
          })}
        </div>
      </Section>
      {editing ? <AccountForm catalog={catalog} {...editing} onClose={() => setEditing(null)} onSaved={load} /> : null}
    </>
  );
}

function AccountForm({
  catalog,
  account,
  type: initialType,
  onClose,
  onSaved,
}: {
  catalog: ProviderCatalog;
  account?: ManagedProvider;
  type?: ProviderType;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [typeId, setTypeId] = useState(initialType?.id ?? catalog.types[0]?.id ?? "");
  const type = catalog.types.find((t) => t.id === typeId);
  const [name, setName] = useState(account?.name ?? "");
  const [label, setLabel] = useState(account?.label ?? "");
  const [settings, setSettings] = useState<Record<string, string>>(account?.settings ?? {});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    if (!type) return;
    setBusy(true);
    setError(null);
    try {
      const values = { ...type.defaults, ...settings };
      for (const k of Object.keys(values)) if (values[k] === "") delete values[k];
      await configureProvider({ name: name.trim(), type: type.id, label: label.trim(), enabled: account?.enabled ?? true, settings: values });
      onSaved();
      onClose();
    } catch (e) {
      setError(describe(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!account || !confirm(`Remove ${account.label || account.name}? Its saved sign-in on this server is deleted. Nothing on the chat network is.`)) return;
    try {
      await removeProvider(account.name);
      onSaved();
      onClose();
    } catch (e) {
      setError(describe(e));
    }
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title={account ? `Edit ${account.label || account.name}` : "Add account"}
      description="Sign-in happens on the account's page once it starts."
      footer={
        <>
          {account ? (
            <Button variant="destructive" className="mr-auto" onClick={() => void remove()}>
              <Trash2 /> Remove
            </Button>
          ) : null}
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" loading={busy} disabled={!name.trim() || !type} onClick={() => void save()}>
            Save
          </Button>
        </>
      }
    >
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        {!account ? (
          <div>
            <Label>Type</Label>
            <div className="flex flex-wrap gap-1.5">
              {catalog.types.map((t) => (
                <button
                  key={t.id}
                  type="button"
                  onClick={() => setTypeId(t.id)}
                  className={cn("h-8 rounded-lg border px-3 text-[13px]", t.id === typeId ? "border-primary bg-primary/12 text-primary" : "border-input hover:bg-accent")}
                >
                  {t.label}
                </button>
              ))}
            </div>
          </div>
        ) : null}
        <div>
          <Label htmlFor="acct-name" hint="Stable id; can't change later">Name</Label>
          <Input id="acct-name" value={name} disabled={!!account} onChange={(e) => setName(e.target.value)} placeholder={type?.id ?? "account"} />
        </div>
        <div>
          <Label htmlFor="acct-label" hint="Optional">Label</Label>
          <Input id="acct-label" value={label} onChange={(e) => setLabel(e.target.value)} placeholder={type?.label} />
        </div>
        {type?.fields.map((f) => (
          <div key={f.id}>
            <Label htmlFor={`acct-${f.id}`} hint={f.required ? undefined : "Optional"}>{f.label}</Label>
            <Input
              id={`acct-${f.id}`}
              type={f.secret ? "password" : "text"}
              value={settings[f.id] ?? ""}
              placeholder={type.defaults[f.id]}
              onChange={(e) => setSettings((s) => ({ ...s, [f.id]: e.target.value }))}
            />
          </div>
        ))}
        {error ? <p className="rounded-lg bg-destructive/10 px-3 py-2 text-[13px] text-destructive">{error}</p> : null}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}

function Devices() {
  const account = useStore((s) => s.account);
  const demo = useStore((s) => s.demo);
  const [list, setList] = useState<PairedDevice[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    devices().then(setList).catch((e) => setError(describe(e)));
  }, []);
  useEffect(load, [load]);
  return (
    <>
      <Section title="Paired devices" description="Every paired device has full access to this server. Revoke one you no longer use.">
        {error ? <p className="text-[13px] text-destructive">{error}</p> : null}
        {!list && !error ? <Spinner /> : null}
        {list ? (
          <div className="divide-y divide-border rounded-xl border border-border">
            {list.map((d) => {
              const self = d.id === account?.deviceId || (demo && d.id === "demo");
              return (
                <div key={d.id} className="flex items-center gap-3 px-4 py-3">
                  <Monitor className="size-4 text-muted-foreground" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13px] font-medium">
                      {d.name} {self ? <span className="ml-1 rounded bg-primary/12 px-1.5 py-0.5 text-2xs text-primary">This browser</span> : null}
                    </div>
                    <div className="text-xs text-muted-foreground">Expires {formatLong(d.expiresAtMs)}</div>
                  </div>
                  <Button
                    size="sm"
                    variant="destructive"
                    onClick={() => {
                      if (!confirm(self ? "Revoke this browser? You'll need to pair it again." : `Revoke ${d.name}?`)) return;
                      revoke(d.id).then(load).catch((e) => toast(describe(e), "error"));
                    }}
                  >
                    Revoke
                  </Button>
                </div>
              );
            })}
          </div>
        ) : null}
      </Section>
      <Section title="This browser" description={demo ? "You're trying the offline demo." : account ? `Paired with ${account.endpoint}.` : undefined}>
        <Button variant="outline" onClick={() => void unpair().then(() => useUi.setState({ settings: null }))}>
          <LogOut /> {demo ? "Leave demo" : "Unpair this browser"}
        </Button>
        {!demo ? (
          <p className="mt-2 text-xs text-muted-foreground">
            The device token is kept in this site's local storage. Unpairing revokes it on the server.
          </p>
        ) : null}
      </Section>
    </>
  );
}

function Appearance() {
  const theme = useUi((s) => s.theme);
  const [notifications, setNotifications] = useState(localStorage.getItem("qmsg.notifications") === "on");
  const options: { id: Theme; label: string; icon: ReactNode }[] = [
    { id: "system", label: "System", icon: <Monitor /> },
    { id: "light", label: "Light", icon: <Sun /> },
    { id: "dark", label: "Dark", icon: <Moon /> },
  ];
  return (
    <>
      <Section title="Theme">
        <div className="grid grid-cols-3 gap-2">
          {options.map((o) => (
            <button
              key={o.id}
              onClick={() => setTheme(o.id)}
              className={cn(
                "flex h-20 flex-col items-center justify-center gap-2 rounded-xl border text-[13px] [&_svg]:size-5",
                theme === o.id ? "border-primary bg-primary/8 text-primary" : "border-border hover:bg-accent",
              )}
            >
              {o.icon}
              {o.label}
            </button>
          ))}
        </div>
      </Section>
      <Section title="Notifications" description="Show a notification for new messages while this tab is in the background.">
        <div className="flex items-center gap-3 rounded-xl border border-border px-4 py-3">
          <Bell className="size-4 text-muted-foreground" />
          <span className="flex-1 text-[13px]">Desktop notifications</span>
          <Switch
            checked={notifications}
            onChange={async (on) => {
              if (on && typeof Notification !== "undefined" && Notification.permission !== "granted") {
                const permission = await Notification.requestPermission();
                if (permission !== "granted") return toast("Notifications are blocked for this site.", "error");
              }
              localStorage.setItem("qmsg.notifications", on ? "on" : "off");
              setNotifications(on);
            }}
          />
        </div>
      </Section>
    </>
  );
}
