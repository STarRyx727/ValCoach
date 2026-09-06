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

### 回放解析
- 全球 13.05 完整支持：138,065 条事件 + 165,047 条移动样本
- 国服 13.05 部分导入：服务器时间线 + 阵容（ReplayData 加密常量不同，移动/战斗不可用）
- 容器级 probe：区域检测、chunk 统计、完整性校验

### 语义建模
- **PlayerResolver**: Subject UUID → PlayerState NetGUID → Character NetGUID → Agent，5v5 阵容
- **RoundBuilder**: roundStarted/MulticastEndRound 回合边界 + switchTeams 攻防切换
- **CombatBuilder**: 射击 burst 合并、伤害事件、击杀归因（server + parser 双源交叉验证）
- **SpikeBuilder**: plant/defuse/explode + TimedBomb 位置 → 区域
- **AbilityBuilder**: 从 actor_spawned 提取技能效果对象（如 `Sova Q (SonarPing)`）
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
- Markdown 渲染：标题/粗体/列表/代码块
- 录像删除：侧栏删除按钮，同时清理本地文件
- 显示名映射：Hunter→Sova, Bonsai→Split, Deadeye→Chamber

## 快速入门

### 环境要求

- Rust 1.97+ (rustup)
- .NET 10 SDK
- Node.js 18+
- Python 3.10+（用于地图数据获取脚本）

### 安装与运行

```powershell
# 1. 安装 C# 解析器（检出 + 补丁 + 构建）
scripts\setup_parser.ps1

# 2. 获取地图元数据（Valorant-API）
python scripts\fetch_maps.py

# 3. 启动后端
cargo run -p valcoach-server

# 4. 启动前端（新终端）
cd web
npm install
npm run dev
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
scripts/            # 地图数据获取、解析器安装、smoke 测试
docs/               # 技术文档（解析报告、schema diff、provider 指南等）
```

## 参考开源项目

| 项目 | 用途 | 许可证 |
|------|------|--------|
| [michel-giehl/ValorantReplayParser](https://github.com/michel-giehl/ValorantReplayParser) | C# 生产级 VALORANT 回放解析器 | MIT |
| [yakisoba0728/vrfkit](https://github.com/yakisoba0728/vrfkit) | Rust VRF 容器解析 + 事件/checkpoint 参考 | MIT |
| [Valorant-API](https://valorant-api.com) | 地图元数据、callout 区域、小地图坐标参数 | 公开 API |

## 许可证

MIT
