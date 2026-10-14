export interface AgentGraphNode {
  id: string;
  session_id: string | null;
  owner_session_id: string | null;
  kind: string;
  title: string;
  project_path: string;
  project_key?: string;
  agent: string;
  agent_type: string | null;
  status: string;
  task: string | null;
  created_at: string;
  started_at: string | null;
  ended_at: string | null;
  updated_at: string;
  available: boolean;
  history_complete: boolean;
}

export interface AgentGraphEdge {
  id: string;
  source: string;
  target: string;
  kind: string;
  evidence: string;
}

export interface AgentGraphSnapshot {
  nodes: AgentGraphNode[];
  edges: AgentGraphEdge[];
  cursor: number;
}

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null;
const nullableString = (value: unknown) => value === null || typeof value === "string";

export function isAgentGraphSnapshot(value: unknown): value is AgentGraphSnapshot {
  if (!isRecord(value) || !Number.isSafeInteger(value.cursor) || (value.cursor as number) < 0) return false;
  if (!Array.isArray(value.nodes) || !Array.isArray(value.edges)) return false;
  if (value.nodes.length > 2000 || value.edges.length > 10000) return false;
  const ids = new Set<string>();
  for (const node of value.nodes) {
    if (!isRecord(node)) return false;
    if (
      !["id", "kind", "title", "project_path", "agent", "status", "created_at", "updated_at"].every(
        (key) => typeof node[key] === "string",
      )
    )
      return false;
    if (
      !["session_id", "owner_session_id", "agent_type", "task", "started_at", "ended_at"].every((key) =>
        nullableString(node[key]),
      )
    )
      return false;
    if (node.project_key !== undefined && typeof node.project_key !== "string") return false;
    if (typeof node.available !== "boolean" || typeof node.history_complete !== "boolean") return false;
    if (!node.id || ids.has(node.id as string)) return false;
    ids.add(node.id as string);
  }
  const edgeIds = new Set<string>();
  return value.edges.every((edge) => {
    if (
      !isRecord(edge) ||
      !["id", "source", "target", "kind", "evidence"].every((key) => typeof edge[key] === "string")
    )
      return false;
    if (!edge.id || edgeIds.has(edge.id as string)) return false;
    edgeIds.add(edge.id as string);
    return ids.has(edge.source as string) && ids.has(edge.target as string);
  });
}
