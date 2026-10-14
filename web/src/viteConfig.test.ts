import { afterEach, describe, expect, it, vi } from "vitest";
import { API_PROXY, SESSION_WS_PROXY } from "../vite.config";

type ProxyEntry = { target: string; ws?: boolean; changeOrigin?: boolean; rewriteWsOrigin?: boolean };

async function loadProxy(env: Record<string, string | undefined>): Promise<Record<string, ProxyEntry> | undefined> {
  vi.resetModules();
  for (const [k, v] of Object.entries(env)) {
    vi.stubEnv(k, v as string | undefined);
  }
  try {
    const mod = await import("../vite.config");
    const factory = mod.default as (e: { command: string; mode: string }) => {
      server: { proxy?: Record<string, ProxyEntry> };
    };
    const cfg = await factory({ command: "serve", mode: "development" });
    return cfg.server.proxy;
  } finally {
    vi.unstubAllEnvs();
  }
}

describe("vite dev server proxy", () => {
  afterEach(() => {
    vi.resetModules();
    vi.unstubAllEnvs();
  });

  it("has no proxy when VITE_PROXY is unset", async () => {
    const proxy = await loadProxy({ VITE_PROXY: undefined });
    expect(proxy).toBeUndefined();
  });

  it("forwards API and session WebSockets to VITE_PROXY, defaulting a bare host to http", async () => {
    const proxy = await loadProxy({ VITE_PROXY: "http://127.0.0.1:8081" });
    const api = proxy?.[API_PROXY];
    expect(api?.target).toBe("http://127.0.0.1:8081");
    expect(api?.ws).toBe(true);
    expect(api?.changeOrigin).toBe(true);
    expect(api?.rewriteWsOrigin).toBeUndefined();
    expect(new RegExp(API_PROXY).test("/api/sessions")).toBe(true);
    expect(new RegExp(API_PROXY).test("/api/agent-graph/ws")).toBe(true);
    expect(new RegExp(API_PROXY).test("/apian")).toBe(false);

    const ws = proxy?.[SESSION_WS_PROXY];
    expect(ws?.target).toBe("ws://127.0.0.1:8081");
    expect(ws?.ws).toBe(true);
    expect(ws?.changeOrigin).toBe(true);
    expect(ws?.rewriteWsOrigin).toBeUndefined();
    expect(new RegExp(SESSION_WS_PROXY).test("/sessions/s1/ws")).toBe(true);
    expect(new RegExp(SESSION_WS_PROXY).test("/sessions/s1/acp/ws")).toBe(true);
    expect(new RegExp(SESSION_WS_PROXY).test("/sessions/s1/acp/ws?since=42")).toBe(true);
    expect(new RegExp(SESSION_WS_PROXY).test("/sessions/s1/live-ws")).toBe(true);
    expect(new RegExp(SESSION_WS_PROXY).test("/sessions/s1/terminal/live-ws")).toBe(true);
    expect(new RegExp(SESSION_WS_PROXY).test("/sessions/s1/container-terminal/live-ws")).toBe(true);

    const bare = await loadProxy({ VITE_PROXY: "localhost:50106" });
    expect(bare?.[API_PROXY].target).toBe("http://localhost:50106");
    expect(bare?.[SESSION_WS_PROXY].target).toBe("ws://localhost:50106");
  });
});
