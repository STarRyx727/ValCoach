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
| LLM 接入 | OpenAI Responses / Claude Messages / DeepSeek 与主流 OpenAI 兼容 API |
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
         spike_events, ability_events, shots, compact_replays, player_issues, user_profiles
  ↓
CompactReplay: 每回合预编译路线/战斗/技能/Spike JSON
  ↓
Agent Context Builder: 按问题范围检索相关回合 + 证据
  ↓
LLM（OpenAI / Claude / DeepSeek / Gemini / Grok / GLM / Kimi / Qwen）
  → 带地图/回合/时间/证据的复盘
```

## 功能

具体可用字段由录像区域、版本和解析结果决定。系统会为每场录像记录 capability 状态；缺失的数据保持缺失，不以零值或模型猜测代替。

### 必需能力（R1–R6）

| 要求 | 实现 |
|---|---|
| R1 核心逻辑用 Rust | 容器探针、流式规范化、语义建模、地图解析、数据库、指标与 Agent 编排均位于 Rust workspace |
| R2 用户交互界面 | React Web UI：账户、上传、阵容绑定、战绩排行、回合地图、个人主页、模型设置与教练对话 |
| R3 模型与参数可配置 | Provider、模型 ID、Base URL、最大输出 Token 和费用单价均可在 UI 或环境变量配置 |
| R4 实时进度与打断 | SSE 推送解析阶段与进度；停止按钮调用取消端点并传播 Rust `CancellationToken` |
| R5 上下文历史管理 | SQLite 按账户/对局保存对话，后续请求加载最近历史；UI 可查看和清空 |
| R6 Token 与费用 | 每次保存输入/输出/总 Token；配置单价后计算并展示估算费用 |

### 回放解析

- 对已支持的国际服版本流式导入事件与移动数据；数据量由录像时长和实际对局内容决定，不依赖固定记录数
- 国服 13.05 可导入服务器时间线和 5v5 阵容；当 ReplayData 变换无法验证时安全降级，不生成未经验证的移动或战斗结论
- 容器级探针：区域与版本识别、数据块统计、完整性校验
- 国际服与国服处理共用稳定领域模型和数据库结构，解析能力差异通过 capability 状态明确表达

### 语义建模

- **PlayerResolver**：将回放中的 UUID、PlayerState 与 Character 关系还原为双方玩家及特工阵容
- **RoundBuilder**：识别回合边界、回合胜方和半场攻防切换；面向用户的回合从第 1 回合开始
- **CombatBuilder**：合并连续射击、伤害与击杀事件，完成攻击者、受害者和首杀/首死归因
- **ScoreboardBuilder**：从回放计算 K/D、总伤害、ADR、爆头率及回放估算 ACS；无法从回放取得的非伤害助攻不会计入 ACS
- **SpikeBuilder**：提取安装、拆除、爆炸及 Spike 所在区域
- **AbilityBuilder**：提取可识别的技能事件并规范化为官方英文名；无法可靠映射的技能不会被猜测补全
- **MapAreaResolver**：根据本地地图元数据进行坐标转换与 callout 区域解析
- **Movement**：为移动样本补充所属回合、存活状态、区域、朝向与速度等语义字段

### 智能体

- 多 provider 支持：OpenAI / Claude / DeepSeek / Gemini / xAI Grok / 智谱 GLM / Kimi / Qwen / 自定义 OpenAI 兼容接口
- 时间统一表示为“回合编号 + 回合内时间”，模型上下文不使用难以阅读的原始毫秒值
- Agent 上下文自动注入人类可读时间和区域名，不要求模型解释原始坐标
- 将同一连续射击窗口内的 shot、damage 与 kill 合并为紧凑战斗片段；详细检索最多保留 6 个高相关回合，证据与移动路线均按固定预算抽样
- 确定性紧凑回放：每回合预编译 JSON，缓存在 SQLite
- 个性化问题记忆：LLM 自动提取问题并记录每次出现；历史问题模式只加载长期问题、画像和近期教练记录，不重复发送当前整局数据
- 个人训练画像：段位、主玩位置/特工和训练目标会作为用户偏好注入模型上下文，不会冒充录像证据
- 连接重试：超时/连接失败/5xx 自动重试 3 次
- 请求中止：教练生成期间可随时停止，取消信号会传递到服务端模型请求
- API Key 仅存后端进程内存，不写入数据库

### 前端

- 三 Tab 布局：**阵容** / **回合** / **教练**
- 本场 10 人战绩排行：K/D、回放估算 ACS、ADR、首杀/首死、爆头率
- 个人主页：可选段位、位置、主玩特工、训练目标，以及已绑定对局的多场趋势图
- 本地游戏内容快照：可玩特工头像、官方英文名称、技能图标、段位图标和地图俯视图
- 2D 地图查看器：SVG 画布展示玩家路线、战斗标记、Spike 图标
- Markdown 渲染：标题/粗体/列表/代码块/表格
- 解析阶段实时进度与停止按钮
- 模型、最大输出、Base URL 与费用单价均可在网页配置
- 对话历史按对局保存、自动带入后续提问，也可手动清空；用量同时显示单次输入/输出和账户累计 Token
- 录像删除：侧栏删除按钮，同时清理本地文件
- 显示名映射由本地内容目录统一提供，界面和 Rust 语义层不再分别维护零散名称表

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

首次运行会自动检出固定版本的 C# 解析器、应用仓库内补丁并安装前端依赖；地图元数据、俯视图、特工头像、技能与段位图标已经随仓库提供，运行时不需要访问资源 API，也无需 Python。

解析器下载会自动重试三次。如果本机的 HTTP 代理软件正在 `127.0.0.1:7890` 监听，脚本会在从 GitHub 下载解析器时自动使用它；这个端口不是 ValCoach 网站端口。其他代理地址可先设置 `VALCOACH_GIT_PROXY`，例如：

```powershell
$env:VALCOACH_GIT_PROXY = 'http://127.0.0.1:7890'
.\start.cmd
```

本地端口分工如下：

| 端口 | 用途 | 是否需要在浏览器打开 |
|---|---|---|
| `7890` | 可选的第三方 HTTP 代理，仅用于首次从 GitHub 下载解析器 | 否；未使用代理软件时无需开放 |
| `3000` | ValCoach Rust API，由 Vite 转发 `/api` 请求 | 否 |
| `5173` | ValCoach Web 界面 | 是，访问 `http://127.0.0.1:5173` |

也就是说，日常使用只需要打开 `http://127.0.0.1:5173`；前端会自动把 API 请求转发到 3000 端口。

重复运行 `start.cmd` 时，脚本会自动识别并结束当前项目残留的 `valcoach-server` 后端，再启动新实例。如果端口被其他程序占用，脚本不会终止该程序，而是保留现场并提示关闭对应程序。

### 使用流程

1. 打开浏览器访问 `http://127.0.0.1:5173`
2. 注册本地账户 → 登录
3. 上传 `.vrf` 录像文件（≤100 MiB）
4. 等待解析完成（SSE 实时进度）
5. 在阵容页选择你扮演的玩家
6. 切换到回合页查看 2D 地图回放
7. 切换到教练页，配置模型后提问
8. 打开个人主页登记训练画像，并在积累多场已绑定对局后查看趋势

### Agent 配置

在 Web UI 的「模型设置」中配置：
- 服务商：OpenAI / Claude / DeepSeek / Gemini / xAI Grok / 智谱 GLM / Kimi / Qwen / OpenAI 兼容
- 模型 ID
- API Key（仅存内存，不回显）
- Base URL（兼容接口必填）
- 最大输出 Tokens
- 可选：每百万 Token 价格（用于成本估算）

网页为各服务商提供常用模型预设，并允许自由输入服务商实际支持的模型 ID。DeepSeek 默认使用 `deepseek-v4-flash`。预设只是便捷填充；服务商变更模型名称后无需修改代码即可使用新 ID。

## 验证

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd web; npm run build
```

## 游戏内容快照

`web/public/game-content/catalog.json` 记录 Riot Public Content Catalog 版本、ETag 与更新时间作为官方资源基线，并保存由 Valorant-API 规范化的 UUID、开发代号、官方英文显示名和本地资源路径。所有图片均随仓库发布，因此正常运行不依赖外网。

需要跟进新版本时，可手动执行：

```powershell
.\scripts\sync_game_content.ps1
```

脚本会刷新特工、技能、地图和段位资源，同时把地图元数据指向新的本地俯视图。该脚本仅用于维护快照，不会在启动时联网执行。

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
scripts/            # 一键启动、固定版本解析器安装与游戏内容快照同步
docs/               # 长期技术文档（Provider、Bundle 协议、评测与架构决策）
```

## 参考开源项目

| 项目 | 用途 | 许可证 |
|------|------|--------|
| [michel-giehl/ValorantReplayParser](https://github.com/michel-giehl/ValorantReplayParser) | C# 生产级 VALORANT 回放解析器 | MIT |
| [yakisoba0728/vrfkit](https://github.com/yakisoba0728/vrfkit) | Rust VRF 容器解析 + 事件/checkpoint 参考 | MIT |
| [Riot Public Content Catalog](https://developer.riotgames.com/docs/valorant#content-catalog) | 官方名称与美术资源版本基线 | Riot Developer Portal |
| [Valorant-API](https://valorant-api.com) | UUID/显示名映射、地图参数与可离线化资源索引 | 公开 API |

## 许可证

MIT
