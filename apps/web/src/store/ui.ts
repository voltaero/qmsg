import { create } from "zustand";

interface Toast {
  id: number;
  text: string;
  tone: "error" | "info";
}

export const useToasts = create<{ current: Toast | null }>()(() => ({ current: null }));

let timer: ReturnType<typeof setTimeout> | undefined;
let nextId = 0;

export function toast(text: string, tone: Toast["tone"] = "info") {
  clearTimeout(timer);
  useToasts.setState({ current: { id: ++nextId, text, tone } });
  timer = setTimeout(() => useToasts.setState({ current: null }), tone === "error" ? 6000 : 3500);
}

export type Theme = "system" | "light" | "dark";

interface Ui {
  /** An account's page, shown instead of a channel, such as to sign in. */
  providerPage: string | null;
  paletteOpen: boolean;
  settings: null | "accounts" | "devices" | "appearance";
  /** On narrow screens, whether the sidebar covers the chat. */
  sidebarOpen: boolean;
  collapsed: Record<string, boolean>;
  theme: Theme;
  members: boolean;
}

const COLLAPSED = "qmsg.collapsed";

function loadCollapsed(): Record<string, boolean> {
  try {
    return JSON.parse(localStorage.getItem(COLLAPSED) ?? "{}") as Record<string, boolean>;
  } catch {
    return {};
  }
}

export const useUi = create<Ui>()(() => ({
  providerPage: null,
  paletteOpen: false,
  settings: null,
  sidebarOpen: false,
  collapsed: loadCollapsed(),
  theme: (localStorage.getItem("qmsg.theme") as Theme | null) ?? "system",
  members: localStorage.getItem("qmsg.members") !== "off",
}));

export function toggleCollapsed(id: string) {
  const collapsed = { ...useUi.getState().collapsed, [id]: !useUi.getState().collapsed[id] };
  localStorage.setItem(COLLAPSED, JSON.stringify(collapsed));
  useUi.setState({ collapsed });
}

export function setTheme(theme: Theme) {
  localStorage.setItem("qmsg.theme", theme);
  useUi.setState({ theme });
  applyTheme();
}

export function toggleMembers() {
  const members = !useUi.getState().members;
  localStorage.setItem("qmsg.members", members ? "on" : "off");
  useUi.setState({ members });
}

const media = matchMedia("(prefers-color-scheme: dark)");
export function applyTheme() {
  const theme = useUi.getState().theme;
  document.documentElement.classList.toggle("dark", theme === "dark" || (theme === "system" && media.matches));
}
media.addEventListener("change", applyTheme);
