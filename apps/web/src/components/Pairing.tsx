import { ClipboardPaste, Sparkles, TerminalSquare } from "lucide-react";
import { useState } from "react";
import { defaultDeviceName } from "~/api/account";
import { describe } from "~/api/errors";
import { pair, startDemo, useStore } from "~/store/store";
import { Button, Input, Label, Textarea } from "./ui";

export function Pairing() {
  const needsPairing = useStore((s) => s.needsPairing);
  const [text, setText] = useState("");
  const [name, setName] = useState(defaultDeviceName());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await pair(text, name.trim() || defaultDeviceName());
    } catch (e) {
      setError(describe(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full overflow-y-auto">
      <div className="m-auto w-full max-w-lg px-5 py-10">
        <div className="mb-8 flex items-center gap-3">
          <img src="/favicon.svg" alt="" className="size-10" />
          <div>
            <h1 className="text-xl font-semibold tracking-tight">qmsg</h1>
            <p className="text-[13px] text-muted-foreground">Simple messaging for everything and everyone.</p>
          </div>
        </div>
        <div className="rounded-2xl border border-border bg-card p-6 shadow-composer">
          <h2 className="text-[15px] font-semibold">{needsPairing ? "Pair this browser again" : "Pair this browser"}</h2>
          <p className="mt-1 text-[13px] text-muted-foreground">
            {needsPairing
              ? "The server no longer accepts this browser's token: it expired or was revoked."
              : "Connect to your qmsg server. On the server, create a one-time pairing code:"}
          </p>
          <div className="mt-3 flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-2 font-mono text-xs">
            <TerminalSquare className="size-3.5 shrink-0 text-muted-foreground" />
            <span className="truncate">qmsg-orchestrator pair orchestrator.toml --json</span>
          </div>
          <form
            className="mt-5 space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <div>
              <Label htmlFor="pairing" hint={
                <button
                  type="button"
                  className="inline-flex items-center gap-1 hover:text-foreground"
                  onClick={async () => setText(await navigator.clipboard.readText().catch(() => text))}
                >
                  <ClipboardPaste className="size-3" /> Paste
                </button>
              }>
                Pairing code
              </Label>
              <Textarea
                id="pairing"
                autoFocus
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder='{"version":1,"endpoint":"…","code":"…",…}'
                className="font-mono text-xs"
                rows={4}
              />
            </div>
            <div>
              <Label htmlFor="device">Device name</Label>
              <Input id="device" value={name} onChange={(e) => setName(e.target.value)} />
            </div>
            {error ? <p className="rounded-lg bg-destructive/10 px-3 py-2 text-[13px] text-destructive">{error}</p> : null}
            <Button type="submit" variant="primary" className="w-full" loading={busy} disabled={!text.trim()}>
              Pair
            </Button>
          </form>
        </div>
        <div className="mt-4 flex items-center justify-between gap-3 px-1 text-[13px] text-muted-foreground">
          <span>No server handy?</span>
          <Button variant="ghost" onClick={() => void startDemo()}>
            <Sparkles /> Try the demo
          </Button>
        </div>
        <p className="mt-6 px-1 text-xs leading-relaxed text-muted-foreground/80">
          The code grants full access to your server and works once, for five minutes. This browser keeps its device
          token in local storage; unpair it from Settings when you're done.
        </p>
      </div>
    </div>
  );
}
