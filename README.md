# AutoTBH_Monitor

A **read-only** companion for **TBH: Task Bar Hero**, rebuilt in **Rust + Tauri + Nuxt 4**.
It reads your local game save (never modifies it), cross-references live Steam Market prices, and
surfaces a stash valuation, sell desk, market browser, bestiary, crafting, farm calibration, and the
latest patch notes — all in a native desktop window.

This is a from-scratch port of the original Electron/Node app: the entire backend is rewritten in Rust
(an embedded `axum` server), and the entire UI is rewritten in Nuxt 4 with `@nuxtjs/i18n` (16 locales).

> **Read-only & unaffiliated.** Reads the save file and public Steam Market data only. Not affiliated
> with Valve, Tesseract Studio, or the TBH developers.

---

## Architecture

```
┌─────────────────────────── Tauri (Rust) desktop shell ───────────────────────────┐
│                                                                                   │
│   src-tauri/  ──spawns──►  axum HTTP server @ 127.0.0.1:5260                       │
│        │                      ├─ /api/*        (Rust port of the Node backend)     │
│        │                      └─ /* static     (serves the built Nuxt SPA)         │
│        │                                                                           │
│        └── WebviewWindow ──loads──►  http://localhost:5260  (same origin as /api)  │
│                                          ▲                                         │
│                                   Nuxt 4 SPA (frontend/, generated to static)      │
└───────────────────────────────────────────────────────────────────────────────────┘
```

- **`src-tauri/`** — Rust. Tauri shell + embedded `axum` server. One binary, no Node at runtime.
- **`frontend/`** — Nuxt 4 SPA (`ssr: false`), `@nuxtjs/i18n`, generated to `.output/public` and served
  by the Rust server. The window loads `http://localhost:5260`, so the UI's `/api/*` calls are same-origin.
- **`data/`** — bundled game data: item-table + item-name seeds (for the save reader) and engine-derived
  snapshots (`codex`, `crafting`, `version`, `rune_tree`, `farm_stages`).

### Rust backend modules (`src-tauri/src/`)

| Module | Port of | Responsibility |
|--------|---------|----------------|
| `save.rs` | `tbh-save.mjs` | ES3 decrypt (AES-128-CBC + PBKDF2-HMAC-SHA1 + gzip), parse `PlayerSaveData`, aggregate the stash by market hash, read tabs. |
| `steam.rs` | `server.mjs` (network) | Steam Market: item list, order book, price history, name→hash resolve, Frankfurter FX, Steam news. |
| `pricing.rs` | `pricing.mjs` | Liquidity, undercut/suggested-list, fee math, history metrics, sell-now score. |
| `currency.rs` | `currency.mjs` | 41 Steam currencies + FX helpers. |
| `news.rs` | `news.mjs` | SteamDB patch-notes RSS + Steam news parsing (safe-HTML/BBCode reduction). |
| `server.rs` | `server.mjs` (routing) | `axum` router mirroring the `/api/*` contract + static SPA serving. |
| `lib.rs` / `main.rs` | `electron/main.js` | Tauri shell: boot the server, open the window on it. |

---

## Feature / port status

| Area | Endpoint(s) | Status |
|------|-------------|--------|
| Stash valuation | `/api/stash`, `/api/stash-tabs` | ✅ Rust (ES3 decrypt + market cross-ref) |
| Market browser | `/api/items` | ✅ Rust (Steam search/render) |
| Order book / hover / depth | `/api/orderbook`, `/api/hover`, `/api/market-depth` | ✅ Rust |
| Price history | `/api/pricehistory` | ✅ Rust |
| Name resolve | `/api/resolve-hash` | ✅ Rust |
| Currency (41) | `/api/currency` | ✅ Rust |
| Bestiary / stages | `/api/codex` | ✅ Bundled snapshot |
| Crafting | `/api/crafting` | ✅ Bundled snapshot |
| Rune tree (197 nodes) | `/api/rune-tree` | ✅ Bundled data |
| Updates | `/api/updates` | ✅ Rust (SteamDB + Steam news) |
| Farm calibration / runs | `/api/farm-calibration`, `/api/runs` | ⚙️ Wired; needs the meter reader's run logs |
| Coach / Upgrade Finder | `/api/insights`, `/api/upgrades` | 🚧 Depends on the TBH simulation engine — port in progress (see below) |
| Live DPS meter | `/api/meter` | ⚙️ Reads the reader's `live.json`; reader integration below |
| i18n (16 locales) | — | ✅ `@nuxtjs/i18n` (English + 简体中文 fully translated; others fall back to English + server-side game-name localization) |

### The simulation engine

The original's coach (`/api/insights`) and Upgrade Finder (`/api/upgrades`) are driven by a ~2 MB
JavaScript game-simulation engine (`engine.js` + `gamedata.js`). Faithfully reproducing party
DPS/EHP/POWER, farm modeling, rune/gear advising, idle/chest planning, and loot drop tables is a large
standalone effort. These endpoints currently return a structured `enginePending` marker and the UI
degrades gracefully. The planned path is to embed the vendored engine via a QuickJS runtime (`rquickjs`)
so it runs inside the Rust process without a Node dependency, then port hot paths to native Rust.

### Live DPS meter

The live DPS/gold/EXP overlay uses a native memory reader. This project is designed to consume the
MIT-licensed reader from [mad-labs-org/tbh-meter](https://github.com/mad-labs-org/tbh-meter) (a
read-only IL2CPP memory sensor): drop its `live.json` output into the app data dir and `/api/meter`
serves it. The reader itself is not bundled.

---

## Develop

Prerequisites: **Rust** (stable, MSVC), **Node 20+**, and the **Tauri v2** prerequisites (WebView2 —
preinstalled on Windows 11).

```bash
# 1) Frontend (Nuxt) — install + build the static SPA
cd frontend
npm install
npm run generate         # → frontend/.output/public

# 2) Run the desktop app (from repo root)
cd ../src-tauri
cargo run                # boots axum on :5260 and opens the window

# Or run the backend headless and open a browser at http://localhost:5260
cargo run --bin autotbh-monitor
```

### Build the installer

```bash
cargo tauri build        # runs `nuxt generate`, compiles Rust, emits an NSIS installer
```

Output: `src-tauri/target/release/bundle/nsis/*.exe`. CI (`.github/workflows/build.yml`) builds this on
every push.

---

## Data & i18n

- **Game data** (`data/engine/*.json`) is bundled so the app is useful offline — bestiary, stages,
  crafting, and the 197-node rune tree render without a network round-trip.
- **Localized game names** (items/monsters/stages) come from the wiki catalog's 16-language name maps,
  resolved server-side per request (`?lang=`).
- **UI chrome** is translated via `@nuxtjs/i18n` in `frontend/i18n.config.ts`.

## Configuration (env)

| Var | Default | Purpose |
|-----|---------|---------|
| `TSM_CURRENCY` | `1` (USD) | Initial Steam currency code. |
| `TBH_GAME_DIR` | auto-detected | Path to `TaskBarHero_Data` if not on a scanned drive. |
| `TBH_ES3_PASSWORD` | auto-extracted | Force the save decryption key. |
| `NUXT_PUBLIC_API_BASE` | `http://localhost:5260` | Backend base URL for the frontend (dev). |

## License

MIT — see [LICENSE](LICENSE). Bundled game data and localized strings originate from TBH: Task Bar Hero
and the community wiki and remain the property of their respective owners; this project is an
interoperable, read-only companion.
