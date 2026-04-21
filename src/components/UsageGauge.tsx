import type { UsageWindow } from "../lib/types";

function formatRemaining(resetsAt: string): string {
  const diff = new Date(resetsAt).getTime() - Date.now();
  if (diff <= 0) return "리셋 완료";
  const h = Math.floor(diff / 3_600_000);
  const m = Math.floor((diff % 3_600_000) / 60_000);
  if (h > 24) {
    const d = Math.floor(h / 24);
    return `${d}일 ${h % 24}시간 후 리셋`;
  }
  return h > 0 ? `${h}시간 ${m}분 후 리셋` : `${m}분 후 리셋`;
}

function gaugeColor(remain: number, expectedRemain: number) {
  if (remain < expectedRemain) {
    return remain < expectedRemain - 10
      ? { barOpacity: 0.25, labelOpacity: 0.4 }
      : { barOpacity: 0.5, labelOpacity: 0.65 };
  }
  return { barOpacity: 0.85, labelOpacity: 1 };
}

export default function UsageGauge({ window: w }: { window: UsageWindow }) {
  const remain = 100 - w.utilization;
  const expectedRemain = 100 - w.timeProgress;
  const colors = gaugeColor(remain, expectedRemain);

  return (
    <div className="mb-3">
      <div className="flex items-baseline justify-between mb-1">
        <span className="text-xs font-medium text-text">{w.name}</span>
        <span className="text-xs font-mono text-accent" style={{ opacity: colors.labelOpacity }}>
          {Math.round(remain)}% 남음
        </span>
      </div>
      <div className="relative h-4 rounded-full bg-surface-light">
        <div className="absolute inset-0 rounded-full overflow-hidden">
          <div
            className="absolute inset-y-0 left-0 rounded-full transition-all duration-500 bg-accent"
            style={{ opacity: colors.barOpacity, width: `${Math.max(remain, 0)}%` }}
          />
          {expectedRemain > 0 && expectedRemain < 100 && (
            <div className="absolute inset-y-0 left-0 bg-red-500/15 z-10" style={{ width: `${expectedRemain}%` }} />
          )}
          {remain > expectedRemain && expectedRemain > 0 && expectedRemain < 100 && (
            <div
              className="absolute inset-y-0 z-10 pointer-events-none rounded-r-full"
              style={{
                left: `${expectedRemain}%`,
                width: `${remain - expectedRemain}%`,
                background:
                  "repeating-linear-gradient(45deg, rgba(255,255,255,0.28) 0 2px, transparent 2px 6px)",
                boxShadow: "inset 0 0 6px rgba(255,255,255,0.25)",
              }}
            />
          )}
        </div>
        {expectedRemain > 0 && expectedRemain < 100 && (
          <>
            <div
              className="absolute inset-y-0 w-0.5 bg-white/50 z-20 pointer-events-none"
              style={{ left: `${expectedRemain}%` }}
            />
            <svg
              className="absolute z-30 pointer-events-none text-white/90 drop-shadow"
              viewBox="0 0 16 16"
              width="11"
              height="11"
              fill="#1a1a1a"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
              style={{
                left: `${expectedRemain}%`,
                top: "50%",
                transform: "translate(-50%, -50%)",
              }}
              aria-label={`예상 잔량 ${Math.round(expectedRemain)}%`}
            >
              <circle cx="8" cy="8" r="6.25" />
              <path d="M8 4.5V8l2.25 1.5" />
            </svg>
          </>
        )}
      </div>
      <div className="flex justify-between mt-0.5">
        <span className="text-[10px] text-text-dim">사용 {Math.round(w.utilization)}%</span>
        <span className="text-[10px] text-text-dim">{formatRemaining(w.resetsAt)}</span>
      </div>
    </div>
  );
}
