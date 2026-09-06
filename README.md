# ValCoach

**个性化、证据落地的 VALORANT 回放复盘 Agent**

ValCoach 将回放文件保留在本地，通过通用 Rust 容器层解析每个 `.vrf` 文件，
将支持的全球 13.05 数据通过 `ValorantReplayParser` 转换，在 Rust 中规范化为版本化的回放数据包，
在 SQLite 中构建证据关联的语义 IR，并向复盘 Agent 和本地 Web UI 暴露按问题范围检索的回放事实。

## 当前功能

- Argon2id 注册/登录/登出，基于会话的所有权检查。
- 基于文件头的全球/国服方言检测、通用服务器时间线、解析任务生命周期、SSE
  状态流、取消，以及 SQLite 事务/批量导入。
- 全球 13.05 完整文件支持（138,065 条解析事件和 165,047 条移动样本）；
  文件数据库使用 WAL，导入期间进度查询保持响应。
- 国服 13.05 可信部分导入：239/239 服务器事件、22 回合和 10 人阵容头信息
  达到 `ready`；ReplayData 移动/战斗因国服加密常量不同而明确不可用。
- 语义回合、移动（位置/朝向/存活/区域）、射击、伤害、击杀、大招和 Spike
  事件，带源文件/行号证据。Agent 只检索相关回合，包括区域占位和附近玩家快照，
  而非接收有损移动摘要或原始数据转储。
- 对局浏览、从稳定玩家状态身份派生的精确 5v5 阵容、基于特工的玩家选择，
  以及本地账号绑定。重生角色被折叠为同一玩家，不会出现重复 GUID。
- 证据落地的复盘，支持 OpenAI Responses、Claude Messages、DeepSeek
  或其他 OpenAI 兼容 API；对话和输入/输出/总 Token 用量保存在本地。
- 人类可读时间格式（R8 00:26.1），Agent 上下文中所有事件附带 `human_time` 字段。
- 多地图区域解析，通过 Valorant-API callout 数据支持所有竞技地图。
- 射击burst合并：连续射击合并为紧凑burst摘要，大幅减少 Token 消耗。
- 确定性紧凑回放：每回合预编译路线/战斗/技能/Spike JSON，缓存在 SQLite 中。
- 2D 地图查看器：SVG 画布展示玩家路线、战斗标记、Spike 图标，支持回合切换。
- 个性化问题记忆：LLM 自动提取战术问题并持久化，跨对局趋势分析。
- 技能事件：从 actor_spawned 提取技能效果对象（如 Sova Q (SonarPing)）。
- 经济推断：从买枪阶段时间推断经济状态（无个人 Credits 金额）。
- React/Vite 本地 UI，支持浏览器内模型设置、分队阵容选择、复盘历史、
  可折叠证据/限制和 Token 总量。浏览器中输入的 API Key 仅保存在后端进程内存中，
  绝不会返回页面。
- Markdown 渲染：AI 回复支持标题、粗体、列表、代码块等格式。
- 录像删除：可从侧栏删除录像，同时清理本地文件和数据库记录。
- 模型连接重试：超时/连接失败/服务器错误自动重试最多 3 次。

国服 ReplayData 仍不支持，因为其负载转换与全球版本不同；其元数据、阵容、回合
和服务器时间线仍可导入。详见 `docs/REPLAY_CHINA_13_05_REPORT.md`。

## 本地运行

```powershell
scripts\setup_parser.ps1
cargo run -p valcoach-server
cd web
npm install
npm run dev
```

后端监听 `http://127.0.0.1:3000`；Vite 在开发期间代理 `/api` 请求。
安装脚本检出 Parser 提交 `b51d674…`，应用 ValCoach 紧凑输出补丁，
构建并运行其测试。如果检出或 dotnet 可执行文件在其他位置，设置
`VALCOACH_PARSER_DIR` 和 `VALCOACH_DOTNET_PATH`。回放文件、SQLite 数据库、
解析器输出、API Key 和 Node 构建产物被 Git 忽略，不会被本项目上传。

Agent 是可选的。最简单的设置是在 Web UI 中的**模型设置**；
`.env.example` 中的环境变量仍可作为服务器范围的回退。模型选择是显式的，
只有在提供当前每百万 Token 价格时才计算成本估算。详见
`docs/AGENT_PROVIDERS.md`。

首次使用前，运行以下命令获取地图元数据：

```powershell
python scripts\fetch_maps.py
```

## 验证

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd web; npm run build
scripts\smoke_global_13_05.ps1
scripts\smoke_cn_13_05.ps1
cargo test -p valcoach-server 13_05_job -- --ignored --nocapture
```

被忽略的测试需要两个本地 fixture 目录。它们通过 probe → Parser → Bundle →
Semantic IR → SQLite 运行真实全球文件，并验证国服达到能力受限的部分 `ready` 结果。
Fixture 和生成产物的哈希/计数记录在 `docs/` 下。

## 项目结构

```
crates/
├─ domain/          # 稳定的数据契约（ReplayCapabilities, MovementSample, EvidenceRef, humanize）
├─ maps/            # Valorant-API 地图元数据、坐标转换、区域解析
├─ replay_adapter/  # ReplayDataSource trait + C# parser / NDJSON / China 实现
├─ vrf_probe/       # 区域无关的 .vrf 容器探针
├─ metrics/         # 确定性移动指标
├─ db/              # SQLite 持久化 + SemanticBuilder + CompactReplay + PersonalMemory
apps/
└─ server/          # axum HTTP 服务（auth/jobs/matches/agent）
web/                # React/Vite 前端（阵容/回合/教练三Tab + 2D地图查看器）
scripts/            # 地图数据获取、解析器安装、smoke 测试
```

## 许可证

MIT
