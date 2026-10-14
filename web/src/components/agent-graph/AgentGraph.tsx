import { useCallback, useMemo, useState, type CSSProperties } from "react";
import { ReactFlow, Background, Controls, MiniMap, ReactFlowProvider } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useAgentGraph } from "../../hooks/useAgentGraph";
import { AgentNode } from "./AgentNode";
import { filterGraph, layoutGraph, type GraphFilters } from "./graphModel";

const nodeTypes = { agent: AgentNode };
const graphTheme = {
  "--xy-background-color": "var(--color-surface-950)",
  "--xy-edge-stroke": "var(--color-text-secondary)",
  "--xy-edge-label-background-color": "var(--color-surface-800)",
  "--xy-edge-label-color": "var(--color-text-primary)",
  "--xy-controls-button-background-color": "var(--color-surface-800)",
  "--xy-controls-button-background-color-hover": "var(--color-surface-700)",
  "--xy-controls-button-color": "var(--color-text-primary)",
  "--xy-controls-button-border-color": "var(--color-surface-600)",
  "--xy-handle-background-color": "var(--color-text-secondary)",
  "--xy-handle-border-color": "var(--color-surface-800)",
  "--xy-minimap-background-color": "var(--color-surface-900)",
  "--xy-minimap-mask-background-color": "color-mix(in srgb, var(--color-surface-950) 65%, transparent)",
  "--xy-minimap-node-background-color": "var(--color-text-secondary)",
  "--xy-background-pattern-dots-color": "var(--color-surface-600)",
} as CSSProperties;

interface Props {
  onOpenSession: (sessionId: string) => void;
  onBack: () => void;
}

export function AgentGraph({ onOpenSession, onBack }: Props) {
  const { snapshot, connection, error } = useAgentGraph();
  const [filters, setFilters] = useState<GraphFilters>({
    project: "",
    agent: "",
    status: "",
    includeHistoric: false,
  });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const options = useMemo(
    () => ({
      projects: [...new Set(snapshot?.nodes.map((node) => node.project_path).filter(Boolean) ?? [])].sort(),
      agents: [...new Set(snapshot?.nodes.map((node) => node.agent).filter(Boolean) ?? [])].sort(),
      statuses: [...new Set(snapshot?.nodes.map((node) => node.status).filter(Boolean) ?? [])].sort(),
    }),
    [snapshot],
  );
  const filtered = useMemo(
    () =>
      snapshot
        ? filterGraph(snapshot, filters)
        : { nodes: [], edges: [], hiddenRelations: new Map<string, number>(), allNodes: [] },
    [snapshot, filters],
  );
  const activate = useCallback(
    (id: string) => {
      if (id !== selectedId) {
        setSelectedId(id);
        return;
      }
      if (id.startsWith("project:")) {
        setSelectedId(null);
        return;
      }
      const node = filtered.nodes.find((candidate) => candidate.id === id);
      if (!node) return;
      const sessionId = node.kind === "session" && node.available ? node.session_id : null;
      const owner = filtered.allNodes.find(
        (candidate) => candidate.session_id === node.owner_session_id && candidate.available,
      );
      const target = sessionId ?? owner?.session_id;
      if (target) onOpenSession(target);
    },
    [filtered.allNodes, filtered.nodes, onOpenSession, selectedId],
  );
  const graph = useMemo(
    () =>
      layoutGraph(
        filtered.nodes,
        filtered.edges,
        filtered.hiddenRelations,
        filtered.allNodes,
        selectedId,
        activate,
        onOpenSession,
      ),
    [activate, filtered, onOpenSession, selectedId],
  );
  const change = (name: keyof GraphFilters, value: string | boolean) => {
    setSelectedId(null);
    setFilters((previous) => ({ ...previous, [name]: value }));
  };

  return (
    <main className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface-950" aria-label="Agent Graph">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-surface-700 px-4 py-3">
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={onBack}
            className="rounded-md border border-surface-600 px-2 py-1.5 text-xs text-text-secondary hover:text-text-primary focus-visible:outline-2 focus-visible:outline-status-running"
          >
            Back to dashboard
          </button>
          <h1 className="text-base font-semibold text-text-primary">Agent Graph</h1>
        </div>
        <span className="font-mono text-xs text-text-secondary" role="status" aria-live="polite">
          {connection === "live" ? "Live" : snapshot ? "Disconnected · showing last snapshot" : "Connecting"}
        </span>
      </div>
      <div className="flex flex-wrap items-center gap-3 border-b border-surface-700 px-4 py-2 text-xs text-text-secondary">
        <label className="flex items-center gap-1">
          Project
          <select
            aria-label="Filter project"
            value={filters.project}
            onChange={(event) => change("project", event.target.value)}
            className="max-w-44 rounded-md border border-surface-600 bg-surface-800 px-2 py-1.5 text-text-primary"
          >
            <option value="">All projects</option>
            {options.projects.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1">
          Agent
          <select
            aria-label="Filter agent"
            value={filters.agent}
            onChange={(event) => change("agent", event.target.value)}
            className="max-w-40 rounded-md border border-surface-600 bg-surface-800 px-2 py-1.5 text-text-primary"
          >
            <option value="">All agents</option>
            {options.agents.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1">
          Status
          <select
            aria-label="Filter status"
            value={filters.status}
            onChange={(event) => change("status", event.target.value)}
            className="rounded-md border border-surface-600 bg-surface-800 px-2 py-1.5 text-text-primary"
          >
            <option value="">All statuses</option>
            {options.statuses.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={filters.includeHistoric}
            onChange={(event) => change("includeHistoric", event.target.checked)}
          />
          Include historical
        </label>
        <span className="font-mono text-[11px] text-text-muted">
          Click a node to focus it; click again to open its session.
        </span>
      </div>
      <p className="sr-only" aria-live="polite">
        {graph.focusedTitle ? `${graph.focusedTitle} focused, ${graph.focusedRelations} direct relationships.` : ""}
      </p>
      {error && !snapshot && (
        <p role="alert" className="px-4 py-3 text-status-error">
          Could not load the graph. Reconnecting…
        </p>
      )}
      {!snapshot && !error && <p className="px-4 py-3 text-text-secondary">Loading agent relationships…</p>}
      {snapshot && !filtered.nodes.length && (
        <p className="px-4 py-3 text-text-secondary">
          {snapshot.nodes.length ? "No agents match the current filters." : "No observed agents yet."}
        </p>
      )}
      {filtered.nodes.length > 0 && (
        <div className="relative min-h-64 flex-1" aria-label="Agent relationship diagram">
          <ReactFlowProvider>
            <ReactFlow
              nodes={graph.nodes}
              edges={graph.edges}
              nodeTypes={nodeTypes}
              style={graphTheme}
              nodesConnectable={false}
              nodesDraggable={true}
              edgesReconnectable={false}
              elementsSelectable={true}
              fitView
              fitViewOptions={{ padding: 0.2 }}
              minZoom={0.25}
              maxZoom={2}
            >
              <Background />
              <Controls showInteractive={false} />
              <MiniMap pannable zoomable />
            </ReactFlow>
          </ReactFlowProvider>
        </div>
      )}
      <div className="border-t border-surface-700 px-4 py-2 text-xs text-text-secondary">
        <p>
          Project hubs group workspace/repository activity. Delegation is an explicit AoE parent; supervisor context is
          a native hook association, not a confirmed direct parent.
        </p>
        <p>Filtered relationships are not reconnected. Historical or deleted sessions have no terminal link.</p>
        {filtered.edges.length > 0 && (
          <details className="mt-1">
            <summary className="cursor-pointer">Relationship evidence ({filtered.edges.length})</summary>
            <ul className="max-h-24 overflow-auto font-mono text-[11px]">
              {filtered.edges.map((edge) => (
                <li key={edge.id}>
                  {edge.source} → {edge.target}: {edge.kind === "explicit_parent" ? "Delegation" : "Supervisor context"}{" "}
                  ({edge.evidence})
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </main>
  );
}
