import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import momentraceLogo from "./assets/momentrace-logo.png";
import { getIconBackground } from "./icon-background";
import {
  ArrowUpRight,
  CalendarRange,
  ChevronLeft,
  ChevronRight,
  Clock3,
  History,
  LayoutGrid,
  Monitor,
  Plus,
  Settings2,
  Sparkles,
  Tags,
  Trash2,
} from "lucide-react";
import {
  Area,
  AreaChart,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import "@fontsource-variable/noto-sans-sc";
import "@fontsource-variable/noto-serif-sc";
import "./styles.css";

type Item = {
  name: string;
  seconds: number;
  color?: string | null;
  executable?: string | null;
  processPath?: string | null;
};
type AppInfo = {
  executable: string;
  displayName: string;
  processPath: string;
  category: string;
  ignored: boolean;
};
type Status = {
  tracking: boolean;
  paused: boolean;
  idleSeconds: number;
  currentApp?: AppInfo | null;
};
type Day = {
  date: string;
  totalSeconds: number;
  categories: Item[];
  applications: Item[];
  hours: Item[];
};
type Range = {
  totalSeconds: number;
  days: Item[];
  categories: Item[];
  applications: Item[];
};
type All = {
  totalSeconds: number;
  months: Item[];
  categories: Item[];
  applications: Item[];
};
/** The first and last local dates that hold usage, used to keep the date
 * pickers inside the recorded window. */
type Bounds = { firstDate: string; lastDate: string };
type Category = { id: number; name: string; color: string };
type Rule = {
  executable: string;
  displayName: string;
  processPath: string;
  categoryId?: number | null;
  ignored: boolean;
  continueTrackingWhileIdle: boolean;
};
type FontFamily = "classic" | "serif" | "sans";
type Config = {
  idleMinutes: number;
  launchAtLogin: boolean;
  showWidget: boolean;
  fontFamily: FontFamily;
  widgetHeight: number;
  widgetXOffset: number;
  widgetYOffset: number;
};
/** A summary paired with the view key it was loaded for, so an in-flight or
 * failed request can never render old data under a new date heading. */
type DaySlot = { key: string; data: Day };
type RangeSlot = { key: string; data: Range };
type AllSlot = { key: string; data: All };

const viewKeyOf = (
  page: string,
  date: string,
  rangeStart: string,
  rangeEnd: string,
) =>
  page === "today"
    ? `day-${date}`
    : page === "range"
      ? `range-${rangeStart}-${rangeEnd}`
      : "all";

const formatDuration = (value = 0) =>
  `${Math.floor(value / 3600)}小时${String(Math.floor((value % 3600) / 60)).padStart(2, "0")}分`;
const formatShort = (value = 0) =>
  value >= 3600
    ? `${(value / 3600).toFixed(1)}小时`
    : `${Math.max(0, Math.round(value / 60))}分`;
const invokeSafe = <T,>(name: string, args?: Record<string, unknown>) =>
  invoke<T>(name, args);
const initials = (name: string) => name.trim().slice(0, 1).toUpperCase() || "•";
const shiftDate = (value: string, days: number) => {
  const [year, month, day] = value.split("-").map(Number);
  // UTC civil arithmetic avoids both the Date 0-99 year quirk and the local
  // timezone's DST gaps when rolling across midnight.
  const shifted = new Date(0);
  shifted.setUTCFullYear(year, month - 1, day + days);
  return `${shifted.getUTCFullYear()}-${String(shifted.getUTCMonth() + 1).padStart(2, "0")}-${String(shifted.getUTCDate()).padStart(2, "0")}`;
};
/** Inclusive day count between two ISO dates, using UTC civil arithmetic for the
 * same reason shiftDate does. */
const daysBetween = (start: string, end: string) => {
  const [ys, ms, ds] = start.split("-").map(Number);
  const [ye, me, de] = end.split("-").map(Number);
  return (
    Math.round((Date.UTC(ye, me - 1, de) - Date.UTC(ys, ms - 1, ds)) / 86_400_000) + 1
  );
};
const formatRangeLabel = (start: string, end: string) =>
  start && end
    ? `${start.replace(/-/g, "/")} – ${end.replace(/-/g, "/")} · ${daysBetween(start, end)} 天`
    : "";
/** Keeps a stored selection inside the recorded window. A stray year in a date
 * picker used to widen the window silently, so the headline total covered far
 * more than the dates the user believed were selected. */
const clampToBounds = (value: string, bounds?: Bounds) => {
  if (!value || !bounds) return value;
  if (value < bounds.firstDate) return bounds.firstDate;
  if (value > bounds.lastDate) return bounds.lastDate;
  return value;
};
const dashboardPages = ["today", "range", "all", "categories", "settings"];
const APPLICATIONS_PER_PAGE = 7;
const storedPage = () => {
  const value = localStorage.getItem("momentrace.dashboard.page");
  return value && dashboardPages.includes(value) ? value : "today";
};
const normalizeFontFamily = (value?: string | null): FontFamily =>
  value === "serif" || value === "sans" ? value : "classic";
const applyFontFamily = (value?: string | null) => {
  const fontFamily = normalizeFontFamily(value);
  document.documentElement.dataset.font = fontFamily;
  localStorage.setItem("momentrace.fontFamily", fontFamily);
};

applyFontFamily(localStorage.getItem("momentrace.fontFamily"));

type IconVisual = { source: string; background: string };
const ICON_CACHE_LIMIT = 192;
const ICON_CACHE_TTL_MS = 30 * 60 * 1000;
type IconEntry = { visual: IconVisual; storedAt: number };
const iconVisuals = new Map<string, IconEntry>();
const iconRequests = new Map<string, Promise<IconVisual | undefined>>();

const readIconVisual = (key: string) => {
  const entry = iconVisuals.get(key);
  if (!entry) return undefined;
  if (Date.now() - entry.storedAt > ICON_CACHE_TTL_MS) {
    iconVisuals.delete(key);
    return undefined;
  }
  // Refresh recency so frequently shown icons survive eviction.
  iconVisuals.delete(key);
  iconVisuals.set(key, entry);
  return entry.visual;
};

const rememberIconVisual = (key: string, visual: IconVisual) => {
  if (iconVisuals.size >= ICON_CACHE_LIMIT) {
    const oldest = iconVisuals.keys().next();
    if (!oldest.done) iconVisuals.delete(oldest.value);
  }
  iconVisuals.set(key, { visual, storedAt: Date.now() });
};

const loadIconVisual = (
  key: string,
  path?: string | null,
  executable?: string | null,
) => {
  const pending = iconRequests.get(key);
  if (pending) return pending;

  const request = invokeSafe<string>("get_app_icon", {
    processPath: path,
    executable,
  })
    .then(async (source) => {
      const visual = { source, background: await getIconBackground(source) };
      rememberIconVisual(key, visual);
      return visual;
    })
    .catch(() => undefined)
    .finally(() => {
      iconRequests.delete(key);
    });
  iconRequests.set(key, request);
  return request;
};

function AppIcon({
  name,
  path,
  executable,
  size = "normal",
}: {
  name: string;
  path?: string | null;
  executable?: string | null;
  size?: "small" | "normal" | "large";
}) {
  const key = `${path ?? ""}\u0000${executable ?? ""}`;
  const [icon, setIcon] = useState<IconVisual | undefined>(() =>
    readIconVisual(key),
  );
  useEffect(() => {
    const cached = readIconVisual(key);
    setIcon(cached);
    if (cached || (!path && !executable)) return;
    let active = true;
    void loadIconVisual(key, path, executable).then((visual) => {
      if (active && visual) setIcon(visual);
    });
    return () => {
      active = false;
    };
  }, [key, path, executable]);
  return (
    <span
      className={`app-icon ${size}`}
      style={
        {
          "--app-icon-background": icon?.background ?? "#ffffff",
        } as React.CSSProperties
      }
    >
      {icon ? <img src={icon.source} alt="" /> : initials(name)}
    </span>
  );
}

function Dashboard() {
  const [page, setPage] = useState(storedPage);
  const [date, setDate] = useState(
    () => localStorage.getItem("momentrace.dashboard.date") ?? "",
  );
  const [rangeStart, setRangeStart] = useState(
    () => localStorage.getItem("momentrace.dashboard.rangeStart") ?? "",
  );
  const [rangeEnd, setRangeEnd] = useState(
    () => localStorage.getItem("momentrace.dashboard.rangeEnd") ?? "",
  );
  const [daySlot, setDaySlot] = useState<DaySlot>();
  const [rangeSlot, setRangeSlot] = useState<RangeSlot>();
  const [allSlot, setAllSlot] = useState<AllSlot>();
  const [status, setStatus] = useState<Status>();
  const [categories, setCategories] = useState<Category[]>([]);
  const [rules, setRules] = useState<Rule[]>([]);
  const [settings, setSettings] = useState<Config>();
  const [bounds, setBounds] = useState<Bounds>();
  const loadId = useRef(0);
  const statusRevision = useRef(0);
  const view = useRef({ page, date, rangeStart, rangeEnd });
  useEffect(() => {
    view.current = { page, date, rangeStart, rangeEnd };
  });
  useEffect(() => {
    localStorage.setItem("momentrace.dashboard.page", page);
    localStorage.setItem("momentrace.dashboard.date", date);
    localStorage.setItem("momentrace.dashboard.rangeStart", rangeStart);
    localStorage.setItem("momentrace.dashboard.rangeEnd", rangeEnd);
  }, [page, date, rangeStart, rangeEnd]);
  useEffect(() => {
    if (settings) applyFontFamily(settings.fontFamily);
  }, [settings?.fontFamily]);
  const load = async () => {
    const requestId = ++loadId.current;
    const revisionAtFetch = statusRevision.current;
    try {
      const [today, recorded] = await Promise.all([
        invokeSafe<string>("get_today"),
        invokeSafe<Bounds | null>("get_recorded_bounds").catch(() => null),
      ]);
      if (requestId !== loadId.current) return;
      // Today stays selectable even before it holds any usage, but dates
      // outside [first recorded day, today] cannot be chosen at all.
      const limits = recorded
        ? {
            firstDate: recorded.firstDate,
            lastDate: recorded.lastDate > today ? recorded.lastDate : today,
          }
        : undefined;
      if (limits) setBounds(limits);
      const selectedDate = clampToBounds(date || today, limits);
      if (date !== selectedDate)
        setDate((current) => (current === date ? selectedDate : current));
      const selectedRangeEnd = clampToBounds(rangeEnd || selectedDate, limits);
      const selectedRangeStart = clampToBounds(
        rangeStart || shiftDate(selectedRangeEnd, -6),
        limits,
      );
      // Updater form: a value typed while this load was in flight must win over
      // the fallback resolved from the state this load started with.
      if (rangeStart !== selectedRangeStart)
        setRangeStart((current) =>
          current === rangeStart ? selectedRangeStart : current,
        );
      if (rangeEnd !== selectedRangeEnd)
        setRangeEnd((current) =>
          current === rangeEnd ? selectedRangeEnd : current,
        );
      const commit = async <X,>(
        promise: Promise<X>,
        apply: (value: X) => void,
      ) => {
        try {
          apply(await promise);
        } catch (error) {
          console.error("Unable to load dashboard data", error);
        }
      };
      if (requestId !== loadId.current) return;
      const isCurrent = () => requestId === loadId.current;
      await Promise.all([
        commit(invokeSafe<Status>("get_live_status"), (live) => {
          if (isCurrent() && statusRevision.current === revisionAtFetch)
            setStatus(live);
        }),
        commit(invokeSafe<Category[]>("get_categories"), (value) => {
          if (isCurrent()) setCategories(value);
        }),
        commit(invokeSafe<Rule[]>("get_app_rules"), (value) => {
          if (isCurrent()) setRules(value);
        }),
        commit(invokeSafe<Config>("get_settings"), (value) => {
          if (isCurrent()) setSettings(value);
        }),
        ...(page === "today"
          ? [
              commit(
                invokeSafe<Day>("get_day_summary", { date: selectedDate }),
                (value) => {
                  if (isCurrent())
                    setDaySlot({
                      key: viewKeyOf("today", selectedDate, "", ""),
                      data: value,
                    });
                },
              ),
            ]
          : []),
        ...(page === "range"
          ? [
              commit(
                invokeSafe<Range>("get_range_summary", {
                  startDate: selectedRangeStart,
                  endDate: selectedRangeEnd,
                }),
                (value) => {
                  if (isCurrent())
                    setRangeSlot({
                      key: viewKeyOf(
                        "range",
                        "",
                        selectedRangeStart,
                        selectedRangeEnd,
                      ),
                      data: value,
                    });
                },
              ),
            ]
          : []),
        ...(page === "all"
          ? [
              commit(invokeSafe<All>("get_all_summary"), (value) => {
                if (isCurrent()) setAllSlot({ key: "all", data: value });
              }),
            ]
          : []),
      ]);
    } catch (error) {
      console.error("Unable to load dashboard data", error);
    }
  };
  const refresh = async () => {
    const requestId = loadId.current;
    const revisionAtFetch = statusRevision.current;
    const {
      page: activePage,
      date: activeDate,
      rangeStart: activeStart,
      rangeEnd: activeEnd,
    } = view.current;
    try {
      const live = await invokeSafe<Status>("get_live_status");
      let nextDay: Day | undefined;
      let nextRange: Range | undefined;
      let nextAll: All | undefined;
      let rangeWindow: { start: string; end: string } | undefined;
      if (activePage === "today") {
        nextDay = await invokeSafe<Day>("get_day_summary", {
          date: activeDate || (await invokeSafe<string>("get_today")),
        });
      } else if (activePage === "range") {
        const selectedEnd =
          activeEnd || activeDate || (await invokeSafe<string>("get_today"));
        rangeWindow = {
          start: activeStart || shiftDate(selectedEnd, -6),
          end: selectedEnd,
        };
        nextRange = await invokeSafe<Range>("get_range_summary", {
          startDate: rangeWindow.start,
          endDate: rangeWindow.end,
        });
      } else if (activePage === "all") {
        nextAll = await invokeSafe<All>("get_all_summary");
      }
      if (requestId !== loadId.current) return;
      if (statusRevision.current === revisionAtFetch) setStatus(live);
      if (nextDay) {
        setDaySlot({
          key: viewKeyOf("today", activeDate || "", "", ""),
          data: nextDay,
        });
      }
      if (nextRange && rangeWindow) {
        // Key the slot with the window that was actually queried, so a summary
        // can never be rendered under a different pair of dates.
        setRangeSlot({
          key: viewKeyOf("range", "", rangeWindow.start, rangeWindow.end),
          data: nextRange,
        });
      }
      if (nextAll) setAllSlot({ key: "all", data: nextAll });
    } catch (error) {
      console.error("Unable to refresh dashboard data", error);
    }
  };
  useEffect(() => {
    void load();
  }, [page, date, rangeStart, rangeEnd]);
  // Registered once for the lifetime of the dashboard. Re-running this whenever
  // the selected dates changed used to tear down the tracker-status listener and
  // restart the timer, so live updates were dropped while the subscription was
  // being re-established and the periodic refresh never fired for anyone who
  // kept adjusting the selection. Everything below reads refs, so the closure
  // captured here always sees the current view.
  useEffect(() => {
    const timer = window.setInterval(() => void refresh(), 30_000);
    const live = listen<Status>("tracker-status", (event) => {
      statusRevision.current += 1;
      setStatus(event.payload);
    });
    const widget = listen<boolean>("widget-visibility", (event) =>
      setSettings((current) =>
        current ? { ...current, showWidget: event.payload } : current,
      ),
    );
    return () => {
      window.clearInterval(timer);
      void live.then((unlisten) => unlisten());
      void widget.then((unlisten) => unlisten());
    };
  }, []);
  const nav = [
    { id: "today", label: "今天", icon: Clock3 },
    { id: "range", label: "时间段", icon: CalendarRange },
    { id: "all", label: "累计", icon: History },
    { id: "categories", label: "分类", icon: Tags },
    { id: "settings", label: "设置", icon: Settings2 },
  ];
  const title =
    page === "today"
      ? "今日，留给重要的事。"
      : page === "range"
        ? "一段时间，一种节奏。"
        : page === "all"
          ? "走过的时间，都有迹可循。"
          : page === "categories"
            ? "把时间归于意义。"
            : "让记录顺其自然。";
  // Hover text only, so a mistyped year is still verifiable without touching
  // the layout of the header.
  const rangeTitle = formatRangeLabel(rangeStart, rangeEnd)
    ? `统计区间 ${formatRangeLabel(rangeStart, rangeEnd)}`
    : "统计区间";
  const settingsIntent = useRef<Config>();
  const settingsSaving = useRef(false);
  const saveSettings = (next: Config) => {
    settingsIntent.current = next;
    setSettings(next);
    if (settingsSaving.current) return;
    settingsSaving.current = true;
    void (async () => {
      while (true) {
        const target = settingsIntent.current!;
        try {
          await invokeSafe("update_settings", { settings: target });
        } catch (error) {
          if (settingsIntent.current === target) alert(String(error));
        }
        if (settingsIntent.current === target) break;
      }
      settingsSaving.current = false;
    })();
  };
  const saveRule = async (rule: Rule, patch: Partial<Rule>) => {
    setRules((current) =>
      current.map((item) =>
        item.executable === rule.executable ? { ...item, ...patch } : item,
      ),
    );
    try {
      await invokeSafe("update_app_rule", {
        executable: rule.executable,
        patch: {
          displayName: patch.displayName,
          categoryId: patch.categoryId,
          categoryIdChanged: Object.prototype.hasOwnProperty.call(
            patch,
            "categoryId",
          ),
          ignored: patch.ignored,
          continueTrackingWhileIdle: patch.continueTrackingWhileIdle,
        },
      });
    } catch (error) {
      alert(String(error));
    } finally {
      await load();
    }
  };
  return (
    <main className="shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">
            <img src={momentraceLogo} alt="" />
          </span>
          <div>
            <b>时迹</b>
            <small>MOMENTRACE</small>
          </div>
        </div>
        <nav>
          {nav.map((item) => {
            const Icon = item.icon;
            return (
              <button
                key={item.id}
                className={page === item.id ? "active" : ""}
                onClick={() => setPage(item.id)}
              >
                <Icon size={17} />
                <span>{item.label}</span>
              </button>
            );
          })}
        </nav>
        <div className="side-now">
          <div className="side-now-head">
            <span
              className={status?.paused ? "status-dot paused" : "status-dot"}
            />
            <small>{status?.paused ? "已暂停记录" : "正在记录"}</small>
          </div>
          <div className="side-now-app">
            <AppIcon
              name={status?.currentApp?.displayName ?? "时迹"}
              path={status?.currentApp?.processPath}
              executable={status?.currentApp?.executable}
              size="small"
            />
            <div>
              <b>{status?.currentApp?.displayName ?? "等待活动"}</b>
              <span>{status?.currentApp?.category ?? "Local only"}</span>
            </div>
          </div>
        </div>
        <p className="side-privacy">所有使用记录仅保存在此设备</p>
      </aside>
      <section className="content">
        <header
          className={`page-header${page === "range" ? " range-header" : ""}`}
        >
          <div>
            <p>
              {page === "today"
                ? "TODAY"
                : page === "range"
                  ? "DATE RANGE"
                  : page === "all"
                    ? "ALL TIME"
                    : page.toUpperCase()}
            </p>
            <h1>{title}</h1>
          </div>
          <div className="header-actions">
            {page === "today" && (
              <input
                type="date"
                value={date}
                min={bounds?.firstDate}
                max={bounds?.lastDate}
                onChange={(event) =>
                  setDate(clampToBounds(event.target.value, bounds))
                }
              />
            )}
            {page === "range" && (
              <>
                <input
                  type="date"
                  className="range-date"
                  aria-label="开始日期"
                  title={rangeTitle}
                  value={rangeStart}
                  min={bounds?.firstDate}
                  max={rangeEnd || bounds?.lastDate}
                  onChange={(event) => {
                    const value = clampToBounds(event.target.value, bounds);
                    setRangeStart(value);
                    if (rangeEnd && value > rangeEnd) setRangeEnd(value);
                  }}
                />
                <input
                  type="date"
                  className="range-date"
                  aria-label="结束日期"
                  title={rangeTitle}
                  value={rangeEnd}
                  min={rangeStart || bounds?.firstDate}
                  max={bounds?.lastDate}
                  onChange={(event) => {
                    const value = clampToBounds(event.target.value, bounds);
                    setRangeEnd(value);
                    if (rangeStart && value < rangeStart) setRangeStart(value);
                  }}
                />
              </>
            )}
            <button
              className="header-status"
              disabled={!status}
              onClick={() =>
                status &&
                void invokeSafe("set_tracking_paused", {
                  paused: status.tracking,
                }).catch((error) => alert(String(error)))
              }
            >
              <span
                className={
                  status?.tracking === false ? "status-dot paused" : "status-dot"
                }
              />
              {status?.tracking === false
                ? "手动暂停中"
                : status?.paused
                  ? "空闲暂停中"
                  : "专注记录中"}
            </button>
          </div>
        </header>
        {page === "today" || page === "range" || page === "all" ? (
          <Overview
            viewKey={
              page === "today"
                ? `day-${date}`
                : page === "range"
                  ? `range-${rangeStart}-${rangeEnd}`
                  : "all"
            }
            data={
              page === "all"
                ? allSlot?.key === "all"
                  ? allSlot.data
                  : undefined
                : page === "range"
                  ? rangeSlot?.key === `range-${rangeStart}-${rangeEnd}`
                    ? rangeSlot.data
                    : undefined
                  : daySlot?.key === `day-${date}`
                    ? daySlot.data
                    : undefined
            }
            period={
              page === "all" ? "all" : page === "range" ? "range" : "day"
            }
            current={status?.currentApp}
          />
        ) : null}
        {page === "categories" ? (
          <Categories rules={rules} categories={categories} save={saveRule} />
        ) : null}
        {page === "settings" ? (
          <Settings
            settings={settings}
            categories={categories}
            save={saveSettings}
            createCategory={async (name, color) => {
              await invokeSafe("create_category", { name, color });
              await load();
            }}
            deleteCategory={async (id) => {
              await invokeSafe("delete_category", { id });
              await load();
            }}
            clear={async () => {
              if (confirm("这会永久删除所有本地使用记录，确定继续吗？")) {
                try {
                  await invokeSafe("clear_usage_data");
                  await load();
                } catch (error) {
                  alert(String(error));
                }
              }
            }}
          />
        ) : null}
      </section>
    </main>
  );
}

function Overview({
  viewKey,
  data,
  period,
  current,
}: {
  viewKey: string;
  data?: Day | Range | All;
  period: "day" | "range" | "all";
  current?: AppInfo | null;
}) {
  const [appPage, setAppPage] = useState(0);
  const [pagedView, setPagedView] = useState(viewKey);
  if (pagedView !== viewKey) {
    setPagedView(viewKey);
    setAppPage(0);
  }
  const isRange = period === "range";
  const isAll = period === "all";
  // Every summary reports its own total, clipped to the window it was queried
  // for. Re-adding the per-bucket values here used to lose a second per day and
  // could not tell whether the buckets covered the selected dates at all.
  const total = data?.totalSeconds;
  const categories = data?.categories ?? [];
  const apps = data?.applications ?? [];
  const chartData =
    period === "day"
      ? (data as Day)?.hours
      : isAll
        ? (data as All)?.months
        : (data as Range)?.days;
  const peak = apps[0];
  const appPageCount = Math.max(
    1,
    Math.ceil(apps.length / APPLICATIONS_PER_PAGE),
  );
  const safeAppPage = Math.min(appPage, appPageCount - 1);
  const visibleApps = apps.slice(
    safeAppPage * APPLICATIONS_PER_PAGE,
    (safeAppPage + 1) * APPLICATIONS_PER_PAGE,
  );
  useEffect(() => {
    setAppPage((current) => Math.min(current, appPageCount - 1));
  }, [appPageCount]);
  const topShare = total && peak ? Math.round((peak.seconds / total) * 100) : 0;
  return (
    <>
      <section className="overview-hero">
        <div className="hero-copy">
          <span className="eyebrow">
            {isAll ? "全部累计" : isRange ? "时间段累计" : "今天已记录"}
          </span>
          <strong>{formatDuration(total)}</strong>
          <p>每一次专注，都值得被温柔地看见。</p>
          <div className="hero-meta">
            <span>
              <Sparkles size={14} /> {categories.length || 0} 个时间分类
            </span>
            <span>
              <LayoutGrid size={14} /> {apps.length || 0} 个应用
            </span>
          </div>
        </div>
        <div className="hero-orb">
          <div
            className="orb-ring"
            style={
              {
                "--share": `${Math.max(0, Math.min(topShare, 100))}%`,
              } as React.CSSProperties
            }
          >
            <div className="orb-core">
              <b>{peak ? `${topShare}%` : "—"}</b>
              <span>最常使用</span>
            </div>
          </div>
          <small>{peak?.name ?? "等待数据"}</small>
        </div>
      </section>
      <section className="insight-grid">
        <article className="glass-card now-card">
          <div className="card-kicker">
            <span>正在进行</span>
            <i className="status-dot" />
          </div>
          <div className="now-app">
            <AppIcon
              name={current?.displayName ?? "等待活动"}
              path={current?.processPath}
              executable={current?.executable}
              size="large"
            />
            <div>
              <b>{current?.displayName ?? "开始使用应用后显示"}</b>
              <span>{current?.category ?? "本地隐私追踪"}</span>
            </div>
          </div>
          <div className="now-rule" />
          <p>不会记录窗口标题、键盘输入或浏览内容。</p>
        </article>
        <article className="glass-card focus-card">
          <div className="card-kicker">
            <span>主要投入</span>
            <ArrowUpRight size={17} />
          </div>
          <div className="focus-app">
            {peak ? (
              <AppIcon
                name={peak.name}
                path={peak.processPath}
                executable={peak.executable}
                size="normal"
              />
            ) : (
              <span className="app-icon normal">•</span>
            )}
            <div>
              <b>{peak?.name ?? "还没有数据"}</b>
              <span>
                {peak ? `占已记录时间 ${topShare}%` : "从第一段专注开始"}
              </span>
            </div>
            <strong>{formatShort(peak?.seconds)}</strong>
          </div>
          <div className="mini-track">
            <i style={{ width: `${topShare}%` }} />
          </div>
        </article>
        <article className="glass-card category-card">
          <div className="card-kicker">
            <span>时间去向</span>
            <span>{categories.length} 类</span>
          </div>
          <div className="category-swatches">
            {categories.slice(0, 5).map((item) => (
              <span
                key={item.name}
                style={{
                  background: item.color ?? "#92a2b9",
                  flex: Math.max(item.seconds, 1),
                }}
              />
            ))}
          </div>
          <div className="category-names">
            {categories.slice(0, 3).map((item) => (
              <span key={item.name}>
                <i style={{ background: item.color ?? "#92a2b9" }} />
                {item.name}
              </span>
            ))}
          </div>
        </article>
      </section>
      <section className="main-grid">
        <article className="glass-card chart-card">
          <div className="chart-heading">
            <div>
              <span className="eyebrow">时间流向</span>
              <b>
                {isAll
                  ? "每个月的积累"
                  : isRange
                    ? "每一天的节奏"
                    : "每小时的专注"}
              </b>
            </div>
            <small>{formatShort(total)} 已记录</small>
          </div>
          <ResponsiveContainer width="100%" height={255}>
            <AreaChart data={chartData}>
              <defs>
                <linearGradient id="timeGlow" x1="0" x2="0" y1="0" y2="1">
                  <stop offset="0" stopColor="#7e87ff" stopOpacity=".48" />
                  <stop offset="1" stopColor="#7e87ff" stopOpacity="0" />
                </linearGradient>
              </defs>
              <XAxis
                dataKey="name"
                axisLine={false}
                tickLine={false}
                tick={{ fill: "#8d96ab", fontSize: 11 }}
              />
              <YAxis hide />
              <Tooltip
                formatter={(value: number) => formatShort(value)}
                contentStyle={{
                  borderRadius: 14,
                  border: "1px solid #ffffff",
                  boxShadow: "0 12px 32px #25396b22",
                }}
              />
              <Area
                type="monotone"
                dataKey="seconds"
                stroke="#6e76f4"
                strokeWidth={3}
                fill="url(#timeGlow)"
              />
            </AreaChart>
          </ResponsiveContainer>
        </article>
        <article className="glass-card balance-card">
          <div className="chart-heading">
            <div>
              <span className="eyebrow">分类比例</span>
              <b>时间的平衡</b>
            </div>
          </div>
          <div className="balance-body">
            <ResponsiveContainer width="100%" height={180}>
              <PieChart>
                <Pie
                  data={categories}
                  dataKey="seconds"
                  nameKey="name"
                  innerRadius={57}
                  outerRadius={79}
                  paddingAngle={5}
                >
                  {categories.map((item) => (
                    <Cell key={item.name} fill={item.color ?? "#92a2b9"} />
                  ))}
                </Pie>
                <Tooltip formatter={(value: number) => formatShort(value)} />
              </PieChart>
            </ResponsiveContainer>
            <div className="balance-list">
              {categories.slice(0, 4).map((item) => (
                <div key={item.name}>
                  <span>
                    <i style={{ background: item.color ?? "#92a2b9" }} />
                    {item.name}
                  </span>
                  <b>{formatShort(item.seconds)}</b>
                </div>
              ))}
            </div>
          </div>
        </article>
        <article className="glass-card applications-card">
          <div className="chart-heading">
            <div>
              <span className="eyebrow">应用档案</span>
              <b>
                {isAll
                  ? "累计常用应用"
                  : isRange
                    ? "时间段常用应用"
                    : "今天常用应用"}
              </b>
            </div>
            <small>{apps.length} 项</small>
          </div>
          {apps.length ? (
            <div
              className={`app-archive${appPageCount > 1 ? " paginated" : ""}`}
            >
              <div className="app-list">
                {visibleApps.map((app, index) => (
                  <div
                    className="app-row"
                    key={app.executable ?? app.name}
                  >
                    <span className="app-rank">
                      {String(
                        safeAppPage * APPLICATIONS_PER_PAGE + index + 1,
                      ).padStart(2, "0")}
                    </span>
                    <AppIcon
                      name={app.name}
                      path={app.processPath}
                      executable={app.executable}
                      size="normal"
                    />
                    <div className="app-copy">
                      <b>{app.name}</b>
                      <span>
                        <i />
                        <em
                          style={{
                            width: `${Math.max(8, (app.seconds / Math.max(peak?.seconds ?? 1, 1)) * 100)}%`,
                          }}
                        />
                      </span>
                    </div>
                    <strong>{formatShort(app.seconds)}</strong>
                  </div>
                ))}
              </div>
              {appPageCount > 1 ? (
                <div className="app-pagination">
                  <button
                    type="button"
                    title="上一页"
                    aria-label="应用档案上一页"
                    disabled={safeAppPage === 0}
                    onClick={() => setAppPage(safeAppPage - 1)}
                  >
                    <ChevronLeft size={16} />
                  </button>
                  <span>
                    {safeAppPage + 1} / {appPageCount}
                  </span>
                  <button
                    type="button"
                    title="下一页"
                    aria-label="应用档案下一页"
                    disabled={safeAppPage === appPageCount - 1}
                    onClick={() => setAppPage(safeAppPage + 1)}
                  >
                    <ChevronRight size={16} />
                  </button>
                </div>
              ) : null}
            </div>
          ) : (
            <div className="empty-state">
              <Monitor size={24} />
              <p>
                {isAll
                  ? "开始使用应用后，这里会呈现累积的专注轨迹。"
                  : isRange
                    ? "所选时间段有记录后，这里会呈现专注轨迹。"
                    : "开始使用应用后，这里会呈现今日的专注轨迹。"}
              </p>
            </div>
          )}
        </article>
      </section>
    </>
  );
}

function Categories({
  rules,
  categories,
  save,
}: {
  rules: Rule[];
  categories: Category[];
  save: (rule: Rule, patch: Partial<Rule>) => Promise<void>;
}) {
  const [editing, setEditing] = useState<string>();
  const [name, setName] = useState("");
  const startEditing = (rule: Rule) => {
    setEditing(rule.executable);
    setName(rule.displayName);
  };
  const finishEditing = async (rule: Rule, commit: boolean) => {
    if (editing !== rule.executable) return;
    const nextName = name.trim();
    setEditing(undefined);
    if (commit && nextName && nextName !== rule.displayName)
      await save(rule, { displayName: nextName });
  };
  return (
    <article className="glass-card table-card">
      <div className="table-intro">
        <div>
          <span className="eyebrow">分类整理</span>
          <b>把每个应用放在合适的位置</b>
        </div>
        <p>双击应用名称可自定义显示名；按可执行文件分类。</p>
      </div>
      <div className="rule-head">
        <span>应用</span>
        <span>归属分类</span>
        <span>空闲续记</span>
        <span>忽略</span>
      </div>
      {rules.map((rule) => (
        <div className="rule-row" key={rule.executable}>
          <div className="rule-app">
            <AppIcon
              name={rule.displayName}
              path={rule.processPath}
              executable={rule.executable}
              size="normal"
            />
            <div>
              {editing === rule.executable ? (
                <input
                  className="rule-name-input"
                  aria-label={`重命名 ${rule.displayName}`}
                  value={name}
                  autoFocus
                  maxLength={48}
                  onChange={(event) => setName(event.target.value)}
                  onBlur={() => void finishEditing(rule, true)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void finishEditing(rule, true);
                    }
                    if (event.key === "Escape") {
                      event.preventDefault();
                      void finishEditing(rule, false);
                    }
                  }}
                />
              ) : (
                <b
                  className="editable-rule-name"
                  title="双击修改显示名称"
                  onDoubleClick={() => startEditing(rule)}
                >
                  {rule.displayName}
                </b>
              )}
              <small>{rule.executable}</small>
            </div>
          </div>
          <select
            className="rule-category-select"
            value={rule.categoryId ?? ""}
            onChange={(event) =>
              save(rule, {
                categoryId: event.target.value
                  ? Number(event.target.value)
                  : null,
              })
            }
          >
            <option value="">其他</option>
            {categories.map((category) => (
              <option value={category.id} key={category.id}>
                {category.name}
              </option>
            ))}
          </select>
          <label className="switch" title="无键鼠操作时仍继续记录">
            <input
              aria-label={`空闲时继续记录 ${rule.displayName}`}
              type="checkbox"
              checked={rule.continueTrackingWhileIdle}
              onChange={(event) =>
                save(rule, {
                  continueTrackingWhileIdle: event.target.checked,
                })
              }
            />
            <span />
          </label>
          <label className="switch">
            <input
              aria-label={`忽略 ${rule.displayName}`}
              type="checkbox"
              checked={rule.ignored}
              onChange={(event) =>
                save(rule, { ignored: event.target.checked })
              }
            />
            <span />
          </label>
        </div>
      ))}
    </article>
  );
}
function Settings({
  settings,
  categories,
  save,
  createCategory,
  deleteCategory,
  clear,
}: {
  settings?: Config;
  categories: Category[];
  save: (settings: Config) => void | Promise<void>;
  createCategory: (name: string, color: string) => Promise<void>;
  deleteCategory: (id: number) => Promise<void>;
  clear: () => Promise<void>;
}) {
  const [categoryName, setCategoryName] = useState("");
  const [categoryColor, setCategoryColor] = useState("#6E8CFF");
  if (!settings) return null;
  return (
    <article className="glass-card settings">
      <label>
        <div>
          <span className="eyebrow">界面字体</span>
          <b>字体风格</b>
          <small>字体随安装包提供，切换后立即生效</small>
        </div>
        <select
          value={settings.fontFamily}
          onChange={(event) =>
            save({
              ...settings,
              fontFamily: event.target.value as FontFamily,
            })
          }
        >
          <option value="classic">原版设计（默认）</option>
          <option value="serif">思源宋体</option>
          <option value="sans">思源黑体</option>
        </select>
      </label>
      <label>
        <div>
          <span className="eyebrow">自动暂停</span>
          <b>空闲暂停阈值</b>
          <small>连续无键鼠操作后停止记录</small>
        </div>
        <select
          value={settings.idleMinutes}
          onChange={(event) =>
            save({ ...settings, idleMinutes: Number(event.target.value) })
          }
        >
          {[1, 5, 10, 15, 30].map((minute) => (
            <option value={minute} key={minute}>
              {minute} 分钟
            </option>
          ))}
        </select>
      </label>
      <label>
        <div>
          <span className="eyebrow">开机启动</span>
          <b>登录后自动开始记录</b>
          <small>始终保持本地、安静地追踪</small>
        </div>
        <label className="switch">
          <input
            type="checkbox"
            checked={settings.launchAtLogin}
            onChange={(event) =>
              save({ ...settings, launchAtLogin: event.target.checked })
            }
          />
          <span />
        </label>
      </label>
      <label>
        <div>
          <span className="eyebrow">左下角应用条</span>
          <b>显示当前应用</b>
          <small>与任务栏齐高，只显示当前正在使用的应用。</small>
        </div>
        <label className="switch">
          <input
            type="checkbox"
            checked={settings.showWidget}
            onChange={(event) => {
              const next = { ...settings, showWidget: event.target.checked };
              void save(next);
            }}
          />
          <span />
        </label>
      </label>
      <label className="widget-geometry-setting">
        <div>
          <span className="eyebrow">悬浮窗适配</span>
          <b>尺寸与位置</b>
          <small>相对自动贴合位置，以逻辑像素精确调整</small>
        </div>
        <div className="widget-geometry-controls">
          <span>
            <small>高度</small>
            <input
              type="number"
              aria-label="悬浮窗高度"
              min={48}
              max={160}
              step={1}
              value={settings.widgetHeight}
              onChange={(event) =>
                save({
                  ...settings,
                  widgetHeight: Math.min(
                    160,
                    Math.max(48, Number(event.target.value) || 48),
                  ),
                })
              }
            />
          </span>
          <span>
            <small>水平</small>
            <input
              type="number"
              aria-label="悬浮窗水平偏移"
              min={-1000}
              max={1000}
              step={1}
              value={settings.widgetXOffset}
              onChange={(event) =>
                save({
                  ...settings,
                  widgetXOffset: Math.min(
                    1000,
                    Math.max(-1000, Number(event.target.value) || 0),
                  ),
                })
              }
            />
          </span>
          <span>
            <small>垂直</small>
            <input
              type="number"
              aria-label="悬浮窗垂直偏移"
              min={-1000}
              max={1000}
              step={1}
              value={settings.widgetYOffset}
              onChange={(event) =>
                save({
                  ...settings,
                  widgetYOffset: Math.min(
                    1000,
                    Math.max(-1000, Number(event.target.value) || 0),
                  ),
                })
              }
            />
          </span>
          <button
            type="button"
            onClick={(event) => {
              event.preventDefault();
              void save({
                ...settings,
                widgetHeight: 64,
                widgetXOffset: 0,
                widgetYOffset: 0,
              });
            }}
          >
            重置
          </button>
        </div>
      </label>
      <div className="category-manager-setting">
        <div>
          <span className="eyebrow">分类管理</span>
          <b>自定义分类</b>
          <small>删除后，相关应用会自动归入其他</small>
        </div>
        <div className="category-manager">
          <div className="category-manager-list">
            {categories.map((category) => (
              <span className="category-manager-item" key={category.id}>
                <i style={{ background: category.color }} />
                <b>{category.name}</b>
                <button
                  type="button"
                  title={`删除分类 ${category.name}`}
                  aria-label={`删除分类 ${category.name}`}
                  onClick={() => {
                    if (!confirm(`删除“${category.name}”分类？`)) return;
                    void deleteCategory(category.id).catch((error) =>
                      alert(String(error)),
                    );
                  }}
                >
                  <Trash2 size={14} />
                </button>
              </span>
            ))}
          </div>
          <form
            className="category-create-form"
            onSubmit={(event) => {
              event.preventDefault();
              const name = categoryName.trim();
              if (!name) return;
              void createCategory(name, categoryColor)
                .then(() => setCategoryName(""))
                .catch((error) => alert(String(error)));
            }}
          >
            <input
              className="category-color-input"
              type="color"
              aria-label="新分类颜色"
              value={categoryColor}
              onChange={(event) => setCategoryColor(event.target.value)}
            />
            <input
              type="text"
              aria-label="新分类名称"
              placeholder="分类名称"
              maxLength={20}
              value={categoryName}
              onChange={(event) => setCategoryName(event.target.value)}
            />
            <button type="submit" disabled={!categoryName.trim()}>
              <Plus size={15} />
              新增
            </button>
          </form>
        </div>
      </div>
      <div className="danger">
        <div>
          <span className="eyebrow">数据管理</span>
          <b>清除所有使用记录</b>
          <small>该操作不可撤销，只会删除本机保存的数据。</small>
        </div>
        <button onClick={clear}>清除数据</button>
      </div>
    </article>
  );
}

const root = createRoot(document.getElementById("root")!);
root.render(<Dashboard />);
