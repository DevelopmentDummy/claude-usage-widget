import { useEffect } from "react";
import type { Settings, ViewMode } from "../lib/types";
import { ipc } from "../lib/ipc";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface Props {
  settings: Settings;
  onChange: (next: Settings) => void;
  onClose: () => void;
}

export default function SettingsMenu({ settings, onChange, onClose }: Props) {
  useEffect(() => {
    const keyHandler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    const blurHandler = () => onClose();
    window.addEventListener("keydown", keyHandler);
    window.addEventListener("blur", blurHandler);
    return () => {
      window.removeEventListener("keydown", keyHandler);
      window.removeEventListener("blur", blurHandler);
    };
  }, [onClose]);

  const update = async (patch: Partial<Settings>) => {
    const next = { ...settings, ...patch };
    onChange(next);
    await ipc.saveSettings(next);
    if ("alwaysOnTop" in patch) {
      await getCurrentWindow().setAlwaysOnTop(next.alwaysOnTop);
    }
    if ("autostart" in patch) {
      try { await ipc.setAutostart(next.autostart); }
      catch (e) { console.error("autostart failed", e); }
    }
  };

  return (
    <>
      <div
        data-no-drag
        className="fixed inset-0 z-40"
        onMouseDown={(e) => { e.stopPropagation(); onClose(); }}
      />
      <div
        data-no-drag
        onMouseDown={(e) => e.stopPropagation()}
        className="absolute right-2 top-10 z-50 w-60 bg-surface border border-border rounded-lg shadow-xl p-3 text-xs space-y-2.5"
      >
        <div>
          <div className="mb-1">뷰 모드</div>
          <div className="flex gap-1">
            {(["normal", "mini", "super"] as ViewMode[]).map((m) => (
              <button
                key={m}
                onClick={() => update({ viewMode: m })}
                className={`px-2 py-1 rounded flex-1 ${
                  settings.viewMode === m
                    ? "bg-accent/30 text-text"
                    : "bg-surface-light text-text-dim hover:bg-surface-light/70"
                }`}
              >
                {m === "normal" ? "일반" : m === "mini" ? "미니" : "미니멀"}
              </button>
            ))}
          </div>
        </div>

        <label className="flex items-center justify-between">
          <span>항상 위</span>
          <input
            type="checkbox"
            checked={settings.alwaysOnTop}
            onChange={(e) => update({ alwaysOnTop: e.target.checked })}
          />
        </label>

        <label className="flex items-center justify-between">
          <span>X 버튼으로 트레이로 숨기기</span>
          <input
            type="checkbox"
            checked={settings.closeToTray}
            onChange={(e) => update({ closeToTray: e.target.checked })}
          />
        </label>

        <label className="flex items-center justify-between">
          <span>시작 시 자동 실행</span>
          <input
            type="checkbox"
            checked={settings.autostart}
            onChange={(e) => update({ autostart: e.target.checked })}
          />
        </label>

        <div>
          <div className="flex items-center justify-between mb-1">
            <span>불투명도</span>
            <span className="text-text-dim">{Math.round(settings.opacity * 100)}%</span>
          </div>
          <input
            type="range"
            min="0.5"
            max="1"
            step="0.01"
            value={settings.opacity}
            onChange={(e) => update({ opacity: parseFloat(e.target.value) })}
            className="w-full"
          />
        </div>

        <div>
          <div className="mb-1">자동 갱신 간격 (재시작 필요)</div>
          <div className="flex gap-1">
            {[
              { l: "1분", v: 60 },
              { l: "5분", v: 300 },
              { l: "15분", v: 900 },
            ].map((opt) => (
              <button
                key={opt.v}
                onClick={() => update({ refreshIntervalSec: opt.v })}
                className={`px-2 py-1 rounded flex-1 ${
                  settings.refreshIntervalSec === opt.v
                    ? "bg-accent/30 text-text"
                    : "bg-surface-light text-text-dim hover:bg-surface-light/70"
                }`}
              >
                {opt.l}
              </button>
            ))}
          </div>
        </div>

        <div className="pt-2 border-t border-border/40 flex justify-between items-center">
          <span className="text-text-dim">v0.2.0</span>
          <button
            onClick={() => ipc.openUrl("https://github.com/")}
            className="text-accent hover:underline"
          >
            GitHub
          </button>
        </div>
      </div>
    </>
  );
}
