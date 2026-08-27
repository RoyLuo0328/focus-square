# Focus Square

一个轻量、跨平台、隐私优先的番茄钟。Focus Square 提供可配置计时、每日/每周/自定义周期统计、本地专注习惯分析和可选兼容 AI 建议。

Focus Square is a lightweight, cross-platform, privacy-first Pomodoro timer with configurable sessions, local focus analytics, and optional OpenAI-compatible advice.

## Features

- 260×260 translucent timer with a topmost 420×420 completion reminder
- Configurable focus, short-break, long-break, and cycle durations
- Local SQLite history with accurate active-time segments
- Daily, weekly, and custom-period reports with previous-period comparison
- Explainable local habit insights; no account or cloud required
- Optional user-configured `/chat/completions` endpoint; only aggregated metrics are sent
- Chinese/English UI, macOS menu bar and Windows system tray

## Development

Requirements: Node.js 24+, Rust stable, and the platform prerequisites from the Tauri 2 documentation.

```bash
npm install
npm run tauri dev
```

Checks:

```bash
npm run build
npm test
npm run tauri build
```

## Signed releases

Pushing a `v*` tag creates a draft GitHub release with a universal macOS DMG and a Windows x64 NSIS installer. The release workflow intentionally stops if signing credentials are absent.

- macOS: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`
- Windows: `WINDOWS_CERTIFICATE_BASE64` (base64-encoded PFX) and `WINDOWS_CERTIFICATE_PASSWORD`

## Privacy

Focus history stays in the local app-data directory. AI analysis is never automatic. When requested, the app sends only report-level aggregates and local findings to the configured compatible endpoint. API keys are stored in macOS Keychain or Windows Credential Manager.

## License

MIT
