# AutoTBH_Monitor

A **read-only** desktop companion for **TBH: Task Bar Hero**, built with **Rust + Tauri + Nuxt 4**.

It reads your local game save (never modifies it), cross-references live Steam Market prices, and
provides stash valuation, sell guidance, market browsing, bestiary, crafting, farm calibration, and
patch notes in a native window. The UI supports multiple languages (including full Simplified Chinese).

> **Read-only & unaffiliated.** Reads the save file and public Steam Market data only. Not affiliated
> with Valve, Tesseract Studio, or the TBH developers.

[中文](../README.md) · [Architecture & features](architecture.en.md)

---

## Features

- **Stash valuation** — read the save and value inventory / tabs against Steam Market prices
- **Sell desk** — liquidity, suggested list price, fees, and sell-now score
- **Market browser** — item search, order book, price history
- **Bestiary / crafting / rune tree** — offline-friendly game data browsing
- **Updates** — SteamDB patch notes and Steam news
- **Live meter (built in)** — live DPS / gold / EXP plus a run tracker. Rewritten natively in Rust
  from the MIT-licensed [tbh-meter](https://github.com/mad-labs-org/tbh-meter), so **no Python and no
  external process is needed**; strictly read-only (`ReadProcessMemory` only — no writes, no injection)
- **Farm calibration** — driven by the real run data the built-in meter records

Some capabilities (Coach, Upgrade Finder, etc.) are still being ported — see
[Architecture & features](architecture.en.md).

---

## Usage

### Prerequisites

- **Rust** (stable; MSVC toolchain on Windows)
- **Node 20+**
- **Tauri v2** prerequisites (WebView2 — preinstalled on Windows 11)

### Run locally

```bash
# 1) Build the frontend static SPA
cd frontend
npm install
npm run generate         # → frontend/.output/public

# 2) Launch the desktop app (from repo root)
cd ../src-tauri
cargo run                # boots the server on :5260 and opens the window

# Or run the backend headless and open http://localhost:5260 in a browser
cargo run --bin autotbh-monitor
```

### Build the installer

```bash
cargo tauri build        # generates the frontend, compiles Rust, emits an NSIS installer
```

Output: `src-tauri/target/release/bundle/nsis/*.exe`. CI builds this on every push.

### Environment variables (optional)

| Var | Default | Purpose |
|-----|---------|---------|
| `TSM_CURRENCY` | `1` (USD) | Initial Steam currency code |
| `TBH_GAME_DIR` | auto-detected | Path to `TaskBarHero_Data` |
| `TBH_ES3_PASSWORD` | auto-extracted | Force the save decryption key |
| `NUXT_PUBLIC_API_BASE` | `http://localhost:5260` | Backend base URL for the frontend (dev) |

---

## Docs

| Doc | Description |
|-----|-------------|
| [Architecture & features](architecture.en.md) | Technical architecture, modules, endpoints, port status |
| [中文 README](../README.md) | Chinese project intro & usage |
| [架构与功能说明](architecture.md) | Chinese architecture & feature status |

---

## License

MIT — see [LICENSE](../LICENSE). Bundled game data and localized strings originate from TBH: Task Bar Hero
and the community wiki and remain the property of their respective owners; this project is an
interoperable, read-only companion.
