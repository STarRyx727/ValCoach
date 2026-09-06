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
type CompactReplay = {
  match_id: string; map: string | null; duration_ms: number | null; player_agent: string;
  rounds: CompactRound[];
};
type CompactRound = {
  round_no: number; human_round: string; side: string | null; winner: string | null;
  start_ms: number | null; end_ms: number | null;
  route: { from?: string; to?: string; area?: string; start: string; end: string; alive: boolean }[];
  combat: { events: CompactCombatEvent[]; totals: { shots: number; damage: number; kills: number; deaths: number } };
  abilities: { time: string; ability: string | null; area: string | null }[];
  spike: { time: string; kind: string | null; area: string | null }[];
};
type CompactCombatEvent = { time: string; kind: string | null; weapon: string | null; area: string | null; shots?: number; damage?: number; result?: string; hit_regions?: string[] };
type MapMeta = {
  display_name: string; map_url: string;
  x_multiplier: number; y_multiplier: number; x_scalar_to_add: number; y_scalar_to_add: number;
  callouts: { region_name: string; super_region_name: string; location: { x: number; y: number } }[];
};

const JOB_LABELS: Record<string, string> = {
  queued: "排队中", probing: "识别录像", parsing: "解析对局", normalizing: "整理数据",
  persisting: "保存对局", computing_metrics: "生成分析", ready: "分析就绪",
  unsupported: "暂不支持", failed: "解析失败", cancelled: "已取消"
};

const AGENT_NAMES: Record<string, string> = {
  AggroBot: "Gekko", Clay: "Raze", Deadeye: "Chamber", Hunter: "Sova", Pine: "Veto", Sarge: "Brimstone",
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
  const deleteMatch = useCallback(async (matchId: string) => {
    await api(`/api/matches/${matchId}`, { method: "DELETE" });
    setMatches((prev) => prev.filter((match) => match.id !== matchId));
    setDetail((prev) => prev?.id === matchId ? null : prev);
  }, []);

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
  const bind = async (player: Player) => {
    if (!detail) return;
    await api(`/api/matches/${detail.id}/bind-player`, { method: player.is_bound ? "DELETE" : "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ player_id: player.id }) });
    await selectMatch(detail.id);
  };
  const logout = async () => { await api("/api/auth/logout", { method: "POST" }); setUser(null); setMatches([]); setDetail(null); setAgentUsage(null); };
  if (!user) return <Auth onAuthenticated={setUser} />;
  const working = !!job && !["ready", "failed", "cancelled", "unsupported"].includes(job.status);

  return <main className="app-shell"><header className="topbar"><Brand compact /><div className="topbar-actions"><span className={`agent-pill ${agentStatus.configured ? "online" : ""}`}><i />{agentStatus.configured ? `${agentStatus.provider} · ${agentStatus.model}` : "教练未配置"}</span><button className="secondary icon-button" onClick={() => setSettingsOpen(true)}>模型设置</button><span className="user-chip">{user.username}</span><button className="text-button" onClick={logout}>退出</button></div></header>
    <section className="upload-panel"><div><span className="eyebrow">NEW REVIEW</span><h1>导入一场录像</h1><p>选择国际服或国服的 .vrf 文件。解析在本机完成。</p></div><form onSubmit={upload} className="upload-form"><label className="file-picker"><input name="replay" type="file" accept=".vrf" onChange={(event) => setFileName(event.target.files?.[0]?.name ?? "")} /><span className="file-icon">↥</span><span><strong>{fileName || "选择录像文件"}</strong><small>{fileName ? "点击可更换文件" : "最大 100 MiB · .vrf"}</small></span></label><button className="primary" disabled={working}>{working ? "正在处理…" : "开始分析"}</button></form>{job && <JobProgress job={job} bundle={bundle} />}{error && <p className="notice error">{error}</p>}</section>
    <section className="workspace"><aside className="match-list panel"><div className="section-heading"><div><span className="eyebrow">HISTORY</span><h2>最近对局</h2></div><span>{matches.length}</span></div>{matches.length === 0 ? <div className="empty-state"><b>暂无录像</b><p>上传完成后，对局会出现在这里。</p></div> : <ul>{matches.map((match) => <li key={match.id} className="match-item"><button className={`match-card ${detail?.id === match.id ? "active" : ""}`} onClick={() => selectMatch(match.id)}><span className="map-code">{mapName(match.metadata.map).slice(0, 2).toUpperCase()}</span><span><strong>{mapName(match.metadata.map)}</strong><small>{formatDuration(match.metadata.duration_ms)} · {match.metadata.replay_id.slice(0, 8)}</small></span><i>›</i></button><button className="delete-replay" title="删除录像" onClick={(event) => { event.stopPropagation(); if (confirm("删除这场录像及其所有分析数据？")) deleteMatch(match.id).catch((reason) => setError(String(reason))); }}>×</button></li>)}</ul>}</aside><article className="review-panel panel">{detail ? <MatchPanel detail={detail} onBind={bind} agentStatus={agentStatus} onUsage={refreshAgentUsage} onOpenSettings={() => setSettingsOpen(true)} /> : <div className="empty-review"><span className="target-glyph">⌖</span><h2>选择一场对局</h2><p>查看双方阵容，确认你的玩家后开始复盘。</p></div>}</article></section>
    <footer><span>VALCOACH // LOCAL MODE</span><span>{agentUsage ? `${agentUsage.total_tokens.toLocaleString()} TOKENS USED` : "NO AGENT USAGE"}</span></footer>
    {settingsOpen && <SettingsModal status={agentStatus} onClose={() => setSettingsOpen(false)} onSaved={(next) => { setAgentStatus(next); setSettingsOpen(false); }} />}
  </main>;
}

function JobProgress({ job, bundle }: { job: Job; bundle: ReplayBundle | null }) {
  const terminal = ["ready", "failed", "cancelled", "unsupported"].includes(job.status);
  const readyText = bundle?.backend.status === "partial"
    ? "基础回合与阵容已导入；该版本暂不提供移动和枪战细节"
    : "已可选择玩家并开始复盘";
  return <div className={`job-progress ${job.status}`}><div className="job-line"><span className="pulse" /><strong>{JOB_LABELS[job.status] ?? job.status}</strong><span>{terminal ? "" : "请保持页面打开"}</span></div>{!terminal && <div className="progress-track"><i /></div>}{job.status === "ready" && bundle && <p>{mapName(bundle.replay.map_asset_path)} · {formatDuration(bundle.replay.duration_ms)} · {readyText}</p>}{job.status === "unsupported" && <p>已读取录像，但该服务器版本的完整战斗数据暂不能解析。</p>}{job.error_message && job.status === "failed" && <p>{job.error_message}</p>}</div>;
}

function MatchPanel({ detail, onBind, agentStatus, onUsage, onOpenSettings }: { detail: MatchDetail; onBind: (player: Player) => Promise<void>; agentStatus: AgentStatus; onUsage: () => Promise<void>; onOpenSettings: () => void }) {
  const [binding, setBinding] = useState<string | null>(null);
  const [tab, setTab] = useState<"roster" | "rounds" | "coach">("roster");
  const [compact, setCompact] = useState<CompactReplay | null>(null);
  const [maps, setMaps] = useState<MapMeta[]>([]);
  const teamA = detail.players.filter((player) => player.team === "team_a");
  const teamB = detail.players.filter((player) => player.team === "team_b");
  const rosterReady = teamA.length === 5 && teamB.length === 5;
  const boundPlayer = detail.players.find((player) => player.is_bound);
  const choose = async (player: Player) => { setBinding(player.id); try { await onBind(player); } finally { setBinding(null); } };
  useEffect(() => { api<CompactReplay>(`/api/matches/${detail.id}/compact`).then(setCompact).catch(() => setCompact(null)); }, [detail.id]);
  useEffect(() => { api<MapMeta[]>("/api/maps").then(setMaps).catch(() => setMaps([])); }, []);
  const mapMeta = maps.find((m) => m.map_url === mapName(detail.metadata.map));
  return <><div className="review-heading"><div><span className="eyebrow">MATCH REVIEW</span><h1>{mapName(detail.metadata.map)}</h1><p>{formatDuration(detail.metadata.duration_ms)} · {detail.metadata.replay_id.slice(0, 13)}</p></div><span className="ready-badge"><i />数据就绪</span></div>
    <nav className="tab-bar">{(["roster", "rounds", "coach"] as const).map((t) => <button key={t} className={`tab ${tab === t ? "active" : ""}`} onClick={() => setTab(t)}>{t === "roster" ? "阵容" : t === "rounds" ? "回合" : "教练"}</button>)}</nav>
    {tab === "roster" && <section className="roster-section"><div className="section-title"><div><h2>本场哪个玩家是你？</h2><p>按本局使用的特工选择。双方阵容已分开显示。</p></div>{boundPlayer && <span className="selection-note">已选择 {displayAgent(boundPlayer.agent_name)}</span>}</div>{!rosterReady ? <div className="notice warning"><strong>需要重新导入这场录像</strong><span>这场对局由旧版解析器保存，尚未生成 5v5 阵容。重新上传原录像即可修复。</span></div> : <div className="teams"><TeamRoster title="A 队" tone="red" players={teamA} binding={binding} onChoose={choose} /><div className="versus">VS</div><TeamRoster title="B 队" tone="blue" players={teamB} binding={binding} onChoose={choose} /></div>}</section>}
    {tab === "rounds" && (compact && mapMeta ? <MapViewer compact={compact} mapMeta={mapMeta} /> : <div className="empty-state"><p>正在加载紧凑回放数据…</p></div>)}
    {tab === "coach" && <CoachPanel matchId={detail.id} status={agentStatus} playerSelected={!!boundPlayer} onUsage={onUsage} onOpenSettings={onOpenSettings} />}
  </>;
}

function TeamRoster({ title, tone, players, binding, onChoose }: { title: string; tone: "red" | "blue"; players: Player[]; binding: string | null; onChoose: (player: Player) => Promise<void> }) {
  return <section className={`team team-${tone}`}><header><span>{title}</span><small>5 PLAYERS</small></header><div>{players.map((player, index) => { const agent = displayAgent(player.agent_name); return <button key={player.id} className={`player-card ${player.is_bound ? "selected" : ""}`} onClick={() => onChoose(player)} disabled={binding !== null}><span className="agent-avatar">{agent.slice(0, 1)}</span><span><strong>{agent}</strong><small>玩家 {index + 1}</small></span><b>{binding === player.id ? "保存中" : player.is_bound ? "再次点击取消" : "这是我"}</b></button>; })}</div></section>;
}

function CoachPanel({ matchId, status, playerSelected, onUsage, onOpenSettings }: { matchId: string; status: AgentStatus; playerSelected: boolean; onUsage: () => Promise<void>; onOpenSettings: () => void }) {
  const [question, setQuestion] = useState("");
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const refresh = useCallback(() => api<AgentMessage[]>(`/api/matches/${matchId}/coaching`).then(setMessages), [matchId]);
  useEffect(() => { refresh().catch(() => setMessages([])); }, [refresh]);
  const ask = async (event: FormEvent) => {
    event.preventDefault();
    if (!question.trim() || pending || !playerSelected) return;
    setPending(true);
    setError("");
    const sentQuestion = question;
    try {
      await api(`/api/matches/${matchId}/coach`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ question: sentQuestion }) });
      setQuestion("");
      refresh().catch(() => undefined);
      onUsage().catch(() => undefined);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "教练请求失败，可以修改问题后重试。");
    } finally {
      setPending(false);
    }
  };
  return <section className="coach-section"><div className="section-title"><div><span className="eyebrow">AI COACH</span><h2>开始复盘</h2></div>{status.configured && <span className="model-label">{status.provider} / {status.model}</span>}</div>{!status.configured ? <div className="coach-gate"><span>◇</span><div><strong>先连接一个模型</strong><p>支持 OpenAI、Claude、DeepSeek 和兼容接口。</p></div><button className="secondary" onClick={onOpenSettings}>打开模型设置</button></div> : !playerSelected ? <div className="coach-gate"><span>◎</span><div><strong>先确认你的玩家</strong><p>选择上方阵容中的"这是我"，教练才会使用对应证据。</p></div></div> : <form onSubmit={ask} className="coach-form"><textarea value={question} onChange={(event) => setQuestion(event.target.value)} maxLength={4000} placeholder="问问这场对局里最值得改进的一件事…" />{error && <p className="notice error"><strong>{error}</strong><span>可以修改问题后重新发送。</span></p>}<button className="primary" disabled={pending || !question.trim()}>{pending ? "正在复盘…" : "发送给教练"}</button></form>}<div className="messages">{messages.map((message) => <section key={message.id} className={`message ${message.role}`}><header><strong>{message.role === "user" ? "你" : "VALCOACH"}</strong>{message.role === "assistant" && <small>{message.usage.total_tokens} tokens</small>}</header>{message.role === "assistant" ? <div className="markdown-body" dangerouslySetInnerHTML={{ __html: renderMarkdown(message.content) }} /> : <p>{message.content}</p>}{message.role === "assistant" && (message.evidence.length > 0 || message.limitations.length > 0) && <details><summary>查看依据与数据限制</summary><pre>{JSON.stringify({ evidence: message.evidence, limitations: message.limitations }, null, 2)}</pre></details>}</section>)}</div></section>;
}

function MapViewer({ compact, mapMeta }: { compact: CompactReplay; mapMeta: MapMeta }) {
  const [selectedRound, setSelectedRound] = useState(0);
  const round = compact.rounds[selectedRound];
  if (!round) return <div className="empty-state"><p>没有回合数据。</p></div>;
  const callouts = mapMeta.callouts.filter((c) => c.region_name);
  return <section className="map-viewer">
    <div className="map-canvas-wrap">
      <div className="map-round-selector">
        {compact.rounds.map((r, i) => <button key={i} className={`round-chip ${i === selectedRound ? "active" : ""}`} onClick={() => setSelectedRound(i)}>{r.human_round}<small>{r.side ?? ""}</small></button>)}
      </div>
      <div className="map-canvas">
        <svg viewBox="0 0 1024 1024" className="map-svg">
          {callouts.map((c, i) => {
            const x = c.location.x; const y = c.location.y;
            return <g key={i}><circle cx={x} cy={y} r={3} fill="#3a4858" /><text x={x + 6} y={y + 3} fill="#5a6a7a" fontSize={10}>{c.region_name}</text></g>;
          })}
          {round.route.map((seg, i) => {
            const fromArea = seg.from ? callouts.find((c) => c.region_name === seg.from) : null;
            const toArea = seg.to ? callouts.find((c) => c.region_name === seg.to) : null;
            const areaPt = seg.area ? callouts.find((c) => c.region_name === seg.area) : null;
            const startX = fromArea?.location.x ?? areaPt?.location.x ?? 512;
            const startY = fromArea?.location.y ?? areaPt?.location.y ?? 512;
            const endX = toArea?.location.x ?? areaPt?.location.x ?? startX;
            const endY = toArea?.location.y ?? areaPt?.location.y ?? startY;
            return <g key={i}>
              <line x1={startX} y1={startY} x2={endX} y2={endY} stroke={seg.alive ? "#53bce8" : "#ff4655"} strokeWidth={2} opacity={0.6} />
              <circle cx={startX} cy={startY} r={4} fill="#53bce8" />
              {seg.to && <circle cx={endX} cy={endY} r={4} fill="#56d6a0" />}
            </g>;
          })}
          {round.combat.events.map((e, i) => {
            const area = e.area ? callouts.find((c) => c.region_name === e.area) : null;
            if (!area) return null;
            const isKill = e.result === "kill";
            return <g key={i}><circle cx={area.location.x} cy={area.location.y} r={isKill ? 7 : 5} fill={isKill ? "#ff4655" : "#f2c66d"} stroke="#fff" strokeWidth={1} /><text x={area.location.x + 9} y={area.location.y + 3} fill={isKill ? "#ff4655" : "#f2c66d"} fontSize={9} fontWeight="bold">{e.weapon ?? e.kind}{e.shots ? ` ×${e.shots}` : ""}</text></g>;
          })}
          {round.spike.map((s, i) => {
            const area = s.area ? callouts.find((c) => c.region_name === s.area) : null;
            if (!area) return null;
            return <g key={i}><rect x={area.location.x - 5} y={area.location.y - 5} width={10} height={10} fill="#56d6a0" stroke="#fff" strokeWidth={1} /><text x={area.location.x + 8} y={area.location.y + 3} fill="#56d6a0" fontSize={9}>{s.kind}</text></g>;
          })}
        </svg>
      </div>
    </div>
    <div className="map-round-detail">
      <div className="round-header"><h3>{round.human_round} · {round.side ?? "未知"}</h3><span>{round.winner ? `胜方: ${round.winner}` : ""}</span></div>
      <div className="round-stats">
        <span>射击: {round.combat.totals.shots}</span><span>伤害: {round.combat.totals.damage}</span>
        <span>击杀: {round.combat.totals.kills}</span><span>死亡: {round.combat.totals.deaths}</span>
      </div>
      <div className="round-route">
        <h4>路线</h4>
        {round.route.map((seg, i) => <div key={i} className="route-seg"><span className="route-time">{seg.start}</span><span>{seg.from ? `${seg.from} → ${seg.to}` : seg.area}</span><span className={`route-alive ${seg.alive ? "alive" : "dead"}`}>{seg.alive ? "存活" : "阵亡"}</span></div>)}
      </div>
      {round.combat.events.length > 0 && <div className="round-combat-list"><h4>战斗</h4>{round.combat.events.map((e, i) => <div key={i} className="combat-item"><span className="combat-time">{e.time}</span><span className="combat-kind">{e.kind}</span><span>{e.weapon}</span>{e.shots ? <span>×{e.shots}</span> : null}{e.damage ? <span>{e.damage} dmg</span> : null}{e.result === "kill" ? <strong className="kill-tag">击杀</strong> : null}<span className="combat-area">{e.area}</span></div>)}</div>}
      {round.abilities.length > 0 && <div className="round-abilities"><h4>技能</h4>{round.abilities.map((a, i) => <div key={i} className="ability-item"><span>{a.time}</span><span>{a.ability}</span><span>{a.area}</span></div>)}</div>}
      {round.spike.length > 0 && <div className="round-spike"><h4>Spike</h4>{round.spike.map((s, i) => <div key={i} className="spike-item"><span>{s.time}</span><span>{s.kind}</span><span>{s.area}</span></div>)}</div>}
    </div>
  </section>;
}

function SettingsModal({ status, onClose, onSaved }: { status: AgentStatus; onClose: () => void; onSaved: (status: AgentStatus) => void }) {
  const [provider, setProvider] = useState(status.provider ?? "openai"); const [model, setModel] = useState(status.model ?? ""); const [apiKey, setApiKey] = useState(""); const [baseUrl, setBaseUrl] = useState(""); const [maxTokens, setMaxTokens] = useState("4096"); const [inputPrice, setInputPrice] = useState(""); const [outputPrice, setOutputPrice] = useState(""); const [pending, setPending] = useState(false); const [error, setError] = useState("");
  useEffect(() => { const close = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); }; window.addEventListener("keydown", close); return () => window.removeEventListener("keydown", close); }, [onClose]);
  const submit = async (event: FormEvent) => { event.preventDefault(); setPending(true); setError(""); try { const next = await api<AgentStatus>("/api/agent/settings", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ provider, model, api_key: apiKey, base_url: baseUrl || null, max_output_tokens: Number(maxTokens), input_usd_per_million: inputPrice === "" ? null : Number(inputPrice), output_usd_per_million: outputPrice === "" ? null : Number(outputPrice) }) }); onSaved(next); } catch (reason) { setError(reason instanceof Error ? reason.message : "保存失败"); } finally { setPending(false); } };
  const clear = async () => { setPending(true); setError(""); try { onSaved(await api<AgentStatus>("/api/agent/settings", { method: "DELETE" })); } catch (reason) { setError(reason instanceof Error ? reason.message : "清除失败"); setPending(false); } };
  const changeProvider = (next: string) => {
    setProvider(next);
    if (!model || model.toLowerCase() === provider || model === "deepseek-chat") {
      setModel(next === "deepseek" ? "deepseek-chat" : next === "openai" ? "gpt-4.1-mini" : next === "anthropic" ? "claude-sonnet-4-5" : "");
    }
    if (!baseUrl || provider === "deepseek") setBaseUrl(next === "deepseek" ? "https://api.deepseek.com" : "");
  };
  return <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}><section className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title"><header><div><span className="eyebrow">AI CONNECTION</span><h2 id="settings-title">模型设置</h2></div><button className="close-button" onClick={onClose} aria-label="关闭">×</button></header><p className="security-note"><span>⌁</span><span><strong>Key 不会写入数据库</strong><small>仅保存在本次后端进程内，页面不会读取或回显。</small></span></p><form onSubmit={submit} className="settings-form"><div className="form-row"><label>服务商<select value={provider} onChange={(event) => changeProvider(event.target.value)}><option value="openai">OpenAI</option><option value="anthropic">Claude / Anthropic</option><option value="deepseek">DeepSeek</option><option value="openai-compatible">OpenAI 兼容接口</option></select></label><label>模型 ID<input value={model} onChange={(event) => setModel(event.target.value)} placeholder={provider === "deepseek" ? "deepseek-chat" : "输入准确的模型 ID"} required maxLength={200} /><em>{provider === "deepseek" ? "DeepSeek 服务商默认使用 deepseek-chat" : "这里填写模型标识，不是服务商名称"}</em></label></div><label>API Key<input value={apiKey} onChange={(event) => setApiKey(event.target.value)} type="password" placeholder="输入新的 API Key" required autoComplete="off" /></label><label>Base URL <em>兼容接口必填，其他服务可留空</em><input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} type="url" placeholder="https://example.com/v1" /></label><div className="form-row"><label>最大输出 Tokens<input value={maxTokens} onChange={(event) => setMaxTokens(event.target.value)} type="number" min="1" max="32768" required /></label><span /></div><details className="price-settings"><summary>可选：费用估算</summary><div className="form-row"><label>输入价格 <em>USD / 1M</em><input value={inputPrice} onChange={(event) => setInputPrice(event.target.value)} type="number" min="0" step="any" /></label><label>输出价格 <em>USD / 1M</em><input value={outputPrice} onChange={(event) => setOutputPrice(event.target.value)} type="number" min="0" step="any" /></label></div></details>{error && <p className="notice error">{error}</p>}<div className="modal-actions">{status.api_key_in_memory && <button type="button" className="danger-text" onClick={clear} disabled={pending}>清除本次 Key</button>}<span /><button type="button" className="secondary" onClick={onClose}>取消</button><button type="submit" className="primary" disabled={pending}>{pending ? "保存中…" : "保存连接"}</button></div></form></section></div>;
}

function mapName(path: string | null | undefined) { return path?.split("/").filter(Boolean).at(-1) ?? "未知地图"; }
function formatDuration(milliseconds: number | null | undefined) { if (!milliseconds) return "时长未知"; const minutes = Math.floor(milliseconds / 60_000); const seconds = Math.floor((milliseconds % 60_000) / 1_000); return `${minutes}:${seconds.toString().padStart(2, "0")}`; }
function displayAgent(codename: string | null) { if (!codename) return "未知特工"; return AGENT_NAMES[codename] ?? codename; }

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

function renderMarkdown(markdown: string): string {
  const codeBlocks: string[] = [];
  let text = markdown.replace(/```(\w*)\n?([\s\S]*?)```/g, (_match, _lang, code) => {
    const placeholder = `\x00CODEBLOCK${codeBlocks.length}\x00`;
    codeBlocks.push(`<pre><code>${escapeHtml(code.replace(/\n$/, ""))}</code></pre>`);
    return placeholder;
  });
  const inlineCodes: string[] = [];
  text = text.replace(/`([^`]+)`/g, (_match, code) => {
    const placeholder = `\x00INLINECODE${inlineCodes.length}\x00`;
    inlineCodes.push(`<code>${escapeHtml(code)}</code>`);
    return placeholder;
  });
  text = escapeHtml(text);
  const lines = text.split("\n");
  const result: string[] = [];
  let inList = false;
  let inOrdered = false;
  let paragraph: string[] = [];
  const flushParagraph = () => {
    if (paragraph.length > 0) {
      const content = paragraph.join(" ").trim();
      if (content) result.push(`<p>${content}</p>`);
      paragraph = [];
    }
  };
  const flushList = () => {
    if (inList) { result.push(inOrdered ? "</ol>" : "</ul>"); inList = false; inOrdered = false; }
  };
  for (const line of lines) {
    const trimmed = line.trim();
    const headerMatch = trimmed.match(/^(#{1,6})\s+(.*)/);
    const unorderedMatch = trimmed.match(/^[-*]\s+(.*)/);
    const orderedMatch = trimmed.match(/^\d+[.)]\s+(.*)/);
    if (headerMatch) { flushParagraph(); flushList(); const level = headerMatch[1].length; result.push(`<h${level}>${headerMatch[2]}</h${level}>`); }
    else if (unorderedMatch) { flushParagraph(); if (!inList || inOrdered) { flushList(); result.push("<ul>"); inList = true; inOrdered = false; } result.push(`<li>${unorderedMatch[1]}</li>`); }
    else if (orderedMatch) {
      flushParagraph();
      if (!inList || !inOrdered) { flushList(); result.push("<ol>"); inList = true; inOrdered = true; }
      result.push(`<li>${orderedMatch[1]}</li>`);
    }
    else if (trimmed === "") { flushParagraph(); }
    else { flushList(); paragraph.push(trimmed); }
  }
  flushParagraph();
  flushList();
  let html = result.join("\n");
  html = html.replace(/\*\*\*(.+?)\*\*\*/g, "<strong><em>$1</em></strong>");
  html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/\*(.+?)\*/g, "<em>$1</em>");
  inlineCodes.forEach((code, i) => { html = html.replace(`\x00INLINECODE${i}\x00`, code); });
  codeBlocks.forEach((block, i) => { html = html.replace(`\x00CODEBLOCK${i}\x00`, block); });
  return html;
}

createRoot(document.getElementById("root")!).render(<App />);
