import { ExternalLink, Paperclip, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { newOperation, type ActionOperation } from "~/api/client";
import { AppError, describe } from "~/api/errors";
import { actionOf, tag, type Arguments, type CompletionItem, type FunctionContext, type FunctionInput, type QFunction, type Value } from "~/api/types";
import { cn } from "~/lib/cn";
import { formatBytes } from "~/lib/content";
import { safeUrl } from "~/store/media";
import { call, complete, uploadFor, verify } from "~/store/store";
import { Button, Dialog, Input, Label, Spinner, Switch, Textarea } from "./ui";

type Item = { label: string; value: Value };
type Raw = string | boolean | Item[] | { text: string; files: File[] };

function isTextList(input: FunctionInput) {
  return typeof input.value_type === "object" && "List" in input.value_type && input.value_type.List === "Text";
}

function initialRaw(input: FunctionInput, value?: Value): Raw {
  const type = input.value_type;
  if (type === "Boolean") return !!value && typeof value === "object" && "Boolean" in value && value.Boolean;
  if (isTextList(input)) {
    if (value && typeof value === "object" && "List" in value) return value.List.map((v) => ({ label: valueSummary(v), value: v }));
    return [];
  }
  if (type === "Content") {
    const text =
      value && typeof value === "object" && "Content" in value
        ? value.Content.map((c) => ("Text" in c ? c.Text : "Formatted" in c ? c.Formatted.text : "")).join("\n")
        : "";
    return { text, files: [] };
  }
  if (typeof input.hint === "object" && "Choices" in input.hint) {
    const i = value ? input.hint.Choices.findIndex((c) => JSON.stringify(c.value) === JSON.stringify(value)) : -1;
    return i >= 0 ? String(i) : "";
  }
  if (!value) return "";
  if (typeof value === "object" && "Text" in value) return value.Text;
  if (typeof value === "object" && "Integer" in value) return String(value.Integer);
  return JSON.stringify(value);
}

/** A short readable rendering of a value, for results and chips. */
export function valueSummary(value: Value): string {
  const [kind, v] = tag(value);
  switch (kind) {
    case "Null":
      return "Done";
    case "Boolean":
      return v ? "Yes" : "No";
    case "Integer":
    case "Text":
      return String(v);
    case "Bytes":
      return `${(v as number[]).length} bytes`;
    case "List":
      return (v as Value[]).map(valueSummary).join(", ");
    case "Record":
      return Object.entries(v as Record<string, Value>)
        .map(([k, x]) => `${k}: ${valueSummary(x)}`)
        .join("\n");
    case "Channel":
      return (v as { channel: string }).channel;
    default:
      return kind;
  }
}

async function toValue(input: FunctionInput, raw: Raw, provider: string): Promise<Value | undefined> {
  const type = input.value_type;
  if (typeof input.hint === "object" && "Choices" in input.hint) {
    return raw === "" ? undefined : input.hint.Choices[Number(raw)]?.value;
  }
  if (type === "Boolean") return { Boolean: raw as boolean };
  if (isTextList(input)) {
    const items = raw as Item[];
    return items.length ? { List: items.map((i) => i.value) } : undefined;
  }
  if (type === "Content") {
    const { text, files } = raw as { text: string; files: File[] };
    const content = text.trim() ? [{ Text: text.trim() }] : [];
    for (const file of files) content.push((await uploadFor(provider, file)) as never);
    return content.length ? { Content: content } : undefined;
  }
  const text = raw as string;
  if (!text.trim()) return undefined;
  if (type === "Text" || type === "Any") return { Text: text };
  if (type === "Integer") {
    const n = Number(text);
    if (!Number.isInteger(n)) throw new FieldError(input.id, "Enter a whole number.");
    return { Integer: n };
  }
  try {
    return JSON.parse(text) as Value;
  } catch {
    throw new FieldError(input.id, "Enter a qmsg JSON value.");
  }
}

class FieldError extends Error {
  constructor(
    readonly input: string,
    message: string,
  ) {
    super(message);
  }
}

/** The arguments typed so far, for completion and verification helpers. */
function partial(inputs: FunctionInput[], raws: Record<string, Raw>, skip: string): Arguments {
  const args: Arguments = {};
  for (const input of inputs) {
    if (input.id === skip) continue;
    const raw = raws[input.id];
    if (typeof raw === "string" && raw && (input.value_type === "Text" || input.value_type === "Any")) args[input.id] = { Text: raw };
    if (typeof raw === "boolean") args[input.id] = { Boolean: raw };
  }
  return args;
}

/** A form for any function, built from its declared inputs. */
export function FunctionDialog({
  provider,
  context,
  fn,
  onClose,
  onDone,
  initial,
  submitLabel,
  timeoutMs,
}: {
  provider: string;
  context: FunctionContext;
  fn: QFunction;
  onClose: () => void;
  onDone?: (value: Value) => void;
  initial?: Arguments;
  submitLabel?: string;
  timeoutMs?: number;
}) {
  const [raws, setRaws] = useState<Record<string, Raw>>(() =>
    Object.fromEntries(fn.inputs.map((i) => [i.id, initialRaw(i, initial?.[i.id])])),
  );
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Value | null>(null);
  // One operation per form, reused when retrying after a lost answer.
  const operation = useRef<ActionOperation>(newOperation());
  const isAction = !!actionOf(fn);

  const submit = async () => {
    setBusy(true);
    setErrors({});
    try {
      const args: Arguments = {};
      for (const input of fn.inputs) {
        const value = await toValue(input, raws[input.id]!, provider);
        if (value === undefined) {
          if (input.required) throw new FieldError(input.id, "Required.");
          continue;
        }
        args[input.id] = value;
      }
      const value = await call(fn, context, args, provider, isAction ? operation.current : undefined, timeoutMs);
      onDone?.(value);
      if (value === "Null" || !showsResult(value)) onClose();
      else setResult(value);
    } catch (error) {
      if (error instanceof FieldError) setErrors({ [error.input]: error.message });
      else if (error instanceof AppError) {
        const issues = error.issues;
        setErrors(Object.keys(issues).length ? issues : { "": error.message });
        // A refused action never ran; a new attempt is a new operation.
        if (!error.deliveryUnknown) operation.current = newOperation();
      } else setErrors({ "": describe(error) });
    } finally {
      setBusy(false);
    }
  };

  if (result) {
    return (
      <Dialog open onClose={onClose} title={fn.label} footer={<Button variant="primary" onClick={onClose}>Done</Button>}>
        <ResultView value={result} />
      </Dialog>
    );
  }

  return (
    <Dialog
      open
      onClose={onClose}
      title={fn.label}
      description={fn.description || undefined}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" loading={busy} onClick={submit}>
            {submitLabel ?? (isAction ? fn.label : "Run")}
          </Button>
        </>
      }
    >
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        {fn.inputs.map((input, i) => (
          <InputField
            key={input.id}
            autoFocus={i === 0}
            input={input}
            raw={raws[input.id]!}
            error={errors[input.id]}
            onChange={(raw) => {
              setRaws((r) => ({ ...r, [input.id]: raw }));
              if (errors[input.id]) setErrors(({ [input.id]: _gone, ...rest }) => rest);
            }}
            onBlur={async () => {
              if (!input.verification) return;
              const value = await toValue(input, raws[input.id]!, provider).catch(() => undefined);
              if (value === undefined) return;
              const issue = await verify(fn, input, context, partial(fn.inputs, raws, input.id), value, provider).catch(() => null);
              if (issue) setErrors((e) => ({ ...e, [input.id]: issue }));
            }}
            suggest={
              input.completion
                ? (query) => complete(fn, input, context, partial(fn.inputs, raws, input.id), query, provider)
                : undefined
            }
          />
        ))}
        {fn.inputs.length === 0 ? <p className="text-[13px] text-muted-foreground">{fn.description || "Run this action?"}</p> : null}
        {errors[""] ? <p className="rounded-lg bg-destructive/10 px-3 py-2 text-[13px] whitespace-pre-line text-destructive">{errors[""]}</p> : null}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}

function showsResult(value: Value): boolean {
  if (value === "Null") return false;
  return !(typeof value === "object" && "Channel" in value);
}

/** Shows a result, with links made openable, such as a sign-in page. */
export function ResultView({ value }: { value: Value }) {
  if (typeof value === "object" && "Record" in value) {
    return (
      <dl className="space-y-2 text-[13px]">
        {Object.entries(value.Record).map(([k, v]) => {
          const text = typeof v === "object" && "Text" in v ? v.Text : null;
          const url = safeUrl(text);
          return (
            <div key={k}>
              <dt className="text-xs text-muted-foreground">{k}</dt>
              <dd className="break-all">
                {url ? (
                  <a href={url} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-primary hover:underline">
                    Open link <ExternalLink className="size-3" />
                  </a>
                ) : (
                  valueSummary(v)
                )}
              </dd>
            </div>
          );
        })}
      </dl>
    );
  }
  return <p className="text-[13px] whitespace-pre-line">{valueSummary(value)}</p>;
}

function InputField({
  input,
  raw,
  error,
  onChange,
  onBlur,
  suggest,
  autoFocus,
}: {
  input: FunctionInput;
  raw: Raw;
  error?: string;
  onChange: (raw: Raw) => void;
  onBlur: () => void;
  suggest?: (query: string) => Promise<CompletionItem[]>;
  autoFocus?: boolean;
}) {
  const id = `fn-${input.id}`;
  let control: ReactNode;
  const hint = input.hint;
  if (typeof hint === "object" && "Choices" in hint) {
    const choices = hint.Choices;
    control =
      choices.length <= 4 ? (
        <div className="flex flex-wrap gap-1.5">
          {choices.map((c, i) => (
            <button
              key={i}
              type="button"
              onClick={() => onChange(String(i))}
              className={cn(
                "h-8 rounded-lg border px-3 text-[13px] transition-colors",
                raw === String(i) ? "border-primary bg-primary/12 text-primary" : "border-input hover:bg-accent",
              )}
            >
              {c.label}
            </button>
          ))}
        </div>
      ) : (
        <select
          id={id}
          value={raw as string}
          onChange={(e) => onChange(e.target.value)}
          className="h-9 w-full rounded-lg border border-input bg-transparent px-2.5 text-[13px] outline-none focus:border-ring/60"
        >
          <option value="">Choose…</option>
          {choices.map((c, i) => (
            <option key={i} value={i}>{c.label}</option>
          ))}
        </select>
      );
  } else if (input.value_type === "Boolean") {
    control = <Switch checked={raw as boolean} onChange={onChange} />;
  } else if (isTextList(input)) {
    control = <ListInput id={id} items={raw as Item[]} onChange={onChange} suggest={suggest} autoFocus={autoFocus} />;
  } else if (input.value_type === "Content") {
    const value = raw as { text: string; files: File[] };
    control = (
      <div>
        <Textarea id={id} autoFocus={autoFocus} value={value.text} onChange={(e) => onChange({ ...value, text: e.target.value })} rows={5} />
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {value.files.map((f, i) => (
            <span key={i} className="flex items-center gap-1 rounded-md bg-accent px-2 py-1 text-xs">
              {f.name} <span className="text-muted-foreground">{formatBytes(f.size)}</span>
              <button type="button" onClick={() => onChange({ ...value, files: value.files.filter((_, j) => j !== i) })} aria-label="Remove">
                <X className="size-3" />
              </button>
            </span>
          ))}
          <label className="flex h-7 cursor-pointer items-center gap-1 rounded-md px-2 text-xs text-muted-foreground hover:bg-accent">
            <Paperclip className="size-3.5" /> Attach
            <input type="file" multiple hidden onChange={(e) => onChange({ ...value, files: [...value.files, ...Array.from(e.target.files ?? [])] })} />
          </label>
        </div>
      </div>
    );
  } else if (hint === "Multiline" || (input.value_type !== "Text" && input.value_type !== "Integer" && input.value_type !== "Any")) {
    control = (
      <Textarea
        id={id}
        autoFocus={autoFocus}
        value={raw as string}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
        placeholder={input.value_type === "Text" ? undefined : "JSON"}
        className={input.value_type === "Text" ? undefined : "font-mono text-xs"}
      />
    );
  } else if (suggest) {
    control = <SuggestInput id={id} value={raw as string} onChange={onChange} suggest={suggest} autoFocus={autoFocus} />;
  } else {
    control = (
      <Input
        id={id}
        autoFocus={autoFocus}
        type={hint === "Secret" ? "password" : input.value_type === "Integer" ? "number" : "text"}
        autoComplete={hint === "Secret" ? "off" : undefined}
        value={raw as string}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
      />
    );
  }
  return (
    <div>
      <Label htmlFor={id} hint={input.required ? undefined : "Optional"}>
        {input.label}
      </Label>
      {control}
      {input.description ? <p className="mt-1 text-xs text-muted-foreground">{input.description}</p> : null}
      {error ? <p className="mt-1 text-xs text-destructive">{error}</p> : null}
    </div>
  );
}

function useSuggestions(query: string, suggest?: (q: string) => Promise<CompletionItem[]>) {
  const [items, setItems] = useState<CompletionItem[]>([]);
  const [loading, setLoading] = useState(false);
  // The callback changes every render; searches follow the query alone.
  const latest = useRef(suggest);
  latest.current = suggest;
  const enabled = !!suggest;
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    setLoading(true);
    const timer = setTimeout(() => {
      latest.current!(query)
        .then((r) => live && setItems(r))
        .catch(() => live && setItems([]))
        .finally(() => live && setLoading(false));
    }, 180);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [query, enabled]);
  return { items, loading };
}

function Suggestions({ items, loading, onPick, active }: { items: CompletionItem[]; loading: boolean; onPick: (i: CompletionItem) => void; active: number }) {
  if (!items.length && !loading) return null;
  return (
    <div className="absolute inset-x-0 top-full z-10 mt-1 max-h-56 overflow-y-auto rounded-lg border border-border bg-popover p-1 shadow-xl">
      {loading && !items.length ? (
        <div className="flex items-center gap-2 px-2 py-1.5 text-xs text-muted-foreground">
          <Spinner className="size-3" /> Searching…
        </div>
      ) : null}
      {items.map((item, i) => (
        <button
          key={i}
          type="button"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onPick(item)}
          className={cn("flex w-full flex-col rounded-md px-2 py-1.5 text-left hover:bg-accent", i === active && "bg-accent")}
        >
          <span className="text-[13px]">{item.label}</span>
          {item.description ? <span className="text-xs text-muted-foreground">{item.description}</span> : null}
        </button>
      ))}
    </div>
  );
}

function SuggestInput({
  id,
  value,
  onChange,
  suggest,
  autoFocus,
}: {
  id: string;
  value: string;
  onChange: (v: string) => void;
  suggest: (q: string) => Promise<CompletionItem[]>;
  autoFocus?: boolean;
}) {
  const [focused, setFocused] = useState(false);
  const { items, loading } = useSuggestions(focused ? value : "", focused ? suggest : undefined);
  return (
    <div className="relative">
      <Input id={id} autoFocus={autoFocus} value={value} onChange={(e) => onChange(e.target.value)} onFocus={() => setFocused(true)} onBlur={() => setFocused(false)} autoComplete="off" />
      {focused ? (
        <Suggestions
          items={items}
          loading={loading}
          active={-1}
          onPick={(item) => {
            onChange(typeof item.value === "object" && "Text" in item.value ? item.value.Text : item.label);
            setFocused(false);
          }}
        />
      ) : null}
    </div>
  );
}

/** Text items as chips, picked from suggestions or typed. */
function ListInput({
  id,
  items,
  onChange,
  suggest,
  autoFocus,
}: {
  id: string;
  items: Item[];
  onChange: (items: Item[]) => void;
  suggest?: (q: string) => Promise<CompletionItem[]>;
  autoFocus?: boolean;
}) {
  const [query, setQuery] = useState("");
  const [focused, setFocused] = useState(false);
  const [active, setActive] = useState(0);
  const { items: found, loading } = useSuggestions(query, focused ? suggest : undefined);
  const picked = useMemo(() => new Set(items.map((i) => JSON.stringify(i.value))), [items]);
  const options = found.filter((f) => !picked.has(JSON.stringify(f.value)));
  const add = (item: Item) => {
    onChange([...items, item]);
    setQuery("");
    setActive(0);
  };
  return (
    <div className="relative">
      <div className="flex min-h-9 flex-wrap items-center gap-1 rounded-lg border border-input px-1.5 py-1 focus-within:border-ring/60 focus-within:ring-2 focus-within:ring-ring/20">
        {items.map((item, i) => (
          <span key={i} className="flex items-center gap-1 rounded-md bg-accent py-0.5 pr-1 pl-2 text-xs">
            {item.label}
            <button type="button" onClick={() => onChange(items.filter((_, j) => j !== i))} aria-label={`Remove ${item.label}`}>
              <X className="size-3" />
            </button>
          </span>
        ))}
        <input
          id={id}
          autoFocus={autoFocus}
          value={query}
          autoComplete="off"
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setActive((a) => Math.min(a + 1, options.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setActive((a) => Math.max(a - 1, 0));
            } else if (e.key === "Enter" || e.key === ",") {
              if (!query.trim() && e.key === "Enter") return;
              e.preventDefault();
              const option = options[active];
              if (option) add({ label: option.label, value: option.value });
              else if (query.trim()) add({ label: query.trim(), value: { Text: query.trim() } });
            } else if (e.key === "Backspace" && !query && items.length) {
              onChange(items.slice(0, -1));
            }
          }}
          className="h-6 min-w-24 flex-1 bg-transparent px-1 text-[13px] outline-none"
          placeholder={items.length ? "" : "Type to search…"}
        />
      </div>
      {focused && (query || options.length) ? (
        <Suggestions items={options} loading={loading} active={active} onPick={(o) => add({ label: o.label, value: o.value })} />
      ) : null}
    </div>
  );
}
