import { useState } from "react";
import type { Provider, UsageResponse } from "../lib/types";
import UsageGauge from "./UsageGauge";
import { ipc } from "../lib/ipc";

const COLORS: Record<Provider, string> = {
  claude: "#ff9f43",
  codex: "#4dff91",
  gemini: "#64b5f6",
};

const LABELS: Record<Provider, string> = {
  claude: "Claude",
  codex: "Codex",
  gemini: "Gemini",
};

const LOGIN_CMD: Record<Provider, string> = {
  claude: "claude login",
  codex: "codex login",
  gemini: "gemini",
};

export default function ProviderCard({ data }: { data: UsageResponse }) {
  const [refreshing, setRefreshing] = useState(false);
  const [cliError, setCliError] = useState<string | null>(null);

  const triggerCliRefresh = async () => {
    setRefreshing(true);
    setCliError(null);
    try {
      await ipc.refreshViaCli(data.provider);
      await ipc.getProviderUsage(data.provider, true);
    } catch (e) {
      setCliError(String(e));
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <div className="mb-4 last:mb-0">
      <div className="flex items-center gap-2 mb-2">
        <span className="inline-block w-2 h-2 rounded-full" style={{ backgroundColor: COLORS[data.provider] }} />
        <span className="text-xs text-text-dim">{LABELS[data.provider]}</span>
        {data.status === "network_error" && (
          <span title={data.error} className="text-xs text-yellow-500">⚠</span>
        )}
      </div>

      {data.status === "not_authenticated" && (
        <div className="text-xs text-text-dim">
          <div className="mb-1">로그인되지 않음</div>
          <code className="text-[10px] bg-surface-light px-1.5 py-0.5 rounded">{LOGIN_CMD[data.provider]}</code>
        </div>
      )}

      {data.status === "expired" && (
        <div className="text-xs">
          <div className="mb-1.5 text-text-dim">토큰 만료</div>
          <button
            onClick={triggerCliRefresh}
            disabled={refreshing}
            className="px-2 py-1 text-[11px] rounded bg-accent/20 hover:bg-accent/30 disabled:opacity-50"
          >
            {refreshing ? "갱신 중... (최대 15초)" : "CLI로 갱신"}
          </button>
          {cliError && (
            <div className="mt-1.5 text-[10px] text-red-400 break-all">
              <div>CLI 실행 실패 — 수동 로그인 필요: <code>{LOGIN_CMD[data.provider]}</code></div>
              <div className="mt-0.5 opacity-80">{cliError}</div>
            </div>
          )}
        </div>
      )}

      {data.status === "ok" && data.windows.map((w) => (
        <UsageGauge key={`${data.provider}-${w.key}`} window={w} />
      ))}

      {data.status !== "ok" && data.error && (
        <div className="mt-1 text-[10px] text-red-400 break-all">
          [{data.status}] {data.error}
        </div>
      )}

      {data.status === "ok" && data.extraUsage?.isEnabled && (
        <div className="mt-1 pt-1 border-t border-border/40">
          <div className="flex items-baseline justify-between">
            <span className="text-xs text-text-dim">추가 사용</span>
            <span className="text-xs font-mono text-text-dim">
              ${data.extraUsage.usedCredits.toFixed(2)}
              {data.extraUsage.monthlyLimit > 0 && ` / $${data.extraUsage.monthlyLimit.toLocaleString()}`}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
