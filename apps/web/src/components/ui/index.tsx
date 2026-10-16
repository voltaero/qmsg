import { Loader2, X } from "lucide-react";
import {
  forwardRef,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type TextareaHTMLAttributes,
} from "react";
import { createPortal } from "react-dom";
import { cn } from "~/lib/cn";
import { useToasts } from "~/store/ui";

type Variant = "default" | "primary" | "ghost" | "outline" | "destructive";
type Size = "sm" | "md" | "icon" | "icon-sm";

const variants: Record<Variant, string> = {
  default: "bg-accent text-accent-foreground hover:bg-[color-mix(in_srgb,var(--accent)_70%,var(--foreground)_8%)]",
  primary: "bg-primary text-primary-foreground hover:brightness-110 shadow-sm",
  ghost: "text-muted-foreground hover:bg-accent hover:text-foreground",
  outline: "border border-border bg-card hover:bg-accent text-foreground",
  destructive: "bg-destructive/12 text-destructive hover:bg-destructive/20",
};
const sizes: Record<Size, string> = {
  sm: "h-7 px-2.5 text-xs gap-1.5 rounded-md",
  md: "h-8 px-3 text-[13px] gap-2 rounded-lg",
  icon: "size-8 rounded-lg",
  "icon-sm": "size-7 rounded-md",
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  loading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "default", size = "md", loading, className, children, disabled, type = "button", ...props },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      disabled={disabled || loading}
      className={cn(
        "inline-flex shrink-0 items-center justify-center font-medium whitespace-nowrap transition-colors select-none disabled:pointer-events-none disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0",
        variants[variant],
        sizes[size],
        className,
      )}
      {...props}
    >
      {loading ? <Loader2 className="animate-spin" /> : null}
      {children}
    </button>
  );
});

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(function Input(
  { className, ...props },
  ref,
) {
  return (
    <input
      ref={ref}
      className={cn(
        "h-9 w-full rounded-lg border border-input bg-transparent px-3 text-[13px] outline-none placeholder:text-muted-foreground/70 focus:border-ring/60 focus:ring-2 focus:ring-ring/20 disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
});

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaHTMLAttributes<HTMLTextAreaElement>>(function Textarea(
  { className, ...props },
  ref,
) {
  return (
    <textarea
      ref={ref}
      className={cn(
        "min-h-20 w-full resize-y rounded-lg border border-input bg-transparent px-3 py-2 text-[13px] outline-none placeholder:text-muted-foreground/70 focus:border-ring/60 focus:ring-2 focus:ring-ring/20",
        className,
      )}
      {...props}
    />
  );
});

export function Label({ children, hint, htmlFor }: { children: ReactNode; hint?: ReactNode; htmlFor?: string }) {
  return (
    <label htmlFor={htmlFor} className="mb-1.5 flex items-baseline justify-between gap-2 text-xs font-medium text-foreground/90">
      <span>{children}</span>
      {hint ? <span className="font-normal text-muted-foreground">{hint}</span> : null}
    </label>
  );
}

export function Spinner({ className }: { className?: string }) {
  return <Loader2 className={cn("size-4 animate-spin text-muted-foreground", className)} />;
}

export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <kbd
      className={cn(
        "inline-flex h-5 min-w-5 items-center justify-center rounded border border-border bg-muted px-1 font-sans text-2xs font-medium text-muted-foreground",
        className,
      )}
    >
      {children}
    </kbd>
  );
}

export function Switch({ checked, onChange, disabled }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:opacity-50",
        checked ? "bg-primary" : "bg-input",
      )}
    >
      <span
        className={cn(
          "inline-block size-4 rounded-full bg-white shadow transition-transform",
          checked ? "translate-x-[18px]" : "translate-x-0.5",
        )}
      />
    </button>
  );
}

/** A modal panel. Closes on Escape and outside clicks. */
export function Dialog({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  className,
}: {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  className?: string;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, onClose]);
  if (!open) return null;
  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/40 p-4 pt-[12vh] animate-fade-in"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        className={cn(
          "relative w-full max-w-md rounded-xl border border-border bg-popover text-popover-foreground shadow-2xl animate-pop-in",
          className,
        )}
      >
        {title ? (
          <div className="flex items-start gap-3 border-b border-border px-5 py-4">
            <div className="min-w-0 flex-1">
              <h2 className="text-[15px] font-semibold">{title}</h2>
              {description ? <p className="mt-1 text-[13px] text-muted-foreground">{description}</p> : null}
            </div>
            <Button variant="ghost" size="icon-sm" onClick={onClose} aria-label="Close" className="-mt-1 -mr-2">
              <X />
            </Button>
          </div>
        ) : null}
        <div className="px-5 py-4">{children}</div>
        {footer ? <div className="flex justify-end gap-2 border-t border-border px-5 py-3">{footer}</div> : null}
      </div>
    </div>,
    document.body,
  );
}

export interface MenuItem {
  label: ReactNode;
  icon?: ReactNode;
  onSelect: () => void;
  destructive?: boolean;
  disabled?: boolean;
  hint?: ReactNode;
}

/** A dropdown menu anchored to an element's rectangle. */
export function Menu({
  anchor,
  items,
  onClose,
  header,
  align = "start",
}: {
  anchor: DOMRect;
  items: (MenuItem | "separator")[];
  onClose: () => void;
  header?: ReactNode;
  align?: "start" | "end";
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ left: number; top: number }>({ left: anchor.left, top: anchor.bottom + 4 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    let left = align === "end" ? anchor.right - width : anchor.left;
    let top = anchor.bottom + 4;
    if (top + height > window.innerHeight - 8) top = Math.max(8, anchor.top - height - 4);
    left = Math.min(Math.max(8, left), window.innerWidth - width - 8);
    setPosition({ left, top });
  }, [anchor, align]);
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);
  return createPortal(
    <div
      ref={ref}
      role="menu"
      style={position}
      className="fixed z-50 min-w-48 rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-xl animate-pop-in"
    >
      {header}
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} className="my-1 h-px bg-border" />
        ) : (
          <button
            key={i}
            role="menuitem"
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
            className={cn(
              "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] hover:bg-accent disabled:opacity-50 [&_svg]:size-4 [&_svg]:text-muted-foreground",
              item.destructive && "text-destructive [&_svg]:text-destructive",
            )}
          >
            {item.icon}
            <span className="flex-1 truncate">{item.label}</span>
            {item.hint ? <span className="text-2xs text-muted-foreground">{item.hint}</span> : null}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}

export function EmptyState({ icon, title, children }: { icon?: ReactNode; title: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      {icon ? (
        <div className="flex size-12 items-center justify-center rounded-2xl border border-border bg-card text-muted-foreground [&_svg]:size-5">
          {icon}
        </div>
      ) : null}
      <div className="text-[15px] font-medium">{title}</div>
      {children ? <div className="max-w-sm text-[13px] text-muted-foreground">{children}</div> : null}
    </div>
  );
}

/** Shows the latest toast at the bottom of the window. */
export function Toaster() {
  const toast = useToasts((s) => s.current);
  if (!toast) return null;
  return createPortal(
    <div
      key={toast.id}
      role="status"
      className={cn(
        "fixed bottom-5 left-1/2 z-[60] max-w-md -translate-x-1/2 rounded-lg border bg-popover px-3.5 py-2 text-[13px] shadow-xl whitespace-pre-line animate-pop-in",
        toast.tone === "error" ? "border-destructive/30 text-destructive" : "border-border text-popover-foreground",
      )}
    >
      {toast.text}
    </div>,
    document.body,
  );
}
