import type { UsageResponse } from "../lib/types";

export default function MissingGeminiWindows({ data }: { data?: UsageResponse }) {
  if (!data || data.provider !== "gemini" || data.status !== "ok") return null;
  // Legacy Gemini CLI quotas use model tiers, not Antigravity time windows.
  if (data.windows.some((w) => ["flash", "pro", "claude", "gpt"].includes(w.key))) return null;
  return <>{[["gemini-5h", "Gemini 5시간"], ["gemini-weekly", "Gemini 7일"]]
    .filter(([key]) => !data.windows.some((w) => w.key === key))
    .map(([key, label]) => (
      <div key={key} className="text-[10px] text-text-dim" title="API에서 해당 기간의 사용량을 제공하지 않았습니다">
        {label}: 정보 없음
      </div>
    ))}</>;
}
