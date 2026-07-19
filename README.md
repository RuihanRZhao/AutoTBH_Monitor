# AutoTBH_Monitor

面向 **TBH: Task Bar Hero** 的**只读**桌面配套工具，基于 **Rust + Tauri + Nuxt 4**。

读取本地游戏存档（从不修改），对照 Steam 市场实时价格，在原生窗口中提供仓库估值、出售建议、市场浏览、图鉴、合成、刷图校准与补丁动态等功能。界面支持多语言（含完整简体中文）。

> **只读且非官方。** 仅读取存档与公开 Steam 市场数据。与 Valve、Tesseract Studio 或 TBH 开发者无关联。

[English](docs/README.en.md) · [架构与功能说明](docs/architecture.md)

---

## 主要功能

- **仓库估值** — 读取存档，按 Steam 市价汇总背包与页签
- **出售台** — 流动性、建议挂单价、手续费与立即出售评分
- **市场浏览器** — 物品搜索、订单簿、价格历史
- **图鉴 / 合成 / 符文树** — 离线可用的游戏数据浏览
- **更新动态** — SteamDB 补丁说明与 Steam 新闻
- **实时面板（内置）** — 实时 DPS / 金币 / 经验与战斗记录器。功能已**完整并入本项目**，
  用原生 Rust 重写自 MIT 许可的 [tbh-meter](https://github.com/mad-labs-org/tbh-meter)，
  **不再需要 Python 或任何外部进程**；严格只读（仅 `ReadProcessMemory`，不写入、不注入）
- **刷图校准** — 由内置实时面板记录的真实战斗数据驱动

部分能力（Coach、升级推荐等）仍在移植中，详见 [架构与功能说明](docs/architecture.md)。

---

## 使用说明

### 前置条件

- **Rust**（stable，Windows 上使用 MSVC 工具链）
- **Node 20+**
- **Tauri v2** 依赖（WebView2；Windows 11 通常已预装）

### 本地运行

```bash
# 1) 构建前端静态资源
cd frontend
npm install
npm run generate         # → frontend/.output/public

# 2) 启动桌面应用（仓库根目录）
cd ../src-tauri
cargo run                # 在 :5260 启动服务并打开窗口

# 或以无界面方式运行后端，浏览器访问 http://localhost:5260
cargo run --bin autotbh-monitor
```

### 构建安装包

```bash
cargo tauri build        # 生成前端、编译 Rust，产出 NSIS 安装包
```

安装包位于 `src-tauri/target/release/bundle/nsis/*.exe`。CI 会在每次推送时构建。

### 环境变量（可选）

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `TSM_CURRENCY` | `1`（USD） | 初始 Steam 货币代码 |
| `TBH_GAME_DIR` | 自动检测 | 游戏目录 `TaskBarHero_Data` 路径 |
| `TBH_ES3_PASSWORD` | 自动提取 | 强制指定存档解密密钥 |
| `NUXT_PUBLIC_API_BASE` | `http://localhost:5260` | 前端 API 基址（开发用） |

---

## 文档

| 文档 | 说明 |
|------|------|
| [架构与功能说明](docs/architecture.md) | 技术架构、模块划分、接口与移植状态 |
| [English README](docs/README.en.md) | English project intro & usage |
| [Architecture (EN)](docs/architecture.en.md) | Architecture & feature status in English |

---

## 许可证

MIT — 见 [LICENSE](LICENSE)。捆绑的游戏数据与本地化字符串源自 TBH: Task Bar Hero 及社区 wiki，仍归其各自所有者所有；本项目为可互操作的只读配套工具。
