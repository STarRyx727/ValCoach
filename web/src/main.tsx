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
  capabilities: Record<string, string>;
};
type AgentStatus = { configured: boolean; provider: string | null; model: string | null };
type AgentUsage = { input_tokens: number; output_tokens: number; total_tokens: number; cost_microusd: number; priced_requests: number };
type AgentMessage = {
  id: string; session_id: string; provider: string; model: string; role: string; content: string;
  evidence: unknown[]; limitations: string[];
  usage: { input_tokens: number; output_tokens: number; total_tokens: number; cost_microusd: number | null };
};
type Player = { id: string; stable_player_id: string | null; display_name: string | null };
type Metric = { id: string; metric_name: string; value: { status: string; data: unknown; limitations: string[] } };
type Match = {
  id: string;
  parser_source: string;
  metadata: { replay_id: string; branch: string | null; map: string | null; duration_ms: number | null };
  capabilities: Record<string, string>;
  summary: { event_count: number; movement_count: number; has_shot_related_events: boolean };
};
type MatchDetail = Match & { players: Player[]; metrics: Metric[] };

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, { credentials: "same-origin", ...init });
  if (!response.ok) {
    const text = await response.text();
    throw new Error(text || `Request failed (${response.status})`);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

function Auth({ onAuthenticated }: { onAuthenticated: (user: User) => void }) {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setError("");
    try {
      const user = await api<User>(`/api/auth/${mode}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username, password })
      });
      onAuthenticated(user);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "登录失败");
    }
  };
  return <main className="auth"><section><h1>ValCoach</h1><p>Personalized Evidence-Grounded VALORANT Replay Coaching Agent</p><form onSubmit={submit}><label>用户名<input value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" /></label><label>密码<input value={password} onChange={(e) => setPassword(e.target.value)} type="password" autoComplete={mode === "login" ? "current-password" : "new-password"} /></label>{error && <p className="error">{error}</p>}<button>{mode === "login" ? "登录" : "注册并登录"}</button></form><button className="link" onClick={() => setMode(mode === "login" ? "register" : "login")}>{mode === "login" ? "没有账户？注册" : "已有账户？登录"}</button></section></main>;
}

function App() {
  const [user, setUser] = useState<User | null>(null);
  const [matches, setMatches] = useState<Match[]>([]);
  const [detail, setDetail] = useState<MatchDetail | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [bundle, setBundle] = useState<ReplayBundle | null>(null);
  const [agentStatus, setAgentStatus] = useState<AgentStatus>({ configured: false, provider: null, model: null });
  const [agentUsage, setAgentUsage] = useState<AgentUsage | null>(null);
  const [error, setError] = useState("");
  const refreshMatches = useCallback(async () => { setMatches(await api<Match[]>("/api/matches")); }, []);
  useEffect(() => { api<User>("/api/auth/me").then(setUser).catch(() => setUser(null)); }, []);
  useEffect(() => { if (user) refreshMatches().catch((reason) => setError(String(reason))); }, [user, refreshMatches]);
  useEffect(() => {
    if (!user) return;
    api<AgentStatus>("/api/agent/status").then(setAgentStatus).catch(() => undefined);
    api<AgentUsage>("/api/agent/usage").then(setAgentUsage).catch(() => undefined);
  }, [user]);
  useEffect(() => {
    if (!job || ["ready", "failed", "cancelled", "unsupported"].includes(job.status)) return;
    const timer = window.setInterval(() => api<Job>(`/api/jobs/${job.id}`).then(setJob).catch((reason) => setError(String(reason))), 800);
    return () => window.clearInterval(timer);
  }, [job]);
  useEffect(() => {
    if (!job || ["ready", "failed", "cancelled", "unsupported"].includes(job.status)) return;
    const stream = new EventSource(`/api/jobs/${job.id}/events`);
    for (const status of ["queued", "probing", "parsing", "normalizing", "persisting", "computing_metrics", "ready", "failed", "cancelled", "unsupported"]) {
      stream.addEventListener(status, () => api<Job>(`/api/jobs/${job.id}`).then(setJob).catch(() => undefined));
    }
    return () => stream.close();
  }, [job?.id, job?.status]);
  useEffect(() => { if (job?.status === "ready") refreshMatches().catch((reason) => setError(String(reason))); }, [job?.status, refreshMatches]);
  useEffect(() => {
    if (!job || !["ready", "unsupported"].includes(job.status)) return;
    api<ReplayBundle>(`/api/jobs/${job.id}/bundle`).then(setBundle).catch(() => setBundle(null));
  }, [job?.id, job?.status]);
  const selectMatch = async (matchId: string) => setDetail(await api<MatchDetail>(`/api/matches/${matchId}`));
  const upload = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault(); setError("");
    const file = new FormData(event.currentTarget).get("replay");
    if (!(file instanceof File) || file.size === 0) { setError("请选择 .vrf 回放文件。"); return; }
    try {
      const created = await api<JobCreated>("/api/replays", { method: "POST", body: new FormData(event.currentTarget) });
      setBundle(null);
      setJob({ id: created.job_id, status: "queued", error_message: null, match_id: null });
    } catch (reason) { setError(reason instanceof Error ? reason.message : "上传失败"); }
  };
  const bind = async (playerId: string) => { if (!detail) return; await api(`/api/matches/${detail.id}/bind-player`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ player_id: playerId }) }); await selectMatch(detail.id); };
  const refreshAgentUsage = () => api<AgentUsage>("/api/agent/usage").then(setAgentUsage);
  const logout = async () => { await api("/api/auth/logout", { method: "POST" }); setUser(null); setMatches([]); setDetail(null); setAgentUsage(null); };
  if (!user) return <Auth onAuthenticated={setUser} />;
  return <main><header><div><h1>ValCoach</h1><p>已登录：{user.username}{agentUsage ? ` · Agent ${agentUsage.total_tokens} tokens` : ""}</p></div><button className="secondary" onClick={logout}>退出</button></header><section className="upload"><h2>导入回放</h2><p>回放仅保存在本机。Global 13.05 可完整解析；China 13.05 可读取容器与时间线，但内容 transform 尚未支持。</p><form onSubmit={upload}><input name="replay" type="file" accept=".vrf" /><button>上传并解析</button></form>{job && <p className={`job ${job.status}`}>任务 {job.id.slice(0, 8)}：<strong>{job.status}</strong>{job.error_message ? ` — ${job.error_message}` : ""}</p>}{bundle && <section className="bundle"><strong>{bundle.replay.region} · {bundle.replay.branch}</strong><span>{bundle.backend.detail}</span><small>{bundle.records.server_events} server events · {bundle.records.normalized_events} parser events · {bundle.records.movement_samples} movement</small></section>}{error && <p className="error">{error}</p>}</section><section className="grid"><aside><h2>最近回放</h2>{matches.length === 0 ? <p>尚未导入回放。</p> : <ul>{matches.map((match) => <li key={match.id}><button className="match" onClick={() => selectMatch(match.id)}>{match.metadata.replay_id}<small>{match.summary.event_count} events · {match.summary.movement_count} movement</small></button></li>)}</ul>}</aside><article><h2>回放详情</h2>{detail ? <MatchPanel detail={detail} onBind={bind} agentStatus={agentStatus} onUsage={refreshAgentUsage} /> : <p>选择一场已完成的回放以查看真实数据。</p>}</article></section></main>;
}

function MatchPanel({ detail, onBind, agentStatus, onUsage }: { detail: MatchDetail; onBind: (playerId: string) => Promise<void>; agentStatus: AgentStatus; onUsage: () => Promise<void> }) {
  return <><p>来源：{detail.parser_source}；时长：{detail.metadata.duration_ms ?? "未知"} ms</p><h3>能力</h3><pre>{JSON.stringify(detail.capabilities, null, 2)}</pre><h3>选择“本场哪个玩家是你”</h3>{detail.players.length === 0 ? <p>此回放未观察到可稳定识别的玩家。</p> : <ul>{detail.players.map((player) => <li key={player.id}><code>{player.stable_player_id ?? player.id}</code><button onClick={() => onBind(player.id)}>绑定为我</button></li>)}</ul>}<h3>确定性指标</h3>{detail.metrics.length === 0 ? <p>暂无可用指标。</p> : detail.metrics.map((metric) => <section className="metric" key={metric.id}><strong>{metric.metric_name} · {metric.value.status}</strong><pre>{JSON.stringify(metric.value, null, 2)}</pre></section>)}<CoachPanel matchId={detail.id} status={agentStatus} onUsage={onUsage} /></>;
}

function CoachPanel({ matchId, status, onUsage }: { matchId: string; status: AgentStatus; onUsage: () => Promise<void> }) {
  const [question, setQuestion] = useState("");
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const refresh = useCallback(() => api<AgentMessage[]>(`/api/matches/${matchId}/coaching`).then(setMessages), [matchId]);
  useEffect(() => { refresh().catch(() => setMessages([])); }, [refresh]);
  const ask = async (event: FormEvent) => {
    event.preventDefault(); if (!question.trim() || pending) return;
    setPending(true); setError("");
    try {
      await api(`/api/matches/${matchId}/coach`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ question }) });
      setQuestion(""); await refresh(); await onUsage();
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Agent 请求失败"); }
    finally { setPending(false); }
  };
  return <section className="coach"><h3>证据教练 Agent</h3>{status.configured ? <><small>{status.provider} · {status.model}</small><form onSubmit={ask}><textarea value={question} onChange={(event) => setQuestion(event.target.value)} maxLength={4000} placeholder="例如：根据可用证据，我的移动有什么可以改进？" /><button disabled={pending}>{pending ? "分析中…" : "询问教练"}</button></form></> : <p>Agent 尚未配置。请在服务端设置 VALCOACH_LLM_PROVIDER、VALCOACH_LLM_MODEL 和对应 API key。</p>}{error && <p className="error">{error}</p>}<div className="messages">{messages.map((message) => <section key={message.id} className={`message ${message.role}`}><strong>{message.role === "user" ? "你" : "ValCoach"}</strong><p>{message.content}</p>{message.role === "assistant" && <><small>{message.usage.input_tokens} in · {message.usage.output_tokens} out · {message.usage.total_tokens} total</small><details><summary>证据与限制</summary><pre>{JSON.stringify({ evidence: message.evidence, limitations: message.limitations }, null, 2)}</pre></details></>}</section>)}</div></section>;
}

createRoot(document.getElementById("root")!).render(<App />);
