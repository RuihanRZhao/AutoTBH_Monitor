# 架构与功能说明

本文档说明 AutoTBH_Monitor 的技术架构、后端模块、接口与移植状态。项目介绍与使用说明见 [README](../README.md)。

[English version](architecture.en.md)

---

## 架构

本项目是原 Electron/Node 应用的从头移植：后端为 Rust（内嵌 `axum`），前端为 Nuxt 4 + `@nuxtjs/i18n`（16 种语言）。

```
┌─────────────────────────── Tauri (Rust) 桌面壳 ───────────────────────────┐
│                                                                           │
│   src-tauri/  ──启动──►  axum HTTP 服务 @ 127.0.0.1:5260                    │
│        │                    ├─ /api/*        （Node 后端的 Rust 移植）       │
│        │                    └─ /* 静态资源   （提供构建后的 Nuxt SPA）        │
│        │                                                                   │
│        └── WebviewWindow ──加载──►  http://localhost:5260  （与 /api 同源） │
│                                          ▲                                 │
│                                   Nuxt 4 SPA（frontend/，生成静态资源）      │
└───────────────────────────────────────────────────────────────────────────┘
```

- **`src-tauri/`** — Tauri 壳 + 内嵌 `axum`。单一可执行文件，运行时不依赖 Node。
- **`frontend/`** — Nuxt 4 SPA（`ssr: false`），生成到 `.output/public` 并由 Rust 服务托管。窗口加载 `http://localhost:5260`，`/api/*` 为同源请求。
- **`data/`** — 捆绑游戏数据：物品表与物品名种子（存档读取），以及引擎导出快照（`codex`、`crafting`、`version`、`rune_tree`、`farm_stages`）。

### Rust 后端模块（`src-tauri/src/`）

| 模块 | 对应原文件 | 职责 |
|------|------------|------|
| `save.rs` | `tbh-save.mjs` | ES3 解密（AES-128-CBC + PBKDF2-HMAC-SHA1 + gzip），解析 `PlayerSaveData`，按市场 hash 聚合仓库，读取页签。 |
| `steam.rs` | `server.mjs`（网络部分） | Steam 市场：物品列表、订单簿、价格历史、名称→hash 解析、Frankfurter 汇率、Steam 新闻。 |
| `pricing.rs` | `pricing.mjs` | 流动性、压价/建议挂单价、手续费、历史指标、立即出售评分。 |
| `currency.rs` | `currency.mjs` | 41 种 Steam 货币 + 汇率辅助。 |
| `news.rs` | `news.mjs` | SteamDB 补丁 RSS + Steam 新闻解析（安全 HTML/BBCode 精简）。 |
| `server.rs` | `server.mjs`（路由） | `axum` 路由，镜像 `/api/*` 契约 + 静态 SPA 托管。 |
| `lib.rs` / `main.rs` | `electron/main.js` | Tauri 壳：启动服务并打开窗口。 |

---

## 功能 / 移植状态

| 功能 | 接口 | 状态 |
|------|------|------|
| 仓库估值 | `/api/stash`、`/api/stash-tabs` | ✅ Rust（ES3 解密 + 市场交叉对照） |
| 市场浏览器 | `/api/items` | ✅ Rust（Steam 搜索/渲染） |
| 订单簿 / 悬停 / 深度 | `/api/orderbook`、`/api/hover`、`/api/market-depth` | ✅ Rust |
| 价格历史 | `/api/pricehistory` | ✅ Rust |
| 名称解析 | `/api/resolve-hash` | ✅ Rust |
| 货币（41 种） | `/api/currency` | ✅ Rust |
| 图鉴 / 关卡 | `/api/codex` | ✅ 捆绑快照 |
| 合成 | `/api/crafting` | ✅ 捆绑快照 |
| 符文树（197 节点） | `/api/rune-tree` | ✅ 捆绑数据 |
| 更新动态 | `/api/updates` | ✅ Rust（SteamDB + Steam 新闻） |
| 刷图校准 / 记录 | `/api/farm-calibration`、`/api/runs` | ✅ Rust（数据来自内置实时面板） |
| Coach / 升级推荐 | `/api/insights`、`/api/upgrades` | 🚧 依赖 TBH 模拟引擎 — 移植进行中 |
| 实时面板（内置） | `/api/meter`、`/api/meter/status` | ⚙️ 内存读取与指标已用 Rust 实现并实机验证；需按游戏版本补标定 |
| i18n（16 种语言） | — | ✅ `@nuxtjs/i18n`（English + 简体中文完整；其余回退英文 + 服务端游戏名本地化） |

### 模拟引擎

原版 coach（`/api/insights`）与升级推荐（`/api/upgrades`）由约 2 MB 的 JavaScript 游戏模拟引擎（`engine.js` + `gamedata.js`）驱动。完整复现队伍 DPS/EHP/POWER、刷图建模、符文/装备建议、挂机/宝箱规划与掉落表是一项独立的大型工作。这些接口目前返回结构化的 `enginePending` 标记，UI 会优雅降级。计划通过 QuickJS（`rquickjs`）嵌入已 vendored 的引擎，在 Rust 进程内运行且不依赖 Node，再将热点路径移植为原生 Rust。

### 实时面板（内置子功能）

实时 DPS / 金币 / 经验面板与战斗记录器的功能来源于 MIT 许可的
[mad-labs-org/tbh-meter](https://github.com/mad-labs-org/tbh-meter)（原实现为 Python
`tbh-reader.exe` 旁路进程 + Electron 覆盖层），现已**完整并入本项目并用原生 Rust 重写**，
成为一级子功能（见 [NOTICE](../NOTICE) 的署名要求）。

**设计要点**

- **无外部依赖** — 不需要 Python、PyInstaller 或任何旁路进程；传感器（`memory.rs` /
  `meter.rs`）就是主程序的一部分。
- **严格只读** — 仅以 `PROCESS_VM_READ | PROCESS_QUERY_INFORMATION` 打开进程，只调用
  `ReadProcessMemory`；**从不请求写权限、从不注入代码**。这是上游明确的安全边界，移植时予以保留。
- **类解析** — 与上游一致：**固定 RVA 锚点 → IL2CPP TypeInfoTable → TypeDefIndex**，
  按**索引**取类，类名仅用于**校验**而非查找（上游的 NAME-FREE 不变量）。本代码库**不使用** AOB 特征扫描。
- **偏移即数据** — IL2CPP 与游戏偏移、枚举、每版本标定全部放在 `data/meter-offsets.json`，
  按**构建指纹**（`版本-PE.TimeDateStamp-PE.SizeOfImage`）索引。游戏更新后重新标定即可，无需重新编译
  （上游记录过至少五次偏移漂移）。
- **全有或全无** — 任一校验失败即拒绝提供数据并说明原因，**绝不输出半解析的错误数值**。
- **不把「读取失败」当作 0** — 读不到的字段保持 `null`。上游曾因把未读到的金币记为 0
  而产生无法与真实 0 区分的污染数据。

**指标推算**

- **伤害 / DPS** — 游戏不暴露伤害计数器，伤害由**怪物血量下降量**推算：每 tick 比较存活怪物血量并累加降幅，
  本 tick 消失的怪物按其剩余血量计入（击杀伤害）。DPS 使用 5 秒滚动窗口。
- **击杀数** — 由存活怪物列表的收缩量累计。
- **金币** — `AggregateManager → AGGREGATES → GoldEarn(2) → SubKey 1`。
  **SubKey 0 是含出售/挂机/任务的汇总值，绝不可使用，也不可对 SubKey 求和。**
- **关卡** — 取存活怪物 `Monster.STAGE_KEY` 的众数（存档中的当前关卡在切关时会滞后）。
- **队伍** — `StageManager → HERO_LIST`，按编队槽位顺序读取。

**当前状态**

进程附加、模块基址解析、PE 构建指纹、标定匹配，以及只读内存原语（IL2CPP
字符串 / 数组 / List / 两种 Dictionary 几何）均已实现并**实机验证通过**。
`data/meter-offsets.json` 内置游戏 **1.00.27** 的标定种子；若你的游戏版本不同，
面板会附加成功但**明确报告缺少该版本标定**，需补充对应 `anchor_rva` 与类型索引。

尚未移植的上游能力：ACTk 混淆经验值解码、基于 `LogManager` 日志对象指针的精确开局/结算判定
（当前为怪物存在性 + 关卡变化的近似判定）、以及 `{ok, value}` 信封序列化格式。

---

## 数据与 i18n

- **游戏数据**（`data/engine/*.json`）已捆绑，便于离线使用 — 图鉴、关卡、合成与 197 节点符文树无需网络即可渲染。
- **本地化游戏名称**（物品/怪物/关卡）来自 wiki 目录的 16 语言名称表，按请求在服务端解析（`?lang=`）。
- **界面文案**通过 `frontend/i18n.config.ts` 中的 `@nuxtjs/i18n` 翻译。
