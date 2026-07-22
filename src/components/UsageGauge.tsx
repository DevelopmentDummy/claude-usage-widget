import { useState } from "react";
import type { UsageWindow } from "../lib/types";
import { formatRemaining } from "../lib/format";

// 리셋 근접 시 사용량 바와 리셋 타이머 마커가 둘 다 왼쪽(바닥) 근처로 몰려
// 미묘한 차이를 읽기 어렵다. 호버하면 "남은 리셋 타이머"(expectedRemain)를
// 게이지의 이 지점(%)까지 끌어올리는 선형 리스케일을 적용해 저부를 확대한다.
const RESCALE_ANCHOR = 60;

function gaugeColor(remain: number, expectedRemain: number) {
  if (remain < expectedRemain) {
    return remain < expectedRemain - 10
      ? { barOpacity: 0.25, labelOpacity: 0.4 }
      : { barOpacity: 0.5, labelOpacity: 0.65 };
  }
  return { barOpacity: 0.85, labelOpacity: 1 };
}

export default function UsageGauge({ window: w }: { window: UsageWindow }) {
  const [hovered, setHovered] = useState(false);

  const remain = 100 - w.utilization;
  const expectedRemain = 100 - w.timeProgress;
  const colors = gaugeColor(remain, expectedRemain);

  // 타이머가 아직 전체의 60%보다 위에 있으면(리셋 직후 등) 확대하지 않는다.
  const canRescale = expectedRemain > 0 && expectedRemain < RESCALE_ANCHOR;
  const rescaled = hovered && canRescale;
  const scale = rescaled ? RESCALE_ANCHOR / expectedRemain : 1;
  // 게이지 위치(%)를 확대 배율만큼 늘리고 0~100으로 클램프.
  const sx = (v: number) => Math.min(100, Math.max(v, 0) * scale);

  const expShown = sx(expectedRemain);
  const remainShown = sx(remain);

  return (
    <div
      className="mb-3"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div className="flex items-baseline justify-between mb-1">
        <span className="text-xs font-medium text-text">{w.name}</span>
        <span className="flex items-baseline gap-1">
          {rescaled && (
            <span
              className="text-[9px] font-mono text-accent/70 px-1 rounded bg-accent/10"
              title={`리셋 임박 확대 보기 — 남은 타이머(${expectedRemain.toFixed(1)}%)를 게이지 ${RESCALE_ANCHOR}% 위치로 확대 (×${scale.toFixed(1)})`}
            >
              ×{scale.toFixed(1)} 확대
            </span>
          )}
          <span className="text-xs font-mono text-accent" style={{ opacity: colors.labelOpacity }}>
            {remain.toFixed(1)}% 남음
          </span>
        </span>
      </div>
      <div className="relative h-4 rounded-full bg-surface-light">
        <div className="absolute inset-0 rounded-full overflow-hidden">
          <div
            className="absolute inset-y-0 left-0 rounded-full transition-all duration-500 bg-accent"
            style={{ opacity: colors.barOpacity, width: `${Math.max(remainShown, 0)}%` }}
          />
          {expectedRemain > 0 && expectedRemain < 100 && (
            <div
              className="absolute inset-y-0 left-0 bg-red-500/15 z-10 transition-all duration-500"
              style={{ width: `${expShown}%` }}
            />
          )}
          {remain > expectedRemain && expectedRemain > 0 && expectedRemain < 100 && (
            <div
              className="absolute inset-y-0 z-10 pointer-events-none rounded-r-full transition-all duration-500"
              style={{
                left: `${expShown}%`,
                width: `${Math.max(remainShown - expShown, 0)}%`,
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
              className="absolute inset-y-0 w-0.5 bg-white/50 z-20 pointer-events-none transition-all duration-500"
              style={{ left: `${expShown}%` }}
            />
            <svg
              className="absolute z-30 pointer-events-none text-white/90 drop-shadow transition-all duration-500"
              viewBox="0 0 16 16"
              width="11"
              height="11"
              fill="#1a1a1a"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
              style={{
                left: `${expShown}%`,
                top: "50%",
                transform: "translate(-50%, -50%)",
              }}
              aria-label={`예상 잔량 ${expectedRemain.toFixed(1)}%`}
            >
              <circle cx="8" cy="8" r="6.25" />
              <path d="M8 4.5V8l2.25 1.5" />
            </svg>
          </>
        )}
      </div>
      <div className="flex justify-between mt-0.5">
        <span className="text-[10px] text-text-dim">사용 {w.utilization.toFixed(1)}%</span>
        <span className="text-[10px] text-text-dim">{formatRemaining(w.resetsAt)}</span>
      </div>
    </div>
  );
}
