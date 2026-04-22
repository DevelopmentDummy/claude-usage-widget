import { useEffect, useRef, useState } from "react";
import { cursorPosition, getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import Header from "./components/Header";
import ProviderCard from "./components/ProviderCard";
import SettingsMenu from "./components/SettingsMenu";
import MiniGauge from "./components/MiniGauge";
import { ipc, type ProviderSnapshot } from "./lib/ipc";
import type { Provider, Settings, ViewMode } from "./lib/types";

const PROVIDER_ORDER: Provider[] = ["claude", "codex", "gemini"];
const PROVIDER_LABELS: Record<Provider, string> = {
  claude: "Claude",
  codex: "Codex",
  gemini: "Gemini",
};
const PROVIDER_CODE: Record<Provider, string> = {
  claude: "C",
  codex: "X",
  gemini: "G",
};

const WIDTH_BY_MODE: Record<ViewMode, number> = {
  normal: 320,
  mini: 160,
  super: 210,
};

type SnapshotMap = Partial<Record<Provider, ProviderSnapshot>>;

export default function App() {
  const [snapshots, setSnapshots] = useState<SnapshotMap>({});
  const [settings, setSettings] = useState<Settings | null>(null);
  const [refreshing, setRefreshing] = useState<Record<Provider, boolean>>({
    claude: false, codex: false, gemini: false,
  });
  const [menuOpen, setMenuOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<Provider>("claude");
  const [, setTick] = useState(0);
  const [titleBarVisible, setTitleBarVisible] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const hideGuardUntilRef = useRef(0);

  useEffect(() => {
    const armHideGuard = (event: MouseEvent) => {
      if (event.button !== 0) return;
      const target = event.target;
      if (!(target instanceof HTMLElement)) return;
      const dragRegion = target.closest("[data-window-drag-region='true']");
      if (!dragRegion || target.closest("button")) return;
      hideGuardUntilRef.current = Date.now() + 400;
      setTitleBarVisible(true);
    };

    document.addEventListener("mousedown", armHideGuard, true);
    return () => {
      document.removeEventListener("mousedown", armHideGuard, true);
    };
  }, []);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    let cancelled = false;
    let pollInFlight = false;

    const syncTitleBarVisibility = async () => {
      if (pollInFlight) return;
      pollInFlight = true;
      try {
        const [cursor, position, size] = await Promise.all([
          cursorPosition(),
          currentWindow.outerPosition(),
          currentWindow.outerSize(),
        ]);
        if (cancelled) return;
        const hovered =
          cursor.x >= position.x &&
          cursor.x < position.x + size.width &&
          cursor.y >= position.y &&
          cursor.y < position.y + size.height;
        const shouldShow =
          menuOpen ||
          hovered ||
          Date.now() < hideGuardUntilRef.current;
        setTitleBarVisible((prev) => (prev === shouldShow ? prev : shouldShow));
      } catch {
        if (!cancelled && Date.now() >= hideGuardUntilRef.current && !menuOpen) {
          setTitleBarVisible(false);
        }
      } finally {
        pollInFlight = false;
      }
    };

    syncTitleBarVisibility();
    const id = window.setInterval(syncTitleBarVisibility, 75);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [menuOpen]);

  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, []);

  const fetchActive = async (force = false) => {
    setRefreshing((r) => ({ ...r, [activeTab]: true }));
    try {
      const snap = await ipc.getProviderUsage(activeTab, force);
      setSnapshots((s) => ({ ...s, [activeTab]: snap }));
    } finally {
      setRefreshing((r) => ({ ...r, [activeTab]: false }));
    }
  };

  useEffect(() => {
    ipc.getSettings().then((s) => {
      setSettings(s);
      document.documentElement.style.setProperty("--widget-opacity", String(s.opacity));
    });
    ipc.getAllSnapshots().then((all) => {
      const next: SnapshotMap = {};
      for (const key of Object.keys(all) as Provider[]) {
        next[key] = all[key];
      }
      setSnapshots(next);
    });

    const unsubUpdated = ipc.onProviderUpdated((p) => {
      setSnapshots((s) => ({ ...s, [p.provider]: p.snapshot }));
      setRefreshing((r) => ({ ...r, [p.provider]: false }));
    });
    const unsubRefreshing = ipc.onUsageRefreshing((p) => {
      setRefreshing((r) => ({ ...r, [p.provider]: true }));
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "F5") { e.preventDefault(); fetchActive(true); }
      if (e.key === "Escape") { getCurrentWindow().minimize(); }
      if (e.ctrlKey && e.key === "q") { getCurrentWindow().close(); }
    };
    window.addEventListener("keydown", onKey);

    return () => {
      unsubUpdated.then((u) => u());
      unsubRefreshing.then((u) => u());
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const run = (force: boolean) => {
      ipc.getProviderUsage(activeTab, force).then((snap) => {
        if (!cancelled) setSnapshots((s) => ({ ...s, [activeTab]: snap }));
      });
    };
    run(false);
    const intervalSec = settings?.refreshIntervalSec ?? 300;
    const id = setInterval(() => run(true), Math.max(30, intervalSec) * 1000);
    return () => { cancelled = true; clearInterval(id); };
  }, [activeTab, settings?.refreshIntervalSec]);

  useEffect(() => {
    if (settings) {
      document.documentElement.style.setProperty("--widget-opacity", String(settings.opacity));
    }
  }, [settings?.opacity]);

  const viewMode: ViewMode = settings?.viewMode ?? "normal";
  const baseWidth = WIDTH_BY_MODE[viewMode];
  const effectiveWidth = menuOpen ? Math.max(baseWidth, 260) : baseWidth;

  useEffect(() => {
    if (!rootRef.current) return;
    const apply = async (el: HTMLElement) => {
      const h = el.scrollHeight;
      const withMenuH = menuOpen ? Math.max(h, 360) : h;
      const clampedH = Math.max(40, Math.min(900, Math.ceil(withMenuH)));
      try {
        await getCurrentWindow().setSize(new LogicalSize(effectiveWidth, clampedH));
      } catch {}
    };
    apply(rootRef.current);
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) apply(e.target as HTMLElement);
    });
    ro.observe(rootRef.current);
    return () => ro.disconnect();
  }, [effectiveWidth, baseWidth, activeTab, snapshots, viewMode, menuOpen]);

  const activeSnap = snapshots[activeTab];
  const activeFetchedAt = activeSnap ? new Date(activeSnap.fetchedAt) : null;
  const activeLabel = (() => {
    if (!activeFetchedAt) return "—";
    const diffSec = Math.floor((Date.now() - activeFetchedAt.getTime()) / 1000);
    if (diffSec < 60) return `${diffSec}초 전`;
    if (diffSec < 3600) return `${Math.floor(diffSec / 60)}분 전`;
    return `${Math.floor(diffSec / 3600)}시간 전`;
  })();

  const cycleProvider = () => {
    const idx = PROVIDER_ORDER.indexOf(activeTab);
    setActiveTab(PROVIDER_ORDER[(idx + 1) % PROVIDER_ORDER.length]);
  };

  const renderTabs = (compact: boolean) => (
    <div className={`flex border-b border-border/40 bg-surface-light/30 ${compact ? "text-[10px]" : ""}`}>
      {PROVIDER_ORDER.map((p) => (
        <button
          key={p}
          onClick={() => setActiveTab(p)}
          className={`flex-1 ${compact ? "px-1.5 py-1" : "px-2 py-1.5"} text-xs transition-colors ${
            activeTab === p
              ? "text-text border-b-2 border-accent -mb-[1px]"
              : "text-text-dim hover:text-text"
          }`}
        >
          {PROVIDER_LABELS[p]}
        </button>
      ))}
    </div>
  );

  const containerBase = "relative w-full flex flex-col";

  const handleBodyDragMouseDown = async (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    const el = e.target as HTMLElement | null;
    if (!el) return;
    if (el.closest("button, a, input, textarea, select, code, [data-no-drag]")) return;
    await getCurrentWindow().startDragging();
  };
  const bgStyle = { backgroundColor: `rgba(26, 26, 26, var(--widget-opacity, 0.92))` };

  const fadeClass = titleBarVisible ? "opacity-100" : "opacity-0 pointer-events-none";
  const header = (
    <div
      className={`transition-opacity duration-150 overflow-hidden rounded-t-xl border-x border-t border-border/60 ${fadeClass}`}
      style={bgStyle}
    >
      <Header
        onRefresh={() => fetchActive(true)}
        refreshing={refreshing[activeTab]}
        onOpenMenu={() => setMenuOpen(true)}
        lastUpdatedAt={activeFetchedAt}
        compact={viewMode !== "normal"}
      />
    </div>
  );
  const bodyOnlyClass = `flex flex-col overflow-hidden border-x border-border/60 ${
    titleBarVisible ? "" : "rounded-xl border-y"
  }`;

  if (viewMode === "super") {
    return (
      <div
        ref={rootRef}
        className={containerBase}
        onMouseDown={handleBodyDragMouseDown}
      >
        {header}
        <div
          data-window-drag-region="true"
          className={`flex flex-row items-center gap-2 px-2 py-1.5 overflow-x-auto border-x border-b border-border/60 ${titleBarVisible ? "rounded-b-xl" : "rounded-xl border-t"}`}
          style={bgStyle}
        >
          <button
            onClick={cycleProvider}
            className="flex-shrink-0 inline-flex items-center justify-center w-5 h-5 rounded bg-surface-light/60 text-[10px] font-semibold hover:bg-surface-light"
            title={`${PROVIDER_LABELS[activeTab]} (클릭해서 전환)`}
          >
            {PROVIDER_CODE[activeTab]}
          </button>
          {activeSnap?.response.status === "ok" && activeSnap.response.windows.length > 0 ? (
            activeSnap.response.windows.map((w) => (
              <MiniGauge key={w.key} window={w} provider={activeTab} />
            ))
          ) : activeSnap?.response.status === "expired" ? (
            <button
              onClick={async () => {
                try {
                  await ipc.refreshViaCli(activeTab);
                  const snap = await ipc.getProviderUsage(activeTab, true);
                  setSnapshots((s) => ({ ...s, [activeTab]: snap }));
                } catch (e) {
                  console.error("CLI refresh failed", e);
                }
              }}
              className="px-1.5 py-0.5 text-[10px] rounded bg-accent/20 hover:bg-accent/30 text-text"
              title="CLI로 토큰 갱신"
            >
              만료 ↻
            </button>
          ) : (
            <span className="text-[10px] text-text-dim">
              {activeSnap?.response.status === "not_authenticated" ? "로그인 안됨" : "—"}
            </span>
          )}
        </div>
        {menuOpen && settings && (
          <SettingsMenu settings={settings} onChange={setSettings} onClose={() => setMenuOpen(false)} />
        )}
      </div>
    );
  }

  const compact = viewMode === "mini";

  return (
    <div
      ref={rootRef}
      className={containerBase}
      onMouseDown={handleBodyDragMouseDown}
    >
      {header}
      <div
        className={`transition-opacity duration-150 overflow-hidden border-x border-border/60 ${fadeClass}`}
        style={bgStyle}
      >
        {renderTabs(compact)}
      </div>
      <div className={bodyOnlyClass} style={bgStyle}>
        <div
          className={compact ? "px-1.5 py-1.5" : "px-3 py-3"}
          style={compact ? { zoom: 0.75 } : undefined}
        >
          {!activeSnap && <div className="text-xs text-text-dim text-center py-4">로딩 중...</div>}
          {activeSnap && <ProviderCard data={activeSnap.response} />}
        </div>
      </div>
      <div
        className={`transition-opacity duration-150 overflow-hidden rounded-b-xl border-x border-b border-border/60 ${fadeClass}`}
        style={bgStyle}
      >
        <div className={`${compact ? "px-2 py-1" : "px-3 py-1.5"} border-t border-border/40 text-[10px] text-text-dim text-right`}>
          마지막 갱신: {activeLabel}
        </div>
      </div>
      {menuOpen && settings && (
        <SettingsMenu settings={settings} onChange={setSettings} onClose={() => setMenuOpen(false)} />
      )}
    </div>
  );
}
