# ValCoach 实施计划（V3 Replay Parser 主线）

> 状态：Global 13.05 已通过完整网站链路；China 13.05 完成 container/server timeline，
> ReplayData transform 明确 Unsupported，不阻塞 Global MVP。

## 阶段与验收

1. **P0-CN：历史限制**
   - CN 13.00 不是 Global 13.00 的简单 alias，实验已回滚。详见 `docs/P0_REPORT.md`。
   - 不再修改 CN transform；未来仅作为显式研究分支。

2. **Milestone 1：Global 13.05 Parser Ready — 完成**
   - 未修改 Parser 基线、Common Event、紧凑生产导出和 Bundle validator 全部通过。
   - 网站真实作业写入 138,065 event / 165,047 movement 并进入 `ready`。
   - 完整证据见 `docs/REPLAY_GLOBAL_13_05_REPORT.md`。

3. **Milestone 2：China 13.05 Container Ready — 完成**
   - Header/branch、ReplayInfo、23 ReplayData、22 Checkpoint 和 239/239 server Event 通过。
   - 最小 Global transform alias 严格判定失败并从生产回滚；网站返回 `unsupported`。
   - 完整证据见 `docs/REPLAY_CHINA_13_05_REPORT.md`。

4. **P1：Rust workspace 与稳定 Replay Adapter**
   - Rust workspace、稳定领域模型与基于 CLI/NDJSON 的流式 Adapter 已完成。
   - 已实现 `ReplayDataSource`、`ValorantReplayParserSource`、`ParsedBundleSource` 与明确 Unsupported 的 `ChinaVrfSource`；真实 Global bundle integration test 已通过。
   - SQLite 初始 migration 与 match summary 持久化已完成，并有 migration test。
   - SQLite 使用单事务、每批 500 条写入 events/movement_samples；文件数据库启用 WAL 和
     多连接，长写入不会阻塞 job 状态读取。
   - Replay Bundle v1 统一 Global/China 上层格式，能力可查询，第三方 schema 不直接暴露。

5. **P2：身份与历史数据**
   - 已实现 Argon2id 用户注册、登录、登出和 `/api/auth/me`，并提供 SQLite user repository。
   - 本地 HTTP session cookie 使用 `HttpOnly` 与 `SameSite=Lax`；进程内 API 测试覆盖完整 session 生命周期。
   - Match 列表、详情、指标和玩家绑定 endpoint 均已按 `user_id` 过滤；从可观察的角色标识生成候选玩家，并保存 Global account binding。

6. **Upload / Job Lifecycle**
   - 已实现 `POST /api/replays`、`GET /api/jobs/:id`、`GET /api/jobs/:id/events` 与 `POST /api/jobs/:id/cancel`。
   - 上传文件只保存到受控的本地 `data/replays/<user-id>/` 路径，大小限制为 100 MiB；job 状态和 match summary 均按 `user_id` 查询。
   - Global fixture job 的 Parser → Adapter → SQLite → ready 集成测试已通过。
   - 已完成本地 React/Vite MVP（登录、上传、进度、详情、绑定）。

7. **P3：确定性指标**
   - 已完成 Movement V1：样本量、时间范围、原始坐标路径距离、速度汇总和 EvidenceRef，缺失 velocity 明确标为 `partial`。
   - 下一步：仅在完整 Global 回放能提供可靠 shot/kill/player 数据时启用 gunplay、death、team spacing、tradeability 和 rounds。

8. **P4：LLM Agent 与证据 — 基础阶段完成**
   - 已实现 OpenAI Responses、Claude Messages、DeepSeek 和自定义 OpenAI-compatible provider；
     API key 只从环境变量读取。
   - 每次调用和会话持久化 input/output/total token；成本仅在配置当前价格时计算。
   - Agent 上下文读取 Replay capability、选中玩家的确定性指标和 EvidenceRef；数据缺失时
     明确限制，不把 raw replay 或完整 NDJSON 发给 provider。

9. **P5：Web UI、趋势与交付**
   - 实现 React/Vite 页面、长期 profile、端到端 smoke、评测与演示文档。

## 当前工作项

- Milestone 3：研究并验证 China 13.05 transform；在 full-file validator PASS 前保持 Unsupported。
- 扩展 Agent 的只读工具注册表和多轮工具循环；当前基础教练调用已支持多 provider、保存、
  token/cost 统计和证据约束。
- 在获得完整 Global 对局后进行 multi-player、gunplay 和历史画像验证；不得用当前短 fixture 伪造这些能力。

## 约束

- ValCoach 核心逻辑使用 Rust；第三方 C# Parser 仅通过其 CLI 和 versioned NDJSON 接入。
- 不伪造 replay/指标；不记录 API key；真实 `.vrf`、数据库、运行产物不得提交 Git。
- 每阶段先运行对应测试和 smoke test，再进入下一阶段。
