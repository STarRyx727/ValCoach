import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { App, CapabilityBadge, CoachPanel, SettingsModal, evidenceTypeLabel, stripCoachingIssues, summarizeCapabilities, type AgentStatus } from "./main";

const unconfigured: AgentStatus = {
  configured: false,
  provider: null,
  model: null,
  source: null,
  api_key_in_memory: false,
  max_output_tokens: null,
};

const configured: AgentStatus = {
  configured: true,
  provider: "openai",
  model: "gpt-5.6-sol",
  source: "web",
  api_key_in_memory: true,
  max_output_tokens: 32768,
};

function mockJson(value: unknown) {
  const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => new Response(JSON.stringify(value), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  }));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("critical UI flows", () => {
  it("prompts the user to configure a model", () => {
    mockJson([]);
    render(<CoachPanel matchId="match-1" status={unconfigured} playerSelected onUsage={async () => undefined} onOpenSettings={() => undefined} />);
    expect(screen.getByText("先连接一个模型")).toBeTruthy();
    expect(screen.getByRole("button", { name: "打开模型设置" })).toBeTruthy();
  });

  it("prevents coaching before a player is bound", () => {
    mockJson([]);
    render(<CoachPanel matchId="match-1" status={configured} playerSelected={false} onUsage={async () => undefined} onOpenSettings={() => undefined} />);
    expect(screen.getByText("先确认你的玩家")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "发送给教练" })).toBeNull();
  });

  it("renders partial replay capability as a yellow basic state", () => {
    render(<CapabilityBadge capabilities={{ player_identity: "supported", rounds: "partial", movement: "unsupported", combat: "unsupported" }} />);
    const badge = screen.getByText("基础解析");
    expect(badge.classList.contains("basic")).toBe(true);
    expect(summarizeCapabilities({ player_identity: "complete", rounds: "complete", movement: "complete", combat: "complete", abilities: "complete" }).label).toBe("完整解析");
    expect(stripCoachingIssues('可读建议。\n<coaching_issues> [{"issue_key":"hidden"}]')).toBe("可读建议。");
    expect(evidenceTypeLabel("nearby_movement")).toBe("邻近移动轨迹");
    expect(evidenceTypeLabel("characterUltimateUsed")).toBe("终极技能事件");
  });

  it("submits model settings", async () => {
    const fetchMock = mockJson(configured);
    const onSaved = vi.fn();
    render(<SettingsModal status={unconfigured} onClose={() => undefined} onSaved={onSaved} />);
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "test-key" } });
    fireEvent.click(screen.getByRole("button", { name: "保存连接" }));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, request] = fetchMock.mock.calls[0];
    expect(request?.method).toBe("POST");
    expect(JSON.parse(String(request?.body))).toMatchObject({
      provider: "openai",
      model: "gpt-5.6-sol",
      api_key: "test-key",
      max_output_tokens: 32768,
    });
    expect(onSaved).toHaveBeenCalledWith(configured);
  });

  it("does not display a stale match after switching to another replay", async () => {
    const match = (id: string, map: string) => ({
      id, parser_source: "fixture", note: "", played_at: "2026-09-13T00:00:00Z",
      metadata: { replay_id: id, branch: null, map: `/Game/Maps/${map}/${map}`, duration_ms: 90_000 },
      capabilities: {}, summary: { event_count: 0, movement_count: 0, has_shot_related_events: false },
    });
    const first = match("match-a", "CaseA");
    const second = match("match-b", "CaseB");
    let finishFirst!: (response: Response) => void;
    const jsonResponse = (value: unknown) => new Response(JSON.stringify(value), {
      status: 200, headers: { "Content-Type": "application/json" },
    });
    vi.stubGlobal("fetch", vi.fn((input: RequestInfo | URL) => {
      const path = String(input);
      if (path === "/api/matches/match-a") return new Promise<Response>((resolve) => { finishFirst = resolve; });
      const value = path === "/api/auth/me" ? { id: "user-1", username: "demo" }
        : path === "/api/matches" ? [first, second]
        : path === "/api/matches/match-b" ? { ...second, players: [], metrics: [], scoreboard: [] }
        : path === "/api/agent/status" ? unconfigured
        : path === "/api/agent/usage" ? { input_tokens: 0, output_tokens: 0, total_tokens: 0, cost_microusd: 0, priced_requests: 0 }
        : path === "/game-content/catalog.json" ? { schema_version: 1, agents: [], maps: [], competitive_tiers: [] }
        : path === "/api/maps" ? []
        : { match_id: "match-b", player_id: null, map: null, duration_ms: null, player_agent: "", rounds: [] };
      return Promise.resolve(jsonResponse(value));
    }));

    render(<App />);
    await waitFor(() => expect(screen.getByText("CaseA")).toBeTruthy());
    fireEvent.click(screen.getByText("CaseA").closest("button")!);
    fireEvent.click(screen.getByText("CaseB").closest("button")!);
    await waitFor(() => expect(screen.getByRole("heading", { name: "CaseB" })).toBeTruthy());
    await act(async () => finishFirst(jsonResponse({ ...first, players: [], metrics: [], scoreboard: [] })));
    expect(screen.getByRole("heading", { name: "CaseB" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "CaseA" })).toBeNull();
  });
});
