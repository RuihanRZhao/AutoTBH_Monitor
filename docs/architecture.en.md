# Architecture & features

Technical architecture, backend modules, endpoints, and port status for AutoTBH_Monitor.
For the project intro and usage guide, see [README.en.md](README.en.md).

[中文版](architecture.md)

---

## Architecture

This is a from-scratch port of the original Electron/Node app: the backend is Rust (embedded `axum`),
and the UI is Nuxt 4 with `@nuxtjs/i18n` (16 locales).

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

- **`src-tauri/`** — Tauri shell + embedded `axum`. One binary, no Node at runtime.
- **`frontend/`** — Nuxt 4 SPA (`ssr: false`), generated to `.output/public` and served by the Rust
  server. The window loads `http://localhost:5260`, so `/api/*` calls are same-origin.
- **`data/`** — bundled game data: item-table + item-name seeds (for the save reader) and
  engine-derived snapshots (`codex`, `crafting`, `version`, `rune_tree`, `farm_stages`).

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
| Farm calibration / runs | `/api/farm-calibration`, `/api/runs` | ✅ Rust (fed by the built-in meter) |
| Coach / Upgrade Finder | `/api/insights`, `/api/upgrades` | 🚧 Depends on the TBH simulation engine — port in progress |
| Live meter (built in) | `/api/meter`, `/api/meter/status` | ⚙️ Memory reader + metrics implemented in Rust and verified live; needs a per-build calibration |
| i18n (16 locales) | — | ✅ `@nuxtjs/i18n` (English + 简体中文 fully translated; others fall back to English + server-side game-name localization) |

### The simulation engine

The original's coach (`/api/insights`) and Upgrade Finder (`/api/upgrades`) are driven by a ~2 MB
JavaScript game-simulation engine (`engine.js` + `gamedata.js`). Faithfully reproducing party
DPS/EHP/POWER, farm modeling, rune/gear advising, idle/chest planning, and loot drop tables is a large
standalone effort. These endpoints currently return a structured `enginePending` marker and the UI
degrades gracefully. The planned path is to embed the vendored engine via a QuickJS runtime (`rquickjs`)
so it runs inside the Rust process without a Node dependency, then port hot paths to native Rust.

### Live meter (built-in sub-feature)

The live DPS / gold / EXP meter and run tracker are **absorbed into this project as a first-class
sub-feature**, not consumed as an external dependency. The functionality originates in the
MIT-licensed [mad-labs-org/tbh-meter](https://github.com/mad-labs-org/tbh-meter) (originally a
Python `tbh-reader.exe` sidecar plus an Electron overlay) and has been reimplemented natively in
Rust — see [NOTICE](../NOTICE) for the required attribution.

**Design**

- **No external dependency** — no Python, no PyInstaller, no sidecar process. The sensor
  (`memory.rs` / `meter.rs`) compiles into the main binary.
- **Strictly read-only** — the process is opened with `PROCESS_VM_READ |
  PROCESS_QUERY_INFORMATION` and only `ReadProcessMemory` is used. **Write access is never
  requested and no code is injected.** This is upstream's stated safety boundary and is preserved.
- **Class resolution** matches upstream: a build-pinned **RVA anchor → IL2CPP TypeInfoTable →
  TypeDefIndex**. Classes are chosen **by index**; names are only used to *validate* (upstream's
  NAME-FREE invariant). This codebase does **not** use AOB signature scanning.
- **Offsets are data, not code** — all IL2CPP/game offsets, enums, and per-build calibrations live
  in `data/meter-offsets.json`, keyed by **build fingerprint**
  (`<version>-<PE.TimeDateStamp>-<PE.SizeOfImage>`). A game update needs a re-pin, not a rebuild
  (upstream documented at least five offset shifts).
- **All-or-nothing resolution** — any validation failure refuses to serve data and reports why;
  partially resolved values are never emitted.
- **"Unread" is never "zero"** — a failed read stays `null`. Upstream traced a whole class of
  corruption to unread gold being recorded as `0`, indistinguishable from a real zero.

**Metrics**

- **Damage / DPS** — the game exposes no damage counter, so damage is inferred from **monster HP
  deltas**: per tick, sum the HP drops of live monsters and credit the remaining HP of any monster
  that vanished (the killing blow). DPS uses a 5-second rolling window.
- **Kills** — accumulated from shrinkage of the live monster list.
- **Gold** — `AggregateManager → AGGREGATES → GoldEarn(2) → SubKey 1`. **SubKey 0 is a rollup
  (combat + sale + idle + quest) and must never be used, nor may the subkeys be summed.**
- **Stage** — the statistical mode of `Monster.STAGE_KEY` across live monsters (the save's current
  stage lags on a stage change).
- **Party** — `StageManager → HERO_LIST`, read in formation-slot order.

**Status**

Process attach, module base resolution, PE build fingerprinting, calibration matching, and the
read-only primitives (IL2CPP string / array / `List<T>` / both `Dictionary` geometries) are
implemented and **verified against the running game**. `data/meter-offsets.json` ships a
calibration seed for game build **1.00.27**; on a different build the meter attaches successfully
but explicitly reports the missing calibration instead of emitting wrong numbers.

Not yet ported from upstream: ACTk-obscured XP decoding, exact run start/close detection via
`LogManager` log-object pointers (currently approximated from monster presence and stage change),
and the `{ok, value}` envelope serialization format.

---

## Data & i18n

- **Game data** (`data/engine/*.json`) is bundled so the app is useful offline — bestiary, stages,
  crafting, and the 197-node rune tree render without a network round-trip.
- **Localized game names** (items/monsters/stages) come from the wiki catalog's 16-language name maps,
  resolved server-side per request (`?lang=`).
- **UI chrome** is translated via `@nuxtjs/i18n` in `frontend/i18n.config.ts`.
