# ValCoach

本地运行的 VALORANT `.vrf` 回放复盘工具。ValCoach 先用确定性程序解析回合、阵容、枪战、技能、Spike 和移动轨迹，再把与问题相关的紧凑证据交给可配置的 LLM，生成带回合、时间和位置依据的复盘建议。

当前支持：

- Global 13.05 完整解析
- China 13.05 独立 payload transform 完整解析
- 其他未验证分支明确拒绝，不会套用相近版本或跨区域 transform
- Web UI、解析实时进度与中止、10 人战绩榜、回合地图、个人画像与多场趋势
- OpenAI、Claude、DeepSeek、Gemini、Grok、GLM、Kimi、Qwen 和自定义 OpenAI 兼容接口
- 对话历史、Token 用量及可选费用估算

## 1. 环境要求

目前提供 Windows 一键启动脚本。编译前请安装：

| 工具 | 最低版本 | 检查命令 |
|---|---:|---|
| Git | 当前稳定版 | `git --version` |
| Rust | 1.97 | `cargo --version` |
| .NET SDK | 10 | `dotnet --version` |
| Node.js | 22.12 | `node --version` |
| npm | 随 Node.js 安装 | `npm --version` |

端口用途：

| 地址 | 用途 |
|---|---|
| `http://127.0.0.1:5173` | 浏览器访问的 Web UI |
| `http://127.0.0.1:3000` | Rust API，通常无需直接打开 |
| `http://127.0.0.1:7890` | 可选的本地 Git HTTP 代理，不是 ValCoach 服务端口 |

## 2. 获取并编译

```powershell
git clone https://github.com/STarRyx727/ValCoach.git
cd ValCoach
```

先安装固定版本的回放解析器。脚本会克隆上游 C# Parser、应用仓库内的单一生产补丁并进行编译：

```powershell
.\scripts\setup_parser.ps1 -SkipTests
```

如果 GitHub 连接需要代理，可以显式指定：

```powershell
$env:VALCOACH_GIT_PROXY = 'http://127.0.0.1:7890'
.\scripts\setup_parser.ps1 -SkipTests
```

脚本也会自动使用正在监听的 `127.0.0.1:7890`，网络失败时最多重试三次。

编译 Rust 后端和 Web 前端：

```powershell
cargo build -p valcoach-server --release

Set-Location web
npm ci
npm run build
Set-Location ..
```

生成结果：

- Rust 后端：`target\release\valcoach-server.exe`
- Web 静态构建：`web\dist\`
- C# Parser：`.external\ValorantReplayParser\src\CliReader\bin\Release\net10.0\`

## 3. 配置 Endpoint 和 API Key

### 方法 A：在网页中配置（推荐）

启动 ValCoach 后，打开右上角“模型设置”，依次填写：

1. 服务商
2. 模型 ID
3. API Key
4. Base URL
5. 最大输出 Tokens
6. 可选的输入、输出价格（USD / 1M Tokens）

常用配置示例：

| 服务商 | 模型示例 | Base URL | Key 环境变量 |
|---|---|---|---|
| OpenAI | `gpt-5.6-sol` | 留空使用 `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| Claude | `claude-sonnet-5` | 留空使用 `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |
| DeepSeek | `deepseek-v4-flash` | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` |
| Gemini | `gemini-3.8-flash` | `https://generativelanguage.googleapis.com/v1beta/openai` | `GEMINI_API_KEY` |
| Grok | `grok-4.6` | `https://api.x.ai/v1` | `XAI_API_KEY` |
| GLM | `glm-5.2` | `https://open.bigmodel.cn/api/paas/v4` | `ZHIPU_API_KEY` |
| Kimi | `kimi-k3` | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` |
| Qwen | `qwen3.8-max` | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` |

模型 ID 必须以服务商控制台当前实际提供的名称为准。预设只是便捷填充，也可以直接输入其他模型 ID。

Base URL 应填写 API 根地址，例如 `https://api.deepseek.com` 或 `https://example.com/v1`，不要填写完整的 `/chat/completions`、`/responses` 或 `/messages` 请求地址。ValCoach 会根据服务商自动追加相应路径。

网页填写的 Key：

- 只保存在当前后端进程内存中
- 不写入 SQLite，也不会由网页读取或回显
- 后端退出后自动清除，下次启动需要重新填写

### 方法 B：启动前设置环境变量

以下为 DeepSeek 示例：

```powershell
$env:VALCOACH_LLM_PROVIDER = 'deepseek'
$env:VALCOACH_LLM_MODEL = 'deepseek-v4-flash'
$env:DEEPSEEK_API_KEY = '你的 API Key'
$env:VALCOACH_LLM_BASE_URL = 'https://api.deepseek.com'
$env:VALCOACH_LLM_MAX_OUTPUT_TOKENS = '4096'
.\start.cmd
```

自定义 OpenAI 兼容服务示例：

```powershell
$env:VALCOACH_LLM_PROVIDER = 'openai-compatible'
$env:VALCOACH_LLM_MODEL = '服务商提供的准确模型 ID'
$env:VALCOACH_LLM_API_KEY = '你的 API Key'
$env:VALCOACH_LLM_BASE_URL = 'https://example.com/v1'
$env:VALCOACH_LLM_MAX_OUTPUT_TOKENS = '4096'
.\start.cmd
```

可选费用配置：

```powershell
$env:VALCOACH_LLM_INPUT_USD_PER_MILLION = '0.00'
$env:VALCOACH_LLM_OUTPUT_USD_PER_MILLION = '0.00'
```

`.env.example` 是变量清单示例，当前程序不会自动加载该文件；请使用 PowerShell `$env:`、系统环境变量，或者直接在网页中配置。

## 4. 运行

最简单的方式是在仓库根目录双击 `start.cmd`，或执行：

```powershell
.\start.cmd
```

首次运行会自动完成 Parser 安装、Web 依赖安装和 Rust debug 编译。启动成功后浏览器会打开：

```text
http://127.0.0.1:5173
```

在当前终端按 `Ctrl+C` 会停止前端和本次启动的后端进程。重复运行脚本时，只会清理由当前仓库生成的残留 `valcoach-server`，不会终止占用端口的其他程序。

不希望自动打开浏览器时：

```powershell
.\scripts\start_valcoach.ps1 -SkipBrowser
```

已经安装过 Parser，希望跳过安装检查时：

```powershell
.\scripts\start_valcoach.ps1 -SkipParserSetup
```

也可以手动运行两个进程。

终端 1：

```powershell
.\target\release\valcoach-server.exe
```

终端 2：

```powershell
Set-Location web
npm run dev -- --host 127.0.0.1 --strictPort
```

## 5. 演示用例

仓库不会上传大型或可能包含个人信息的 `.vrf` 文件。准备一份受支持的 Global 13.05 或 China 13.05 录像，然后按以下流程演示。

### 示例：使用 DeepSeek 复盘一场录像

1. 执行 `.\start.cmd`，访问 `http://127.0.0.1:5173`。
2. 注册一个本地测试账户并登录。
3. 打开“模型设置”，选择 `DeepSeek`。
4. 填写模型 `deepseek-v4-flash`、Base URL `https://api.deepseek.com`、自己的 Key，以及最大输出 `4096`。
5. 上传 `.vrf`，观察实时解析进度；需要时可以点击停止。
6. 解析完成后检查状态：受支持录像应显示“完整解析”，并列出阵容、战斗、移动与技能能力。
7. 在“阵容”中点击自己对应的玩家；再次点击可以取消绑定。
8. 查看战绩榜中的 K/D、估算 ACS、ADR、首杀/首死和爆头率。
9. 在“回合”中选择某一回合，查看地图轨迹、交战和 Spike 标记。
10. 在“教练”中依次提问：

```text
这局最值得改的一件事是什么？请给出对应回合和时间证据。
```

```text
分析我的首死：哪些可以通过站位、节奏或队友协同避免？
```

```text
和我的历史问题相比，这场有没有改善？只分析历史问题，不要重新总结整局。
```

预期结果：回答正文使用人类可读的回合、时间和区域名；“查看依据与数据限制”展示证据卡片，不显示原始 JSON；页面底部显示本次输入、输出和累计 Token，用量价格未填写时只统计 Token、不估算费用。

### 开发者完整 fixture 演示

如果本地有测试录像，将文件放到以下位置：

```text
Demos-Global\ec22cf8e-b1f4-48b7-8426-c60a20562b3e.vrf
Demos-China\0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf
```

然后运行：

```powershell
.\scripts\release_check.ps1
```

该脚本会依次验证 Parser transform、国际服真实录像、国服 13.05 真实录像、Web smoke test 和生产构建。测试录像不会进入 Git。

## 6. 常用验证命令

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

Set-Location web
npm test
npm run build
Set-Location ..
```

## 7. 项目结构

```text
apps/server/            Rust/axum API、认证、解析任务与 Agent
crates/db/              SQLite、语义层、紧凑回放与历史记忆
crates/domain/          稳定领域模型和官方名称映射
crates/maps/            地图坐标转换与区域解析
crates/replay_adapter/  Parser 进程封装、NDJSON 和 China 13.05 接入
crates/vrf_probe/       Rust VRF 容器探针与服务器事件
patches/                固定 C# Parser 的 ValCoach 生产补丁
scripts/                安装、启动、资源同步与发布检查
web/                    React/Vite 用户界面及离线游戏内容资源
docs/                   Bundle、Provider 和评测等技术文档
```

核心逻辑使用 Rust；C# Parser 通过受控子进程和流式 NDJSON 边界接入。地图、特工头像、技能图标和官方英文名称随仓库提供，正常运行不依赖内容资源 API，也不需要 Python。

## 许可证与上游项目

ValCoach 使用 MIT License。回放能力基于固定版本的 [ValorantReplayParser](https://github.com/michel-giehl/ValorantReplayParser) 和 [vrfkit](https://github.com/yakisoba0728/vrfkit)；游戏名称及资源基线来自 [Riot Public Content Catalog](https://developer.riotgames.com/docs/valorant#content-catalog) 与 [Valorant-API](https://valorant-api.com)。
