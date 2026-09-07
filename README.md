# ValCoach

> 个性化、证据落地的 VALORANT 回放复盘 Agent

## 项目概要

ValCoach 是一个本地运行的 VALORANT 回放分析工具。用户上传 `.vrf` 录像文件后，系统在本地完成解析、语义建模和证据索引，然后通过 LLM Agent 提供基于具体回合/时间/位置的战术复盘建议。

核心理念：**不让大模型读原始回放数据；让程序先把回放编译成大模型真正需要的战术事实。**

## 技术栈

| 层 | 技术 |
|---|---|
| 核心逻辑 | Rust 2024 edition, tokio async runtime |
| HTTP 服务 | axum 0.8, tower-sessions |
| 数据库 | SQLite (sqlx, WAL 模式, 批量事务写入) |
| 前端 | React 18 + TypeScript + Vite |
| 回放解析器 | C# / .NET 10 — `michel-giehl/ValorantReplayParser` (通过 CLI + NDJSON 接入) |
| 容器探针 | Rust — `vrf-container` from `yakisoba0728/vrfkit` |
| LLM 接入 | OpenAI Responses / Claude Messages / DeepSeek / OpenAI 兼容 API |
| 认证 | Argon2id 密码哈希, 本地 session cookie |

## 架构

```
.vrf 上传
  ↓
vrf_probe: 容器解析 → 区域检测 → 服务器时间线 → 阵容
  ↓
C# ValorantReplayParser: events.ndjson + movement.ndjson
  ↓
Rust ParsedBundleSource: 流式 NDJSON → GenericEvent / MovementSample
  ↓
SemanticBuilder: 构建 rounds / combat / abilities / spike / movement enrichment
  ↓
SQLite: events, movement_samples, players, rounds, combat_events,
         spike_events, ability_events, shots, compact_replays, player_issues
  ↓
CompactReplay: 每回合预编译路线/战斗/技能/Spike JSON
  ↓
Agent Context Builder: 按问题范围检索相关回合 + 证据
  ↓
LLM (DeepSeek/OpenAI/Claude) → 带地图/回合/时间/证据的复盘
```

## 功能

### 必需能力（R1–R6）

| 要求 | 实现 |
|---|---|
| R1 核心逻辑用 Rust | 容器探针、流式规范化、语义建模、地图解析、数据库、指标与 Agent 编排均位于 Rust workspace |
| R2 用户交互界面 | React Web UI：账户、上传、阵容绑定、回合地图、模型设置与教练对话 |
| R3 模型与参数可配置 | Provider、模型 ID、Base URL、最大输出 Token 和费用单价均可在 UI 或环境变量配置 |
| R4 实时进度与打断 | SSE 推送解析阶段与进度；停止按钮调用取消端点并传播 Rust `CancellationToken` |
| R5 上下文历史管理 | SQLite 按账户/对局保存对话，后续请求加载最近历史；UI 可查看和清空 |
| R6 Token 与费用 | 每次保存输入/输出/总 Token；配置单价后计算并展示估算费用 |

### 回放解析
- 全球 13.05 完整支持：138,065 条事件 + 165,047 条移动样本
- 国服 13.05 部分导入：服务器时间线 + 阵容（ReplayData 加密常量不同，移动/战斗不可用）
- 容器级 probe：区域检测、chunk 统计、完整性校验

### 语义建模
- **PlayerResolver**: Subject UUID → PlayerState NetGUID → Character NetGUID → Agent，5v5 阵容
- **RoundBuilder**: roundStarted/MulticastEndRound 回合边界 + switchTeams 攻防切换
- **CombatBuilder**: 射击 burst 合并、伤害事件、击杀归因（server + parser 双源交叉验证）
- **SpikeBuilder**: plant/defuse/explode + TimedBomb 位置 → 区域
- **AbilityBuilder**: 从 actor_spawned 提取技能效果，并转换为官方英文技能名（如 `Sova — Recon Bolt`）
- **MapAreaResolver**: Valorant-API callout 区域解析，支持全部竞技地图
- **Movement**: round/alive/area/yaw/pitch/velocity 完整 enrichment

### 智能体
- 多 provider 支持：OpenAI / Claude / DeepSeek / OpenAI 兼容
- 人类可读时间格式（`R8 00:26.1`）
- Agent 上下文自动注入 `human_time` + 区域名（非原始坐标）
- 射击 burst 合并：304 条独立 shot → ~20-30 个紧凑 burst
- 确定性紧凑回放：每回合预编译 JSON，缓存在 SQLite
- 个性化问题记忆：LLM 自动提取 `<coaching_issue>` 块并持久化，跨对局趋势分析
- 连接重试：超时/连接失败/5xx 自动重试 3 次
- API Key 仅存后端进程内存，不写入数据库

### 前端
- 三 Tab 布局：**阵容** / **回合** / **教练**
- 2D 地图查看器：SVG 画布展示玩家路线、战斗标记、Spike 图标
- Markdown 渲染：标题/粗体/列表/代码块/表格
- 解析阶段实时进度与停止按钮
- 模型、最大输出、Base URL 与费用单价均可在网页配置
- 对话历史按对局保存、自动带入后续提问，也可手动清空
- 录像删除：侧栏删除按钮，同时清理本地文件
- 显示名映射：Hunter→Sova, Bonsai→Split, Deadeye→Chamber

## 快速入门

### 环境要求

- Rust 1.97+ (rustup)
- .NET 10 SDK
- Node.js 18+

### 安装与运行

克隆仓库后双击根目录的 `start.cmd`，或在 PowerShell 执行：

```powershell
.\scripts\start_valcoach.ps1
```

首次运行会自动检出固定版本的 C# 解析器、应用仓库内补丁并安装前端依赖；地图元数据和俯视图已经随仓库提供，无需运行额外的 Python 脚本。

解析器下载会自动重试三次，并自动使用正在监听的 `http://127.0.0.1:7890` 本地代理。其他代理地址可先设置 `VALCOACH_GIT_PROXY`，例如：

```powershell
$env:VALCOACH_GIT_PROXY = 'http://127.0.0.1:7890'
.\start.cmd
```

后端监听 `http://127.0.0.1:3000`，Vite 开发服务器代理 `/api` 请求。

### 使用流程

1. 打开浏览器访问 `http://localhost:5173`
2. 注册本地账户 → 登录
3. 上传 `.vrf` 录像文件（≤100 MiB）
4. 等待解析完成（SSE 实时进度）
5. 在阵容页选择你扮演的玩家
6. 切换到回合页查看 2D 地图回放
7. 切换到教练页，配置模型后提问

### Agent 配置

在 Web UI 的「模型设置」中配置：
- 服务商：OpenAI / Claude / DeepSeek / OpenAI 兼容
- 模型 ID
- API Key（仅存内存，不回显）
- Base URL（兼容接口必填）
- 最大输出 Tokens
- 可选：每百万 Token 价格（用于成本估算）

网页提供当前模型预设：OpenAI 的 GPT-6 / GPT-5.6 系列、Claude 5 / 4.6 系列、DeepSeek V4 系列；仍可自由输入兼容服务支持的模型 ID。DeepSeek 默认使用 `deepseek-v4-flash`。

## 验证

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd web; npm run build
```

## 项目结构

```
crates/
├─ domain/          # 稳定数据契约 + humanize + 显示名映射
├─ maps/            # Valorant-API 地图元数据 + 坐标转换 + 区域解析
├─ replay_adapter/  # ReplayDataSource trait + C# parser / NDJSON / China 实现
├─ vrf_probe/       # .vrf 容器探针（region/branch/chunks/server_events）
├─ metrics/         # 确定性移动指标
├─ db/              # SQLite + SemanticBuilder + CompactReplay + PersonalMemory
apps/
└─ server/          # axum HTTP 服务（auth/jobs/matches/agent）
web/                # React/Vite 前端
scripts/            # 一键启动、解析器安装、Bundle 验证与 smoke 测试
docs/               # 长期技术文档（Provider、Bundle 协议、评测与架构决策）
```

## 参考开源项目

| 项目 | 用途 | 许可证 |
|------|------|--------|
| [michel-giehl/ValorantReplayParser](https://github.com/michel-giehl/ValorantReplayParser) | C# 生产级 VALORANT 回放解析器 | MIT |
| [yakisoba0728/vrfkit](https://github.com/yakisoba0728/vrfkit) | Rust VRF 容器解析 + 事件/checkpoint 参考 | MIT |
| [Valorant-API](https://valorant-api.com) | 地图元数据、callout 区域、小地图坐标参数 | 公开 API |

## 许可证

MIT
