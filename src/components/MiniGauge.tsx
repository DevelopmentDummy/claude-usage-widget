import type { UsageWindow } from "../lib/types";

function shortLabel(name: string, provider: string): string {
  if (provider === "gemini") {
    if (name === "Flash Lite") return "FL";
    if (name === "Flash") return "F";
    if (name === "Pro") return "P";
    return name.slice(0, 2).toUpperCase();
  }
  const hourMatch = name.match(/(\d+)\s*시간/);
  if (hourMatch) return `${hourMatch[1]}h`;
  const dayMatch = name.match(/(\d+)\s*일(?:\s*\((\w)\w*\))?/);
  if (dayMatch) return dayMatch[2] ? `${dayMatch[1]}d(${dayMatch[2].toLowerCase()})` : `${dayMatch[1]}d`;
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
  return (
    <span className="inline-flex items-center gap-1 text-[10px]">
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
