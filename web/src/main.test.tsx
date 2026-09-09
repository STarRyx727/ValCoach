import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { CapabilityBadge, CoachPanel, SettingsModal, stripCoachingIssues, summarizeCapabilities, type AgentStatus } from "./main";

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
  max_output_tokens: 4096,
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
      max_output_tokens: 4096,
    });
    expect(onSaved).toHaveBeenCalledWith(configured);
  });
});
