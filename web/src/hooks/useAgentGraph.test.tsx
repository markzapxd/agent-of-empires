// @vitest-environment jsdom

import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FakeWebSocket, json, lastSocket, sockets } from "./__tests__/acpHarness";
import { useAgentGraph } from "./useAgentGraph";

const snapshot = { nodes: [], edges: [], cursor: 2 };

beforeEach(() => {
  sockets.length = 0;
  vi.stubGlobal("WebSocket", FakeWebSocket);
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => json(snapshot)),
  );
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("useAgentGraph", () => {
  it("fetches only read-only snapshot, upgrades with auth, deduplicates and keeps stale data", async () => {
    const { result, unmount } = renderHook(() => useAgentGraph());
    expect(lastSocket().url).toMatch(/\/api\/agent-graph\/ws$/);
    expect(lastSocket().protocols).toContain("aoe-auth");
    await waitFor(() => expect(result.current.snapshot?.cursor).toBe(2));
    expect(fetch).toHaveBeenCalledWith("/api/agent-graph", { signal: expect.any(AbortSignal) });
    lastSocket().open();
    lastSocket().message({ type: "update", snapshot: { ...snapshot, cursor: 3 } });
    lastSocket().message({ type: "update", snapshot: { ...snapshot, cursor: 2 } });
    expect(result.current.snapshot?.cursor).toBe(3);
    lastSocket().drop();
    expect(result.current.connection).toBe("reconnecting");
    expect(result.current.snapshot?.cursor).toBe(3);
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 250));
    });
    expect(sockets).toHaveLength(2);
    lastSocket().open();
    lastSocket().message({ type: "snapshot", snapshot: { ...snapshot, cursor: 4 } });
    expect(result.current.snapshot?.cursor).toBe(4);
    expect(result.current.connection).toBe("live");
    unmount();
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("rejects malformed nested records without replacing the last valid snapshot", async () => {
    const { result } = renderHook(() => useAgentGraph());
    await waitFor(() => expect(result.current.snapshot?.cursor).toBe(2));
    for (const invalid of [
      { nodes: [null], edges: [], cursor: 3 },
      { nodes: [{ status: null }], edges: [], cursor: 3 },
      { nodes: [], edges: [null], cursor: 3 },
      {
        nodes: [],
        edges: [{ id: "edge", source: "missing", target: "other", kind: "explicit_parent", evidence: "parent" }],
        cursor: 3,
      },
    ]) {
      lastSocket().message({ type: "update", snapshot: invalid });
      expect(result.current.snapshot).toEqual(snapshot);
    }
  });

  it("rejects a malformed GET snapshot and accepts a later socket snapshot", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => json({ nodes: [null], edges: [], cursor: 2 })),
    );
    const { result } = renderHook(() => useAgentGraph());
    await waitFor(() => expect(result.current.error).toBe(true));
    expect(result.current.snapshot).toBeNull();
    lastSocket().message({ type: "snapshot", snapshot });
    expect(result.current.error).toBe(false);
    expect(result.current.snapshot?.cursor).toBe(2);
  });

  it("reports unavailable GET without inventing data and accepts a later socket snapshot", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => json({ message: "Unavailable" }, 503)),
    );
    const { result } = renderHook(() => useAgentGraph());
    await waitFor(() => expect(result.current.error).toBe(true));
    expect(result.current.snapshot).toBeNull();
    lastSocket().message({ type: "snapshot", snapshot });
    expect(result.current.error).toBe(false);
    expect(result.current.snapshot?.cursor).toBe(2);
  });
});
