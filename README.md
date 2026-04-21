# Claude Usage Widget

3-provider (Claude Code / Codex / Gemini) 사용량을 한눈에 보는 Windows 데스크톱 위젯.

프레임리스·투명·항상 위 플로팅 창으로 떠 있으면서, 각 provider의 OAuth 토큰을 직접 읽어 사용량 API를 호출합니다. CLI 실행은 토큰 갱신이 필요할 때만.

## Features

- **3 provider 통합 게이지**
  - Claude Code — Anthropic OAuth usage
  - Codex — `codex app-server` JSON-RPC `account/rateLimits/read`
  - Gemini — CloudCode `retrieveUserQuota`
- **탭 기반 per-provider 조회** — 활성 탭만 `refreshIntervalSec` 주기로 갱신. 스냅샷은 디스크 캐시돼 재실행 시 즉시 복원
- **시간 진행률 대비 잔량 시각화** — 게이지에 예상 잔량 마커(시계 아이콘)와 "여유" 하이라이트 스트라이프
- **3 뷰 모드**
  - 일반 — 전체 카드 뷰
  - 미니 — 절반 크기 컴팩트 뷰
  - 미니멀 — 가로 한 줄 스트립 (provider 코드 + 미니 게이지)
- **타이틀바/탭/푸터 auto-hide** — 창에 커서 올리면 페이드 인, 벗어나면 페이드 아웃 (드래그 중에도 유지)
- **토큰 만료 시 CLI 갱신 버튼** — `claude.cmd` / `codex.cmd` / `gemini.cmd` spawn, 토큰 파일 mtime 검증
- **위치·크기·갱신 시각 영속화** — `%APPDATA%\com.example.claudeusagewidget\` 에 저장
- **single-instance lock + tray** — 트레이 좌클릭 토글, 우클릭 메뉴(보이기/숨기기/종료). X 버튼 동작(종료/트레이)은 설정에서 전환
- **NSIS 인스톨러** — 태그 푸시 시 GitHub Actions로 자동 빌드·릴리스

## Install

[Releases](https://github.com/DevelopmentDummy/claude-usage-widget/releases)에서 최신 `claude-usage-widget_x.y.z_x64-setup.exe` 다운로드 후 실행.

혹은 포터블: `claude_usage.exe` (릴리스에서 함께 배포하거나 직접 빌드).

## Dev

```bash
npm install
npm run tauri dev
```

요구 사항: Node 20+, Rust stable, Windows x64.

## Build

```bash
npm run tauri build
```

산출물:
- `src-tauri/target/release/claude_usage.exe` — 포터블
- `src-tauri/target/release/bundle/nsis/claude-usage-widget_x.y.z_x64-setup.exe` — 설치 프로그램

## Release

`v*` 태그 푸시 시 `.github/workflows/release.yml`이 Windows에서 빌드해 draft release로 NSIS 인스톨러를 업로드합니다.

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Stack

Tauri 2 · Rust · React 19 · TypeScript · Vite · Tailwind 3.

## License

MIT
