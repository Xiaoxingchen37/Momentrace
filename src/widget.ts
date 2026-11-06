import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "@fontsource-variable/noto-sans-sc";
import "@fontsource-variable/noto-serif-sc";
import { getIconBackground } from "./icon-background";
import "./widget.css";

type AppInfo = {
  executable: string;
  displayName: string;
  processPath: string;
  category: string;
  ignored: boolean;
};

type WidgetStatus = {
  tracking: boolean;
  paused: boolean;
  idleSeconds: number;
  currentApp?: AppInfo | null;
  currentAppSeconds?: number | null;
};

type FontFamily = "system" | "custom";
type Settings = { fontFamily?: FontFamily | null; fontName?: string | null };
type FontSelection = { family: FontFamily; name?: string | null };

const root = document.getElementById("root");
if (!root) throw new Error("Widget root element is missing");

document.body.classList.add("widget-view");
root.innerHTML = `
  <main class="taskbar-overlay">
    <span class="app-icon small" aria-hidden="true">
      <img alt="" hidden />
      <span class="app-icon-fallback">时</span>
    </span>
    <div class="taskbar-app-copy">
      <b>等待应用活动</b>
      <span>本地记录</span>
    </div>
    <strong>0分</strong>
  </main>
`;

const iconImage = root.querySelector<HTMLImageElement>(".app-icon img")!;
const iconContainer = root.querySelector<HTMLElement>(".app-icon")!;
const iconFallback = root.querySelector<HTMLElement>(".app-icon-fallback")!;
const appName = root.querySelector<HTMLElement>(".taskbar-app-copy b")!;
const category = root.querySelector<HTMLElement>(".taskbar-app-copy span")!;
const elapsed = root.querySelector<HTMLElement>(".taskbar-overlay > strong")!;

let iconRequestId = 0;
let currentIconSource: string | null = null;
let statusRevision = 0;
let fontRevision = 0;

const normalizeFontFamily = (value?: string | null): FontFamily =>
  value === "custom" ? "custom" : "system";

const sanitizeFontName = (value?: string | null) =>
  (value ?? "")
    .replace(/["'\\;{}<>]/g, "")
    .trim()
    .slice(0, 64);

const applyFontFamily = (value?: string | null, name?: string | null) => {
  const fontFamily = normalizeFontFamily(value);
  const fontName = fontFamily === "custom" ? sanitizeFontName(name) : "";
  const root = document.documentElement;
  root.dataset.font =
    fontFamily === "custom" && !fontName ? "system" : fontFamily;
  if (fontName) root.style.setProperty("--custom-font", `"${fontName}"`);
  else root.style.removeProperty("--custom-font");
  localStorage.setItem("momentrace.fontFamily", fontFamily);
  localStorage.setItem("momentrace.fontName", fontName);
};

/** The widget window is a separate document, so it resolves the system face on
 * its own instead of relying on the dashboard having done it. */
const loadSystemUiFont = async () => {
  try {
    const face = sanitizeFontName(await invoke<string>("get_system_ui_font"));
    if (face) {
      document.documentElement.style.setProperty("--system-font", `"${face}"`);
      localStorage.setItem("momentrace.systemFont", face);
    }
  } catch {
    // The stylesheet stack already covers a failed lookup.
  }
};

const cachedSystemFont = sanitizeFontName(
  localStorage.getItem("momentrace.systemFont"),
);
if (cachedSystemFont) {
  document.documentElement.style.setProperty(
    "--system-font",
    `"${cachedSystemFont}"`,
  );
}
void loadSystemUiFont();

const initials = (name: string) => name.trim().slice(0, 1).toUpperCase() || "·";

const formatShort = (value = 0) =>
  value >= 3600
    ? `${(value / 3600).toFixed(1)}小时`
    : `${Math.max(0, Math.round(value / 60))}分`;

const showIconFallback = (name: string) => {
  iconContainer.style.setProperty("--app-icon-background", "#ffffff");
  iconImage.hidden = true;
  iconImage.removeAttribute("src");
  iconFallback.textContent = initials(name);
  iconFallback.hidden = false;
};

const updateIcon = async (
  name: string,
  processPath?: string | null,
  executable?: string | null,
) => {
  const path = processPath || null;
  const source = `${path ?? ""}\u0000${executable ?? ""}`;
  if (source === currentIconSource) {
    if ((!path && !executable) || iconImage.hidden) showIconFallback(name);
    return;
  }

  currentIconSource = source;
  const requestId = ++iconRequestId;
  showIconFallback(name);
  if (!path && !executable) return;

  try {
    const icon = await invoke<string>("get_app_icon", {
      processPath: path,
      executable,
    });
    const background = await getIconBackground(icon);
    if (requestId !== iconRequestId || source !== currentIconSource) return;
    iconContainer.style.setProperty("--app-icon-background", background);
    iconImage.src = icon;
    iconImage.hidden = false;
    iconFallback.hidden = true;
  } catch {
    // A transient failure must not permanently poison this app's icon: reset
    // the source so the next status update retries. The Rust side throttles
    // repeated lookups for executables that genuinely have no icon.
    currentIconSource = "";
    showIconFallback(name);
  }
};

const render = (status: WidgetStatus) => {
  const current = status.currentApp;
  const name = current?.displayName ?? (status.paused ? "已暂停记录" : "等待应用活动");
  appName.textContent = name;
  category.textContent = current?.category ?? "本地记录";
  elapsed.textContent = formatShort(current ? (status.currentAppSeconds ?? 0) : 0);
  void updateIcon(name, current?.processPath, current?.executable);
};

applyFontFamily(
  localStorage.getItem("momentrace.fontFamily"),
  localStorage.getItem("momentrace.fontName"),
);

const statusListener = listen<WidgetStatus>("widget-status", (event) => {
  statusRevision += 1;
  render(event.payload);
});
const fontListener = listen<FontSelection>("font-family-changed", (event) => {
  fontRevision += 1;
  applyFontFamily(event.payload.family, event.payload.name);
});

void statusListener.catch(() => undefined);
void fontListener.catch(() => undefined);

const initialRevision = statusRevision;
void invoke<WidgetStatus>("get_widget_status")
  .then((status) => {
    if (initialRevision === statusRevision) render(status);
  })
  .catch(() => undefined);
const initialFontRevision = fontRevision;
void invoke<Settings>("get_settings")
  .then((settings) => {
    if (initialFontRevision === fontRevision)
      applyFontFamily(settings.fontFamily, settings.fontName);
  })
  .catch(() => undefined);

window.addEventListener("beforeunload", () => {
  void statusListener.then((unlisten) => unlisten()).catch(() => undefined);
  void fontListener.then((unlisten) => unlisten()).catch(() => undefined);
});
