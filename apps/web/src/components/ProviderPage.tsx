import { ExternalLink, KeyRound, LogOut, Menu as MenuIcon, MessageSquarePlus, RefreshCw, Sparkles } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { renderSVG } from "uqr";
import { describe } from "~/api/errors";
import { actionOf, tag, type FunctionContext, type LoginStep, type QFunction, type Value } from "~/api/types";
import { cn } from "~/lib/cn";
import { accountIdentity, sortedOrganizations, statusLabel, statusTone, type ProviderState } from "~/store/directory";
import { safeUrl } from "~/store/media";
import { call, functions, show, useStore } from "~/store/store";
import { toast, useUi } from "~/store/ui";
import { Avatar } from "./Avatar";
import { FunctionDialog } from "./FunctionDialog";
import { Button, EmptyState, Spinner } from "./ui";

const toneText = {
  ok: "text-success",
  busy: "text-warning",
  attention: "text-warning",
  error: "text-destructive",
};

/** An account: its status, how to sign in, and what it can do. */
export function ProviderPage({ name }: { name: string }) {
  const provider = useStore((s) => s.providers[name]);
  const [fns, setFns] = useState<QFunction[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<{ fn: QFunction; context: FunctionContext } | null>(null);
  const status = provider?.status;
  const statusKey = JSON.stringify(status);

  const refresh = useCallback(() => {
    setError(null);
    functions("Provider", name)
      .then(setFns)
      .catch((e) => {
        setFns([]);
        setError(describe(e));
      });
  }, [name]);

  // Discovery is a snapshot; fetch again when the account's status changes.
  useEffect(refresh, [refresh, statusKey]);

  if (!provider) {
    return <EmptyState title="Account not found">It may have been removed.</EmptyState>;
  }
  const [state, value] = tag(provider.status);
  const step = state === "LoginRequired" ? (value as LoginStep) : null;
  const identity = accountIdentity(provider);
  const tone = statusTone(provider.status);
  const loginFns = (fns ?? []).filter((f) => actionOf(f) === "Login");
  const other = (fns ?? []).filter((f) => {
    const a = actionOf(f);
    return a && a !== "Login" && a !== "Typing";
  });

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-13 shrink-0 items-center gap-2 border-b border-border px-3 sm:px-4">
        <Button variant="ghost" size="icon-sm" className="md:hidden" onClick={() => useUi.setState({ sidebarOpen: true })} aria-label="Open sidebar">
          <MenuIcon />
        </Button>
        <h1 className="text-[14px] font-semibold">{name}</h1>
        <span className={cn("text-xs", toneText[tone])}>{statusLabel(provider.status)}</span>
        <div className="flex-1" />
        <Button variant="ghost" size="icon-sm" onClick={refresh} aria-label="Refresh" title="Refresh">
          <RefreshCw />
        </Button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-2xl space-y-6 px-5 py-8">
          <div className="flex items-center gap-4">
            <Avatar name={name} size={56} square />
            <div className="min-w-0">
              <h2 className="text-xl font-semibold">{name}</h2>
              <p className="text-[13px] text-muted-foreground">
                {identity ?? "Not signed in"} · instance {provider.id.instance}
              </p>
            </div>
          </div>

          {step ? <LoginCard provider={provider} step={step} fns={loginFns} onOpen={(fn) => setDialog({ fn, context: "Provider" })} /> : null}

          {state === "Failed" || state === "Disconnected" ? (
            <div className="rounded-xl border border-destructive/30 bg-destructive/8 px-4 py-3 text-[13px] text-destructive">{statusLabel(provider.status)}</div>
          ) : null}

          {other.length ? (
            <section>
              <h3 className="mb-2 text-xs font-medium text-muted-foreground">Actions</h3>
              <div className="grid gap-2 sm:grid-cols-2">
                {other.map((fn) => {
                  const a = actionOf(fn);
                  const Icon = a === "OpenChannel" ? MessageSquarePlus : a === "Logout" ? LogOut : Sparkles;
                  return (
                    <button
                      key={fn.id}
                      onClick={() => {
                        if (a === "Logout" && !confirm(`Sign out of ${name}? Its saved sign-in on the server is deleted.`)) return;
                        if (fn.inputs.length === 0) {
                          call(fn, "Provider", {}, name).catch((e) => toast(describe(e), "error"));
                        } else setDialog({ fn, context: "Provider" });
                      }}
                      className={cn(
                        "flex items-start gap-3 rounded-xl border border-border bg-card p-3 text-left transition-colors hover:bg-accent",
                        a === "Logout" && "text-destructive",
                      )}
                    >
                      <Icon className="mt-0.5 size-4 shrink-0" />
                      <span className="min-w-0">
                        <span className="block text-[13px] font-medium">{fn.label}</span>
                        {fn.description ? <span className="block text-xs text-muted-foreground">{fn.description}</span> : null}
                      </span>
                    </button>
                  );
                })}
              </div>
            </section>
          ) : null}

          {fns === null ? (
            <div className="flex justify-center">
              <Spinner />
            </div>
          ) : null}
          {error ? <p className="text-xs text-destructive">{error}</p> : null}

          {sortedOrganizations(provider).length ? (
            <section>
              <h3 className="mb-2 text-xs font-medium text-muted-foreground">Organizations</h3>
              <div className="divide-y divide-border rounded-xl border border-border bg-card">
                {sortedOrganizations(provider).map((org) => (
                  <div key={org.id} className="flex items-center gap-3 px-3 py-2.5">
                    <Avatar name={org.name} seed={org.id} media={org.icon} size={28} square />
                    <span className="min-w-0 flex-1 truncate text-[13px]">{org.name}</span>
                    <span className="text-xs text-muted-foreground">
                      {Object.keys(org.scope.channels).length} channels · {Object.keys(org.scope.users).length} people
                    </span>
                  </div>
                ))}
              </div>
            </section>
          ) : null}
        </div>
      </div>
      {dialog ? (
        <FunctionDialog
          provider={name}
          context={dialog.context}
          fn={dialog.fn}
          // A login can wait for the person to finish elsewhere.
          timeoutMs={actionOf(dialog.fn) === "Login" ? 900_000 : undefined}
          initial={oauthDefaults(dialog.fn)}
          onClose={() => setDialog(null)}
          onDone={(value) => {
            if (typeof value === "object" && "Channel" in value) {
              useUi.setState({ providerPage: null });
              show({ provider: name, channel: value.Channel });
            }
            const oauth = oauthStart(value);
            if (oauth) void finishOAuth(name, fns ?? [], oauth);
          }}
        />
      ) : null}
    </div>
  );
}

function LoginCard({ provider, step, fns, onOpen }: { provider: ProviderState; step: LoginStep; fns: QFunction[]; onOpen: (fn: QFunction) => void }) {
  const link = safeUrl(step.link);
  const named = step.function ? fns.find((f) => f.id === step.function) : undefined;
  const choices = named ? [named] : fns;
  return (
    <section className="rounded-xl border border-warning/30 bg-warning/6 p-4">
      <div className="flex items-start gap-3">
        <KeyRound className="mt-0.5 size-4 shrink-0 text-warning" />
        <div className="min-w-0 flex-1 space-y-3">
          <div>
            <h3 className="text-[13px] font-semibold">Sign in to {provider.id.name}</h3>
            <p className="mt-0.5 text-[13px] whitespace-pre-line text-muted-foreground">{step.message}</p>
          </div>
          {step.qr_code ? <QrCode text={step.qr_code} /> : null}
          <div className="flex flex-wrap gap-2">
            {link ? (
              <Button variant="primary" onClick={() => window.open(link, "_blank", "noopener")}>
                <ExternalLink /> Open sign-in page
              </Button>
            ) : null}
            {choices.map((fn) => (
              <Button key={fn.id} variant={link ? "outline" : "primary"} onClick={() => onOpen(fn)}>
                {fn.label}
              </Button>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}

function QrCode({ text }: { text: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.innerHTML = renderSVG(text, { border: 2 });
  }, [text]);
  return (
    <div className="inline-block rounded-xl bg-white p-2">
      <div ref={ref} className="size-48 [&_svg]:size-full" aria-label="Sign-in QR code" />
    </div>
  );
}

// MARK: OAuth through this page

const CALLBACK = "/oauth/callback";

/** Points a login that asks for a client callback address at this page. */
function oauthDefaults(fn: QFunction) {
  return fn.inputs.some((i) => i.id === "redirect_uri") ? { redirect_uri: { Text: `${location.origin}${CALLBACK}` } as Value } : undefined;
}

function oauthStart(value: Value): { url: string; state: string } | null {
  if (typeof value !== "object" || !("Record" in value)) return null;
  const text = (v: Value | undefined) => (v && typeof v === "object" && "Text" in v ? v.Text : null);
  const url = safeUrl(text(value.Record.url));
  const state = text(value.Record.state);
  return url && state ? { url, state } : null;
}

/** Opens the provider's sign-in page; the callback page hands back the code. */
async function finishOAuth(provider: string, known: QFunction[], { url, state }: { url: string; state: string }) {
  const popup = window.open(url, "qmsg-oauth", "popup,width=520,height=680");
  if (!popup) toast("Allow pop-ups to sign in.", "error");
  const channel = new BroadcastChannel("qmsg-oauth");
  const result = await new Promise<{ code?: string; state?: string; error?: string } | null>((resolve) => {
    const timer = setTimeout(() => resolve(null), 10 * 60_000);
    channel.onmessage = (e) => {
      clearTimeout(timer);
      resolve(e.data as { code?: string; state?: string; error?: string });
    };
  });
  channel.close();
  const fns = await functions("Provider", provider).catch(() => known);
  if (!result || result.error || result.state !== state || !result.code) {
    const cancel = fns.find((f) => f.id === "cancel_login");
    if (cancel) await call(cancel, "Provider", {}, provider).catch(() => {});
    toast(result?.error ? `Sign-in was cancelled: ${result.error}` : "Sign-in didn't finish.", "error");
    return;
  }
  const finish = fns.find((f) => f.id === "finish_login");
  if (!finish) return toast("This account can't finish the sign-in.", "error");
  try {
    await call(finish, "Provider", { code: { Text: result.code }, state: { Text: result.state } }, provider, undefined, 120_000);
    toast("Signed in.");
  } catch (error) {
    toast(describe(error), "error");
  }
}

/** The page a provider's sign-in returns to, in its pop-up. */
export function OAuthCallback() {
  useEffect(() => {
    const params = new URLSearchParams(location.search);
    const channel = new BroadcastChannel("qmsg-oauth");
    channel.postMessage({ code: params.get("code") ?? undefined, state: params.get("state") ?? undefined, error: params.get("error") ?? undefined });
    channel.close();
    // Keep the code out of history.
    history.replaceState(null, "", CALLBACK);
    setTimeout(() => window.close(), 300);
  }, []);
  return <EmptyState title="Signed in">You can close this window.</EmptyState>;
}
