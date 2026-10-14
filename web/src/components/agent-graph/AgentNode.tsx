import { Handle, Position, type NodeProps, type Node } from "@xyflow/react";
import { useEffect, useState } from "react";
import type { GraphNodeData, MapObservation, ProjectMapNode } from "./graphModel";
import { formatDuration, isHistoric } from "./graphModel";

const statusTone: Record<string, string> = {
  running: "text-status-running",
  completed: "text-status-running",
  waiting: "text-status-waiting",
  error: "text-status-error",
};

const isProjectNode = (node: MapObservation): node is ProjectMapNode => "projectKey" in node;

const projectNode = (node: ProjectMapNode, data: GraphNodeData) => (
  <button
    type="button"
    onClick={() => data.onActivate(node.id)}
    className={`h-28 overflow-hidden rounded-lg border border-brand-600 bg-surface-800 p-3 text-left shadow-md transition-colors motion-reduce:transition-none ${
      data.focused ? "ring-2 ring-status-running ring-offset-2 ring-offset-surface-950" : "hover:bg-surface-700"
    } ${data.muted ? "opacity-35" : ""}`}
    aria-label={`Project ${node.title}, ${node.memberCount} agents`}
    aria-pressed={data.focused}
  >
    <Handle type="target" position={Position.Top} id="top" isConnectable={false} />
    <Handle type="target" position={Position.Left} id="left" isConnectable={false} />
    <span className="block font-mono text-[11px] uppercase tracking-wide text-text-muted">Project hub</span>
    <span className="mt-1 block truncate text-sm font-semibold text-text-primary" title={node.title}>
      {node.title}
    </span>
    <span className="mt-2 block font-mono text-xs text-text-secondary">
      {node.memberCount} {node.memberCount === 1 ? "agent" : "agents"}
    </span>
    <Handle type="source" position={Position.Bottom} id="bottom" isConnectable={false} />
    <Handle type="source" position={Position.Right} id="right" isConnectable={false} />
  </button>
);

const agentNode = (node: Exclude<MapObservation, ProjectMapNode>, data: GraphNodeData, now: number) => {
  const subagent = node.kind === "subagent";
  const historic = isHistoric(node);
  const isOwnerFallback = subagent && data.openSessionId !== null;
  const label = isOwnerFallback ? `Open supervisor session for ${node.title}` : `Open session ${node.title}`;
  return (
    <div
      className={`overflow-hidden rounded-lg border border-surface-600 bg-surface-800 p-3 text-text-primary shadow-md transition-colors motion-reduce:transition-none ${
        data.focused ? "ring-2 ring-status-running ring-offset-2 ring-offset-surface-950" : ""
      } ${data.muted ? "opacity-35" : ""} ${subagent ? "w-44 min-h-27" : "w-63 min-h-46"}`}
    >
      <Handle type="target" position={Position.Top} id="top" isConnectable={false} />
      <Handle type="target" position={Position.Left} id="left" isConnectable={false} />
      <button
        type="button"
        onClick={() => data.onActivate(node.id)}
        className="nodrag nopan block w-full text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-status-running"
        aria-label={`${subagent ? "Subagent" : "Agent"} ${node.title}`}
        aria-pressed={data.focused}
      >
        <span className="flex items-start justify-between gap-2">
          <span className="font-mono text-xs font-semibold truncate" title={node.title}>
            {node.title || "Untitled session"}
          </span>
          <span className={`shrink-0 font-mono text-[11px] ${statusTone[node.status] ?? "text-text-secondary"}`}>
            <span aria-hidden="true">{node.status === "running" ? "● " : node.status === "error" ? "! " : "○ "}</span>
            {node.status}
          </span>
        </span>
        <span className="mt-1 block font-mono text-[11px] uppercase tracking-wide text-text-muted">
          {subagent ? "Subagent" : "Agent"}
        </span>
        {!subagent && (
          <span
            className="mt-2 block line-clamp-2 text-xs wrap-anywhere text-text-secondary"
            title={node.task ?? undefined}
          >
            Task: {node.task?.trim() || "Not provided"}
          </span>
        )}
      </button>
      <dl
        className={`grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 text-[11px] text-text-muted ${subagent ? "mt-2" : "mt-3"}`}
      >
        <dt>Agent</dt>
        <dd className="truncate font-mono" title={node.agent}>
          {node.agent_type ? `${node.agent} · ${node.agent_type}` : node.agent || "Unknown"}
        </dd>
        {!subagent && (
          <>
            <dt>Duration</dt>
            <dd className="font-mono tabular-nums">{formatDuration(node, now || Date.parse(node.updated_at))}</dd>
          </>
        )}
      </dl>
      {!subagent && (historic || !node.history_complete || data.hiddenRelations > 0) && (
        <div className="mt-2 flex flex-wrap gap-1 text-[11px] text-text-secondary">
          {historic && <span>Historical</span>}
          {!node.history_complete && <span>Earlier history incomplete</span>}
          {data.hiddenRelations > 0 && <span>{data.hiddenRelations} filtered relationship(s)</span>}
        </div>
      )}
      {data.openSessionId && (
        <button
          type="button"
          className="nodrag nopan mt-2 rounded-md border border-surface-600 px-2 py-1 text-xs text-text-primary hover:bg-surface-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-status-running"
          onClick={() => data.onOpenSession(data.openSessionId!)}
          aria-label={label}
        >
          {isOwnerFallback ? "Open supervisor session" : "Open session"}
        </button>
      )}
      <Handle type="source" position={Position.Bottom} id="bottom" isConnectable={false} />
      <Handle type="source" position={Position.Right} id="right" isConnectable={false} />
    </div>
  );
};

export function AgentNode({ data }: NodeProps<Node<GraphNodeData>>) {
  const { observation: node } = data;
  const [now, setNow] = useState(0);
  const agent = isProjectNode(node) ? null : node;
  useEffect(() => {
    if (!agent || agent.ended_at) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [agent]);
  if (isProjectNode(node)) return projectNode(node, data);
  return agentNode(node, data, now);
}
