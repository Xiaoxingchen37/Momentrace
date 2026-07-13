# Momentrace（时迹）

Windows-only, local-first desktop app usage tracking. It records foreground **applications only** (never window titles, keystrokes, browser URLs, or cloud data).

## Prerequisites

- Node.js 20+
- [Rust stable](https://rustup.rs/) with the MSVC Windows target
- Microsoft C++ Build Tools / Windows SDK required by Tauri

## Run

```powershell
npm install
npm run tauri dev
```

## Build

```powershell
npm run tauri build
```

All data is stored in the user-local app-data directory under `com.local.screentime`.
