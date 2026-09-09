# Agent providers and token accounting

ValCoach has one evidence-grounded coaching endpoint with three transport adapters. Each signed-in
user can configure these values from **模型设置** in the web UI; the table below also documents the
equivalent server environment fallback:

| `VALCOACH_LLM_PROVIDER` | API transport | Default base URL | Default key variable |
|---|---|---|---|
| `openai` | Responses API | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| `anthropic` or `claude` | Messages API | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |
| `deepseek` | Chat Completions | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` |
| `gemini` or `google` | OpenAI-compatible Chat Completions | `https://generativelanguage.googleapis.com/v1beta/openai` | `GEMINI_API_KEY` |
| `xai` or `grok` | OpenAI-compatible Chat Completions | `https://api.x.ai/v1` | `XAI_API_KEY` |
| `zhipu` or `glm` | OpenAI-compatible Chat Completions | `https://open.bigmodel.cn/api/paas/v4` | `ZHIPU_API_KEY` |
| `moonshot` or `kimi` | OpenAI-compatible Chat Completions | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` |
| `qwen` or `dashscope` | OpenAI-compatible Chat Completions | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` |
| `openai-compatible` | Chat Completions | required override | `VALCOACH_LLM_API_KEY` |

`VALCOACH_LLM_MODEL` is explicit and all model IDs are preserved exactly. The web UI supplies
convenient current presets, but a user can enter any model ID accepted by the selected provider.
`VALCOACH_LLM_BASE_URL` can override the endpoint. Non-loopback endpoints must use HTTPS.
`VALCOACH_LLM_MAX_OUTPUT_TOKENS` defaults to 32768. OpenAI's limit includes both visible output and
reasoning tokens, so a very small limit can finish before the model produces an answer.

The implementation follows the [official OpenAI Responses API](https://developers.openai.com/api/reference/cli/resources/responses/methods/create),
the [Claude Messages API](https://docs.anthropic.com/en/api/messages), and the
[DeepSeek Chat Completions API](https://api-docs.deepseek.com/api/create-chat-completion/).
Provider responses supply input/output usage; ValCoach stores input, output and total tokens for
every assistant answer in SQLite. Anthropic cache-read and cache-creation input tokens are added to
the recorded input total. Provider request ids are retained for diagnostics.

Cost is deliberately configuration-driven because provider/model prices change. If both
`VALCOACH_LLM_INPUT_USD_PER_MILLION` and `VALCOACH_LLM_OUTPUT_USD_PER_MILLION` are set, ValCoach
records an estimate in micro-USD; otherwise token totals remain exact and cost stays unpriced.

API keys supplied through the web UI are sent once to the local backend and held only in process
memory; they disappear when the server restarts or when the user clears the setting. Environment
keys are also held only in memory. Keys are never returned to the browser, stored in conversations,
usage rows, databases, logs, Replay Bundles, or Git. The model receives the
stable match metadata, capability map, scoped rounds, area/movement timelines, combat, abilities,
Spike facts, nearby-player snapshots, compact deterministic metrics and limitations—not the `.vrf`
file or full raw NDJSON. Match analysis ranks and includes at most six detailed rounds, compacts
movement into area transitions, samples evidence references, and enforces a 96,000-byte serialized
context ceiling before a provider request is made. Questions that ask only for historical problems
use a separate lightweight path containing the saved issue memory, player profile, and up to eight
recent coaching exchanges; the current replay timeline is not loaded.

Every prompt requires capability checks, separates observations from recommendations, and forbids
inventing missing facts. A user must bind an observed player before personalized movement metrics
are included. The response returns machine-readable EvidenceRef values and limitations alongside
the prose. This is a guardrail, not proof that arbitrary model text is logically correct; the UI
therefore keeps evidence visible.

API routes:

- `GET /api/agent/status`
- `GET /api/agent/usage`
- `POST /api/agent/settings` to set a per-user, process-memory-only provider configuration
- `DELETE /api/agent/settings` to clear it and fall back to environment configuration, if present
- `POST /api/matches/{id}/coach` with `{ "question": "..." }`
- `POST /api/matches/{id}/coach/cancel` to interrupt the active model request
- `GET /api/matches/{id}/coaching`
