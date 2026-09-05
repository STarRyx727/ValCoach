import { FormEvent, useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

type User = { id: string; username: string };
type Job = { id: string; status: string; error_message: string | null; match_id: string | null };
type JobCreated = { job_id: string };
type ReplayBundle = {
  replay: { region: string; branch: string; map_asset_path: string | null; duration_ms: number };
  backend: { status: string; detail: string };
  records: { server_events: number; normalized_events: number; movement_samples: number };
};
type AgentStatus = { configured: boolean; provider: string | null; model: string | null; source: string | null; api_key_in_memory: boolean };
type AgentUsage = { input_tokens: number; output_tokens: number; total_tokens: number; cost_microusd: number; priced_requests: number };
type AgentMessage = {
  id: string; session_id: string; provider: string; model: string; role: string; content: string;
  evidence: unknown[]; limitations: string[];
  usage: { input_tokens: number; output_tokens: number; total_tokens: number; cost_microusd: number | null };
};
type Player = {
  id: string; stable_player_id: string | null; display_name: string | null;
  team: "team_a" | "team_b" | null; agent_name: string | null; player_slot: number | null; is_bound: boolean;
};
type Match = {
  id: string; parser_source: string;
  metadata: { replay_id: string; branch: string | null; map: string | null; duration_ms: number | null };
  capabilities: Record<string, string>;
  summary: { event_count: number; movement_count: number; has_shot_related_events: boolean };
};
type MatchDetail = Match & { players: Player[]; metrics: unknown[] };

const JOB_LABELS: Record<string, string> = {
  queued: "排队中", probing: "识别录像", parsing: "解析对局", normalizing: "整理数据",
  persisting: "保存对局", computing_metrics: "生成分析", ready: "分析就绪",
  unsupported: "暂不支持", failed: "解析失败", cancelled: "已取消"
};

const AGENT_NAMES: Record<string, string> = {
  Clay: "Raze", Deadeye: "Chamber", Hunter: "Sova", Pine: "Veto", Sarge: "Brimstone",
  Smonk: "Clove", Sprinter: "Neon", Vampire: "Reyna", Wushu: "Jett"
};

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, { credentials: "same-origin", ...init });
  if (!response.ok) {
    const text = await response.text();
    let message = text || `请求失败（${response.status}）`;
    try {
      const parsed = JSON.parse(text) as { message?: string; error?: string };
      message = parsed.message ?? parsed.error ?? message;
    } catch { /* Preserve a non-JSON server response. */ }
    throw new Error(message);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

function Brand({ compact = false }: { compact?: boolean }) {
  return <div className={`brand ${compact ? "compact" : ""}`}><span className="brand-mark" aria-hidden="true"><i /><i /></span><span><strong>VALCOACH</strong>{!compact && <small>REPLAY INTELLIGENCE</small>}</span></div>;
}

function Auth({ onAuthenticated }: { onAuthenticated: (user: User) => void }) {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const submit = async (event: FormEvent) => {
    event.preventDefault(); setError("");
    try {
      const user = await api<User>(`/api/auth/${mode}`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ username, password }) });
      onAuthenticated(user);
    } catch (reason) { setError(reason instanceof Error ? reason.message : "登录失败"); }
  };
  return <main className="auth-shell"><section className="auth-panel"><Brand /><div className="auth-copy"><span className="eyebrow">LOCAL REPLAY COACH</span><h1>把每场对局<br />变成下一场的优势</h1><p>录像和分析数据只保存在你的电脑上。</p></div><form onSubmit={submit} className="stack-form"><label>用户名<input value={username} onChange={(event) => setUsername(event.target.value)} autoComplete="username" /></label><label>密码<input value={password} onChange={(event) => setPassword(event.target.value)} type="password" autoComplete={mode === "login" ? "current-password" : "new-password"} /></label>{error && <p className="notice error">{error}</p>}<button className="primary" type="submit">{mode === "login" ? "进入控制台" : "创建账户"}</button></form><button className="text-button" onClick={() => setMode(mode === "login" ? "register" : "login")}>{mode === "login" ? "首次使用？创建本地账户" : "已有账户？返回登录"}</button></section><aside className="auth-art" aria-hidden="true"><div className="scan-ring"><span>V</span></div><b>READ THE ROUND</b></aside></main>;
}

function App() {
  const [user, setUser] = useState<User | null>(null);
  const [matches, setMatches] = useState<Match[]>([]);
  const [detail, setDetail] = useState<MatchDetail | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [bundle, setBundle] = useState<ReplayBundle | null>(null);
  const [agentStatus, setAgentStatus] = useState<AgentStatus>({ configured: false, provider: null, model: null, source: null, api_key_in_memory: false });
  const [agentUsage, setAgentUsage] = useState<AgentUsage | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [fileName, setFileName] = useState("");
  const [error, setError] = useState("");

  const refreshMatches = useCallback(async () => setMatches(await api<Match[]>("/api/matches")), []);
  const refreshAgentStatus = useCallback(async () => setAgentStatus(await api<AgentStatus>("/api/agent/status")), []);
  const refreshAgentUsage = useCallback(async () => setAgentUsage(await api<AgentUsage>("/api/agent/usage")), []);
  const selectMatch = useCallback(async (matchId: string) => setDetail(await api<MatchDetail>(`/api/matches/${matchId}`)), []);

  useEffect(() => { api<User>("/api/auth/me").then(setUser).catch(() => setUser(null)); }, []);
  useEffect(() => { if (user) { refreshMatches().catch((reason) => setError(String(reason))); refreshAgentStatus().catch(() => undefined); refreshAgentUsage().catch(() => undefined); } }, [user, refreshAgentStatus, refreshAgentUsage, refreshMatches]);
  useEffect(() => {
    if (!job || ["ready", "failed", "cancelled", "unsupported"].includes(job.status)) return;
    const timer = window.setInterval(() => api<Job>(`/api/jobs/${job.id}`).then(setJob).catch((reason) => setError(String(reason))), 800);
    return () => window.clearInterval(timer);
  }, [job]);
  useEffect(() => {
    if (!job || ["ready", "failed", "cancelled", "unsupported"].includes(job.status)) return;
    const stream = new EventSource(`/api/jobs/${job.id}/events`);
    Object.keys(JOB_LABELS).forEach((status) => stream.addEventListener(status, () => api<Job>(`/api/jobs/${job.id}`).then(setJob).catch(() => undefined)));
    return () => stream.close();
  }, [job?.id, job?.status]);
  useEffect(() => {
    if (job?.status !== "ready") return;
    refreshMatches().catch((reason) => setError(String(reason)));
    if (job.match_id) selectMatch(job.match_id).catch((reason) => setError(String(reason)));
  }, [job?.status, job?.match_id, refreshMatches, selectMatch]);
  useEffect(() => {
    if (!job || !["ready", "unsupported"].includes(job.status)) return;
    api<ReplayBundle>(`/api/jobs/${job.id}/bundle`).then(setBundle).catch(() => setBundle(null));
  }, [job?.id, job?.status]);

  const upload = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault(); setError("");
    const data = new FormData(event.currentTarget);
    const file = data.get("replay");
    if (!(file instanceof File) || file.size === 0) { setError("请选择一个 .vrf 录像文件。"); return; }
    try {
      const created = await api<JobCreated>("/api/replays", { method: "POST", body: data });
      setBundle(null); setDetail(null); setJob({ id: created.job_id, status: "queued", error_message: null, match_id: null });
    } catch (reason) { setError(reason instanceof Error ? reason.message : "上传失败"); }
  };
  const bind = async (playerId: string) => {
    if (!detail) return;
    await api(`/api/matches/${detail.id}/bind-player`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ player_id: playerId }) });
    await selectMatch(detail.id);
  };
  const logout = async () => { await api("/api/auth/logout", { method: "POST" }); setUser(null); setMatches([]); setDetail(null); setAgentUsage(null); };
  if (!user) return <Auth onAuthenticated={setUser} />;
  const working = !!job && !["ready", "failed", "cancelled", "unsupported"].includes(job.status);

  return <main className="app-shell"><header className="topbar"><Brand compact /><div className="topbar-actions"><span className={`agent-pill ${agentStatus.configured ? "online" : ""}`}><i />{agentStatus.configured ? `${agentStatus.provider} · ${agentStatus.model}` : "教练未配置"}</span><button className="secondary icon-button" onClick={() => setSettingsOpen(true)}>模型设置</button><span className="user-chip">{user.username}</span><button className="text-button" onClick={logout}>退出</button></div></header>
    <section className="upload-panel"><div><span className="eyebrow">NEW REVIEW</span><h1>导入一场录像</h1><p>选择国际服或国服的 .vrf 文件。解析在本机完成。</p></div><form onSubmit={upload} className="upload-form"><label className="file-picker"><input name="replay" type="file" accept=".vrf" onChange={(event) => setFileName(event.target.files?.[0]?.name ?? "")} /><span className="file-icon">↥</span><span><strong>{fileName || "选择录像文件"}</strong><small>{fileName ? "点击可更换文件" : "最大 100 MiB · .vrf"}</small></span></label><button className="primary" disabled={working}>{working ? "正在处理…" : "开始分析"}</button></form>{job && <JobProgress job={job} bundle={bundle} />}{error && <p className="notice error">{error}</p>}</section>
    <section className="workspace"><aside className="match-list panel"><div className="section-heading"><div><span className="eyebrow">HISTORY</span><h2>最近对局</h2></div><span>{matches.length}</span></div>{matches.length === 0 ? <div className="empty-state"><b>暂无录像</b><p>上传完成后，对局会出现在这里。</p></div> : <ul>{matches.map((match) => <li key={match.id}><button className={`match-card ${detail?.id === match.id ? "active" : ""}`} onClick={() => selectMatch(match.id)}><span className="map-code">{mapName(match.metadata.map).slice(0, 2).toUpperCase()}</span><span><strong>{mapName(match.metadata.map)}</strong><small>{formatDuration(match.metadata.duration_ms)} · {match.metadata.replay_id.slice(0, 8)}</small></span><i>›</i></button></li>)}</ul>}</aside><article className="review-panel panel">{detail ? <MatchPanel detail={detail} onBind={bind} agentStatus={agentStatus} onUsage={refreshAgentUsage} onOpenSettings={() => setSettingsOpen(true)} /> : <div className="empty-review"><span className="target-glyph">⌖</span><h2>选择一场对局</h2><p>查看双方阵容，确认你的玩家后开始复盘。</p></div>}</article></section>
    <footer><span>VALCOACH // LOCAL MODE</span><span>{agentUsage ? `${agentUsage.total_tokens.toLocaleString()} TOKENS USED` : "NO AGENT USAGE"}</span></footer>
    {settingsOpen && <SettingsModal status={agentStatus} onClose={() => setSettingsOpen(false)} onSaved={(next) => { setAgentStatus(next); setSettingsOpen(false); }} />}
  </main>;
}

function JobProgress({ job, bundle }: { job: Job; bundle: ReplayBundle | null }) {
  const terminal = ["ready", "failed", "cancelled", "unsupported"].includes(job.status);
  return <div className={`job-progress ${job.status}`}><div className="job-line"><span className="pulse" /><strong>{JOB_LABELS[job.status] ?? job.status}</strong><span>{terminal ? "" : "请保持页面打开"}</span></div>{!terminal && <div className="progress-track"><i /></div>}{job.status === "ready" && bundle && <p>{mapName(bundle.replay.map_asset_path)} · {formatDuration(bundle.replay.duration_ms)} · 已可选择玩家并开始复盘</p>}{job.status === "unsupported" && <p>已读取录像，但该服务器版本的完整战斗数据暂不能解析。</p>}{job.error_message && job.status === "failed" && <p>{job.error_message}</p>}</div>;
}

function MatchPanel({ detail, onBind, agentStatus, onUsage, onOpenSettings }: { detail: MatchDetail; onBind: (playerId: string) => Promise<void>; agentStatus: AgentStatus; onUsage: () => Promise<void>; onOpenSettings: () => void }) {
  const [binding, setBinding] = useState<string | null>(null);
  const teamA = detail.players.filter((player) => player.team === "team_a");
  const teamB = detail.players.filter((player) => player.team === "team_b");
  const rosterReady = teamA.length === 5 && teamB.length === 5;
  const boundPlayer = detail.players.find((player) => player.is_bound);
  const choose = async (playerId: string) => { setBinding(playerId); try { await onBind(playerId); } finally { setBinding(null); } };
  return <><div className="review-heading"><div><span className="eyebrow">MATCH REVIEW</span><h1>{mapName(detail.metadata.map)}</h1><p>{formatDuration(detail.metadata.duration_ms)} · {detail.metadata.replay_id.slice(0, 13)}</p></div><span className="ready-badge"><i />数据就绪</span></div><section className="roster-section"><div className="section-title"><div><h2>本场哪个玩家是你？</h2><p>按本局使用的特工选择。双方阵容已分开显示。</p></div>{boundPlayer && <span className="selection-note">已选择 {displayAgent(boundPlayer.agent_name)}</span>}</div>{!rosterReady ? <div className="notice warning"><strong>需要重新导入这场录像</strong><span>这场对局由旧版解析器保存，尚未生成 5v5 阵容。重新上传原录像即可修复。</span></div> : <div className="teams"><TeamRoster title="A 队" tone="red" players={teamA} binding={binding} onChoose={choose} /><div className="versus">VS</div><TeamRoster title="B 队" tone="blue" players={teamB} binding={binding} onChoose={choose} /></div>}</section><CoachPanel matchId={detail.id} status={agentStatus} playerSelected={!!boundPlayer} onUsage={onUsage} onOpenSettings={onOpenSettings} /></>;
}

function TeamRoster({ title, tone, players, binding, onChoose }: { title: string; tone: "red" | "blue"; players: Player[]; binding: string | null; onChoose: (playerId: string) => Promise<void> }) {
  return <section className={`team team-${tone}`}><header><span>{title}</span><small>5 PLAYERS</small></header><div>{players.map((player, index) => { const agent = displayAgent(player.agent_name); return <button key={player.id} className={`player-card ${player.is_bound ? "selected" : ""}`} onClick={() => onChoose(player.id)} disabled={binding !== null}><span className="agent-avatar">{agent.slice(0, 1)}</span><span><strong>{agent}</strong><small>玩家 {index + 1}</small></span><b>{player.is_bound ? "已选择" : binding === player.id ? "保存中" : "这是我"}</b></button>; })}</div></section>;
}

function CoachPanel({ matchId, status, playerSelected, onUsage, onOpenSettings }: { matchId: string; status: AgentStatus; playerSelected: boolean; onUsage: () => Promise<void>; onOpenSettings: () => void }) {
  const [question, setQuestion] = useState(""); const [messages, setMessages] = useState<AgentMessage[]>([]); const [pending, setPending] = useState(false); const [error, setError] = useState("");
  const refresh = useCallback(() => api<AgentMessage[]>(`/api/matches/${matchId}/coaching`).then(setMessages), [matchId]);
  useEffect(() => { refresh().catch(() => setMessages([])); }, [refresh]);
  const ask = async (event: FormEvent) => { event.preventDefault(); if (!question.trim() || pending || !playerSelected) return; setPending(true); setError(""); try { await api(`/api/matches/${matchId}/coach`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ question }) }); setQuestion(""); await refresh(); await onUsage(); } catch (reason) { setError(reason instanceof Error ? reason.message : "教练请求失败"); } finally { setPending(false); } };
  return <section className="coach-section"><div className="section-title"><div><span className="eyebrow">AI COACH</span><h2>开始复盘</h2></div>{status.configured && <span className="model-label">{status.provider} / {status.model}</span>}</div>{!status.configured ? <div className="coach-gate"><span>◇</span><div><strong>先连接一个模型</strong><p>支持 OpenAI、Claude、DeepSeek 和兼容接口。</p></div><button className="secondary" onClick={onOpenSettings}>打开模型设置</button></div> : !playerSelected ? <div className="coach-gate"><span>◎</span><div><strong>先确认你的玩家</strong><p>选择上方阵容中的“这是我”，教练才会使用对应证据。</p></div></div> : <form onSubmit={ask} className="coach-form"><textarea value={question} onChange={(event) => setQuestion(event.target.value)} maxLength={4000} placeholder="问问这场对局里最值得改进的一件事…" /><button className="primary" disabled={pending || !question.trim()}>{pending ? "正在复盘…" : "发送给教练"}</button></form>}{error && <p className="notice error">{error}</p>}<div className="messages">{messages.map((message) => <section key={message.id} className={`message ${message.role}`}><header><strong>{message.role === "user" ? "你" : "VALCOACH"}</strong>{message.role === "assistant" && <small>{message.usage.total_tokens} tokens</small>}</header><p>{message.content}</p>{message.role === "assistant" && (message.evidence.length > 0 || message.limitations.length > 0) && <details><summary>查看依据与数据限制</summary><pre>{JSON.stringify({ evidence: message.evidence, limitations: message.limitations }, null, 2)}</pre></details>}</section>)}</div></section>;
}

function SettingsModal({ status, onClose, onSaved }: { status: AgentStatus; onClose: () => void; onSaved: (status: AgentStatus) => void }) {
  const [provider, setProvider] = useState(status.provider ?? "openai"); const [model, setModel] = useState(status.model ?? ""); const [apiKey, setApiKey] = useState(""); const [baseUrl, setBaseUrl] = useState(""); const [maxTokens, setMaxTokens] = useState("800"); const [inputPrice, setInputPrice] = useState(""); const [outputPrice, setOutputPrice] = useState(""); const [pending, setPending] = useState(false); const [error, setError] = useState("");
  useEffect(() => { const close = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); }; window.addEventListener("keydown", close); return () => window.removeEventListener("keydown", close); }, [onClose]);
  const submit = async (event: FormEvent) => { event.preventDefault(); setPending(true); setError(""); try { const next = await api<AgentStatus>("/api/agent/settings", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ provider, model, api_key: apiKey, base_url: baseUrl || null, max_output_tokens: Number(maxTokens), input_usd_per_million: inputPrice === "" ? null : Number(inputPrice), output_usd_per_million: outputPrice === "" ? null : Number(outputPrice) }) }); onSaved(next); } catch (reason) { setError(reason instanceof Error ? reason.message : "保存失败"); } finally { setPending(false); } };
  const clear = async () => { setPending(true); setError(""); try { onSaved(await api<AgentStatus>("/api/agent/settings", { method: "DELETE" })); } catch (reason) { setError(reason instanceof Error ? reason.message : "清除失败"); setPending(false); } };
  return <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}><section className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title"><header><div><span className="eyebrow">AI CONNECTION</span><h2 id="settings-title">模型设置</h2></div><button className="close-button" onClick={onClose} aria-label="关闭">×</button></header><p className="security-note"><span>⌁</span><span><strong>Key 不会写入数据库</strong><small>仅保存在本次后端进程内，页面不会读取或回显。</small></span></p><form onSubmit={submit} className="settings-form"><div className="form-row"><label>服务商<select value={provider} onChange={(event) => setProvider(event.target.value)}><option value="openai">OpenAI</option><option value="anthropic">Claude / Anthropic</option><option value="deepseek">DeepSeek</option><option value="openai-compatible">OpenAI 兼容接口</option></select></label><label>模型<input value={model} onChange={(event) => setModel(event.target.value)} placeholder="输入模型 ID" required maxLength={200} /></label></div><label>API Key<input value={apiKey} onChange={(event) => setApiKey(event.target.value)} type="password" placeholder="输入新的 API Key" required autoComplete="off" /></label><label>Base URL <em>兼容接口必填，其他服务可留空</em><input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} type="url" placeholder="https://example.com/v1" /></label><div className="form-row"><label>最大输出 Tokens<input value={maxTokens} onChange={(event) => setMaxTokens(event.target.value)} type="number" min="1" max="32768" required /></label><span /></div><details className="price-settings"><summary>可选：费用估算</summary><div className="form-row"><label>输入价格 <em>USD / 1M</em><input value={inputPrice} onChange={(event) => setInputPrice(event.target.value)} type="number" min="0" step="any" /></label><label>输出价格 <em>USD / 1M</em><input value={outputPrice} onChange={(event) => setOutputPrice(event.target.value)} type="number" min="0" step="any" /></label></div></details>{error && <p className="notice error">{error}</p>}<div className="modal-actions">{status.api_key_in_memory && <button type="button" className="danger-text" onClick={clear} disabled={pending}>清除本次 Key</button>}<span /><button type="button" className="secondary" onClick={onClose}>取消</button><button type="submit" className="primary" disabled={pending}>{pending ? "保存中…" : "保存连接"}</button></div></form></section></div>;
}

function mapName(path: string | null | undefined) { return path?.split("/").filter(Boolean).at(-1) ?? "未知地图"; }
function formatDuration(milliseconds: number | null | undefined) { if (!milliseconds) return "时长未知"; const minutes = Math.floor(milliseconds / 60_000); const seconds = Math.floor((milliseconds % 60_000) / 1_000); return `${minutes}:${seconds.toString().padStart(2, "0")}`; }
function displayAgent(codename: string | null) { if (!codename) return "未知特工"; return AGENT_NAMES[codename] ?? codename; }

createRoot(document.getElementById("root")!).render(<App />);
