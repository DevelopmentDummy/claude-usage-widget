import type { UsageWindow } from "../lib/types";
import { formatRemaining } from "../lib/format";

function shortLabel(name: string, provider: string): string {
  if (provider === "gemini") {
    if (name === "Flash Lite") return "FL";
    if (name === "Flash") return "F";
    if (name === "Pro") return "P";
    return name.slice(0, 2).toUpperCase();
  }
  const hourMatch = name.match(/(\d+)\s*시간(?:\s*\(([^)]+)\))?/);
  if (hourMatch) return hourMatch[2] ? `${hourMatch[1]}h(${hourMatch[2][0].toLowerCase()})` : `${hourMatch[1]}h`;
  const dayMatch = name.match(/(\d+)\s*일(?:\s*\(([^)]+)\))?/);
  if (dayMatch) return dayMatch[2] ? `${dayMatch[1]}d(${dayMatch[2][0].toLowerCase()})` : `${dayMatch[1]}d`;
  return name.slice(0, 3);
}

export default function MiniGauge({
  window: w,
  provider,
}: {
  window: UsageWindow;
  provider: string;
}) {
  const remain = 100 - w.utilization;
  const expectedRemain = 100 - w.timeProgress;
  let barOpacity = 1;
  if (remain < expectedRemain) {
    barOpacity = remain < expectedRemain - 10 ? 0.25 : 0.55;
  }
  const tooltip = `${w.name} · 사용 ${w.utilization.toFixed(1)}% · ${remain.toFixed(1)}% 남음 · ${formatRemaining(w.resetsAt)}`;
  return (
    <span className="inline-flex items-center gap-1 text-[10px]" title={tooltip}>
      <span className="text-text-dim">{shortLabel(w.name, provider)}</span>
      <span className="inline-flex items-center relative w-[28px] h-[8px] rounded-sm bg-surface-light overflow-hidden">
        <span
          className="absolute inset-y-0 left-0 rounded-sm bg-accent"
          style={{ opacity: barOpacity, width: `${Math.max(remain, 0)}%` }}
        />
        {expectedRemain > 0 && expectedRemain < 100 && (
          <span
            className="absolute inset-y-0 w-0.5 bg-white/70"
            style={{ left: `${expectedRemain}%` }}
          />
        )}
      </span>
    </span>
  );
}
