# Claude Usage Widget

3-provider (Claude Code / Codex / Gemini) 사용량을 한눈에 보는 Windows 데스크톱 위젯.

프레임리스·투명·항상 위 플로팅 창으로 떠 있으면서, 각 provider의 OAuth 토큰을 직접 읽어 사용량 API를 호출합니다.

## Features

- **3 provider 통합 게이지**
  - Claude Code — Anthropic OAuth usage
  - Codex — `codex app-server` JSON-RPC `account/rateLimits/read`
  - Gemini — CloudCode `retrieveUserQuota`
- **탭 기반 per-provider 조회** — 활성 탭만 주기적으로 갱신, 스냅샷 디스크 캐시
- **시간 대비 잔량 시각화** — 예상 잔량 마커(시계 아이콘) + "여유" 하이라이트 스트라이프
- **3 뷰 모드** — 일반 / 미니 / 미니멀(가로 스트립)
- **타이틀바/탭/푸터 auto-hide** — 커서 호버 시 페이드 인
- **토큰 만료 시 CLI 갱신 버튼** — `claude` / `codex` / `gemini` spawn
- **위치·크기·갱신 시각 영속화**
- **Single-instance lock + 트레이 아이콘** — X 버튼 동작(종료/트레이) 설정 전환

## Install

[Releases](https://github.com/DevelopmentDummy/claude-usage-widget/releases)에서 `claude_usage.exe` 다운로드 후 실행.

## Dev

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

## Stack

Tauri 2 · Rust · React 19 · TypeScript · Vite · Tailwind 3.

## License

MIT
