import { describe, expect, it, vi } from "vitest";
import type { AgentGraphNode, AgentGraphSnapshot } from "../../../lib/agentGraph";
import { filterGraph, formatDuration, layoutGraph, normalizeGraph, sessionForNode } from "../graphModel";

const session = (overrides: Partial<AgentGraphNode> = {}): AgentGraphNode => ({
  id: "root",
  session_id: "session-1",
  owner_session_id: null,
  kind: "session",
  title: "Supervisor",
  project_path: "/repo",
  project_key: "workspace:/repo",
  agent: "claude",
  agent_type: null,
  status: "running",
  task: null,
  created_at: "2026-01-01T00:00:00Z",
  started_at: "2026-01-01T00:01:00Z",
  ended_at: null,
  updated_at: "2026-01-01T00:01:00Z",
  available: true,
  history_complete: true,
  ...overrides,
});
const native = session({
  id: "native",
  session_id: null,
  owner_session_id: "session-1",
  kind: "subagent",
  title: "Child",
  agent: "codex",
  available: false,
});
const snapshot: AgentGraphSnapshot = {
  nodes: [native, session(), session({ id: "archived", session_id: "session-2", available: false })],
  edges: [{ id: "one", source: "root", target: "native", kind: "native_subagent", evidence: "hook_context" }],
  cursor: 5,
};

const layout = (graph: AgentGraphSnapshot, selectedId: string | null = null) => {
  const filtered = filterGraph(graph, { project: "", agent: "", status: "", includeHistoric: false });
  return layoutGraph(
    filtered.nodes,
    filtered.edges,
    filtered.hiddenRelations,
    graph.nodes,
    selectedId,
    vi.fn(),
    vi.fn(),
  );
};

describe("Agent Graph model", () => {
  it("filters without reconnecting links and shows hidden relationship counts", () => {
    const filtered = filterGraph(snapshot, { project: "", agent: "claude", status: "", includeHistoric: false });
    expect(filtered.nodes.map((node) => node.id)).toEqual(["root"]);
    expect(filtered.edges).toEqual([]);
    expect(filtered.hiddenRelations.get("root")).toBe(1);
    expect(
      filterGraph(snapshot, { project: "/elsewhere", agent: "", status: "", includeHistoric: true }).nodes,
    ).toEqual([]);
    expect(
      filterGraph(snapshot, { project: "", agent: "", status: "running", includeHistoric: false }).nodes.map(
        (node) => node.id,
      ),
    ).toEqual(["native", "root"]);
  });

  it("only opens a native child through its available supervisor, never a removed terminal", () => {
    expect(sessionForNode(native, snapshot.nodes)).toBe("session-1");
    expect(sessionForNode(native, [session({ available: false })])).toBeNull();
    expect(sessionForNode(session({ available: false }), snapshot.nodes)).toBeNull();
  });

  it("derives one project hub, keeps confirmed edges, and omits labels from project links", () => {
    const graph = layout(snapshot);
    const project = graph.nodes.find((node) => node.id === "project:workspace:/repo");
    expect(project?.data.observation).toMatchObject({ kind: "project", title: "repo", memberCount: 2 });
    expect(graph.edges).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: "project_member:project:workspace:/repo:root", label: undefined }),
        expect.objectContaining({ source: "root", target: "native", label: "Supervisor context" }),
      ]),
    );
  });

  it("uses a deterministic non-linear constellation and focus neighborhood", () => {
    const graph = {
      ...snapshot,
      nodes: [
        ...snapshot.nodes,
        session({ id: "second", session_id: "session-3", project_path: "/other", project_key: "workspace:/other" }),
        session({ id: "third", session_id: "session-4" }),
      ],
    };
    const first = layout(graph);
    const second = layout({ ...graph, nodes: [...graph.nodes].reverse() });
    expect(first.nodes.map(({ id, position }) => [id, position])).toEqual(
      second.nodes.map(({ id, position }) => [id, position]),
    );
    expect(new Set(first.nodes.map((node) => node.position.x)).size).toBeGreaterThan(2);
    expect(new Set(first.nodes.map((node) => node.position.y)).size).toBeGreaterThan(2);
    const focused = layout(snapshot, "root");
    expect(focused.nodes.find((node) => node.id === "root")?.data.focused).toBe(true);
    expect(focused.nodes.find((node) => node.id === "native")?.data.muted).toBe(false);
    expect(focused.nodes.find((node) => node.id === "project:workspace:/repo")?.data.muted).toBe(false);
  });

  it("collapses duplicate session identities and remaps their relationships", () => {
    const legacy = session({ id: "session::session-1", updated_at: "2026-01-01T00:00:30Z" });
    const canonical = session({ id: "session:main:session-1", updated_at: "2026-01-01T00:02:00Z" });
    const graph = normalizeGraph({
      cursor: 8,
      nodes: [legacy, canonical, native],
      edges: [
        { id: "legacy-edge", source: legacy.id, target: native.id, kind: "native_subagent", evidence: "hook_context" },
        {
          id: "canonical-edge",
          source: canonical.id,
          target: native.id,
          kind: "native_subagent",
          evidence: "hook_context",
        },
      ],
    });
    expect(graph.nodes.map((node) => node.id).sort()).toEqual(["native", "session:main:session-1"]);
    expect(graph.edges).toHaveLength(1);
    expect(graph.edges[0]).toMatchObject({ source: "session:main:session-1", target: "native" });
  });

  it("freezes duration at end time and handles invalid timestamps", () => {
    expect(formatDuration(session({ ended_at: "2026-01-01T01:01:00Z" }), Date.now())).toBe("1h 0m");
    expect(formatDuration(session({ created_at: "invalid", started_at: null }), 100)).toBe("Unknown");
  });
});
