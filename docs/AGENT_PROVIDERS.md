# Agent providers and token accounting

ValCoach has one evidence-grounded coaching endpoint with three transport adapters:

| `VALCOACH_LLM_PROVIDER` | API transport | Default base URL | Default key variable |
|---|---|---|---|
| `openai` | Responses API | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| `anthropic` or `claude` | Messages API | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |
| `deepseek` | Chat Completions | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` |
| `openai-compatible` | Chat Completions | required override | `VALCOACH_LLM_API_KEY` |

`VALCOACH_LLM_MODEL` is always explicit; ValCoach does not silently substitute a newer model.
`VALCOACH_LLM_BASE_URL` can override the endpoint. Non-loopback endpoints must use HTTPS.
`VALCOACH_LLM_MAX_OUTPUT_TOKENS` defaults to 800.

The implementation follows the [official OpenAI Responses API](https://developers.openai.com/api/reference/cli/resources/responses/methods/create),
the [Claude Messages API](https://docs.anthropic.com/en/api/messages), and the
[DeepSeek Chat Completions API](https://api-docs.deepseek.com/api/create-chat-completion/).
Provider responses supply input/output usage; ValCoach stores input, output and total tokens for
every assistant answer in SQLite. Anthropic cache-read and cache-creation input tokens are added to
the recorded input total. Provider request ids are retained for diagnostics.

Cost is deliberately configuration-driven because provider/model prices change. If both
`VALCOACH_LLM_INPUT_USD_PER_MILLION` and `VALCOACH_LLM_OUTPUT_USD_PER_MILLION` are set, ValCoach
records an estimate in micro-USD; otherwise token totals remain exact and cost stays unpriced.

API keys are read from environment variables, held only in process memory, redacted from `Debug`,
and never stored in conversations, usage rows, logs, Replay Bundles, or Git. The model receives the
stable match metadata, capability map, compact deterministic metrics, selected-player evidence and
limitations—not the `.vrf` file or full raw NDJSON.

Every prompt requires capability checks, separates observations from recommendations, and forbids
inventing missing facts. A user must bind an observed player before personalized movement metrics
are included. The response returns machine-readable EvidenceRef values and limitations alongside
the prose. This is a guardrail, not proof that arbitrary model text is logically correct; the UI
therefore keeps evidence visible.

API routes:

- `GET /api/agent/status`
- `GET /api/agent/usage`
- `POST /api/matches/{id}/coach` with `{ "question": "..." }`
- `GET /api/matches/{id}/coaching`
