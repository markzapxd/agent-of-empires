import type { Edge, Node } from "@xyflow/react";
import type { AgentGraphEdge, AgentGraphNode, AgentGraphSnapshot } from "../../lib/agentGraph";

export interface GraphFilters {
  project: string;
  agent: string;
  status: string;
  includeHistoric: boolean;
}

export interface ProjectMapNode {
  id: string;
  kind: "project";
  title: string;
  projectKey: string;
  projectPath: string;
  memberCount: number;
}

export type MapObservation = AgentGraphNode | ProjectMapNode;

export interface GraphNodeData extends Record<string, unknown> {
  observation: MapObservation;
  hiddenRelations: number;
  openSessionId: string | null;
  focused: boolean;
  muted: boolean;
  onActivate: (id: string) => void;
  onOpenSession: (id: string) => void;
}

export interface MapGraph {
  nodes: Node<GraphNodeData>[];
  edges: Edge[];
  focusedTitle: string | null;
  focusedRelations: number;
}

const PROJECT_SIZE = { width: 188, height: 112 };
const AGENT_SIZE = { width: 252, height: 184 };
const SUBAGENT_SIZE = { width: 176, height: 108 };

const compareNodes = (a: AgentGraphNode, b: AgentGraphNode): number =>
  a.created_at.localeCompare(b.created_at) || a.id.localeCompare(b.id);

const keyForProject = (node: AgentGraphNode): string => node.project_key || `workspace:${node.project_path}`;

const labelForProject = (path: string): string => {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) || path || "Workspace";
};

export function normalizeGraph(snapshot: AgentGraphSnapshot): AgentGraphSnapshot {
  const groups = new Map<string, AgentGraphNode[]>();
  for (const node of snapshot.nodes) {
    const key = node.kind === "session" && node.session_id ? `session:${node.session_id}` : `node:${node.id}`;
    const group = groups.get(key);
    if (group) group.push(node);
    else groups.set(key, [node]);
  }

  const aliases = new Map<string, string>();
  const nodes = [...groups.values()].map((group) => {
    const byId = [...group].sort((a, b) => a.id.localeCompare(b.id));
    const first = byId[0];
    if (!first) throw new Error("Agent Graph node group cannot be empty");
    const canonical = byId.find((node) => !node.session_id || node.id !== `session::${node.session_id}`) ?? first;
    const freshest =
      [...group].sort(
        (a, b) =>
          Number(b.available) - Number(a.available) ||
          b.updated_at.localeCompare(a.updated_at) ||
          a.id.localeCompare(b.id),
      )[0] ?? first;
    for (const node of group) aliases.set(node.id, canonical.id);
    return {
      ...freshest,
      id: canonical.id,
      project_key: freshest.project_key || keyForProject(freshest),
      history_complete: group.every((node) => node.history_complete),
    };
  });

  const seenEdges = new Set<string>();
  const edges: AgentGraphEdge[] = [];
  for (const edge of [...snapshot.edges].sort((a, b) => a.id.localeCompare(b.id))) {
    const source = aliases.get(edge.source);
    const target = aliases.get(edge.target);
    if (!source || !target || source === target) continue;
    const key = `${edge.kind}\0${source}\0${target}\0${edge.evidence}`;
    if (seenEdges.has(key)) continue;
    seenEdges.add(key);
    edges.push({ ...edge, source, target });
  }
  return { ...snapshot, nodes, edges };
}

export const isHistoric = (node: AgentGraphNode): boolean =>
  (node.kind === "session" && !node.available) ||
  node.ended_at !== null ||
  ["ended", "removed", "deleted", "completed"].includes(node.status.toLowerCase());

export const sessionForNode = (node: AgentGraphNode, nodes: AgentGraphNode[]): string | null => {
  if (node.kind === "session") return node.available ? node.session_id : null;
  const owner = nodes.find((candidate) => candidate.session_id === node.owner_session_id && candidate.available);
  return owner?.session_id ?? null;
};

export function filterGraph(snapshot: AgentGraphSnapshot, filters: GraphFilters) {
  const normalized = normalizeGraph(snapshot);
  const nodes = normalized.nodes.filter(
    (node) =>
      (filters.includeHistoric || !isHistoric(node)) &&
      (!filters.project || node.project_path === filters.project) &&
      (!filters.agent || node.agent === filters.agent) &&
      (!filters.status || node.status === filters.status),
  );
  const ids = new Set(nodes.map((node) => node.id));
  const edges = normalized.edges.filter((edge) => ids.has(edge.source) && ids.has(edge.target));
  const hiddenRelations = new Map(nodes.map((node) => [node.id, 0]));
  for (const edge of normalized.edges) {
    if (ids.has(edge.source) && !ids.has(edge.target))
      hiddenRelations.set(edge.source, (hiddenRelations.get(edge.source) ?? 0) + 1);
    if (ids.has(edge.target) && !ids.has(edge.source))
      hiddenRelations.set(edge.target, (hiddenRelations.get(edge.target) ?? 0) + 1);
  }
  return { nodes, edges, hiddenRelations, allNodes: normalized.nodes };
}

function projectGraph(nodes: AgentGraphNode[], edges: AgentGraphEdge[]) {
  const byProject = new Map<string, AgentGraphNode[]>();
  for (const node of nodes) {
    const key = keyForProject(node);
    const group = byProject.get(key);
    if (group) group.push(node);
    else byProject.set(key, [node]);
  }

  const projects: ProjectMapNode[] = [...byProject.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([projectKey, members]) => {
      const path = [...members].sort(compareNodes)[0]?.project_path ?? "";
      return {
        id: `project:${projectKey}`,
        kind: "project",
        title: labelForProject(path),
        projectKey,
        projectPath: path,
        memberCount: members.length,
      };
    });
  const projectIds = new Set(projects.map((project) => project.id));
  const ownedChildren = new Set(
    edges
      .filter((edge) => edge.kind === "native_subagent" || edge.kind === "explicit_parent")
      .map((edge) => edge.target),
  );
  const projectEdges: AgentGraphEdge[] = [];
  for (const node of nodes) {
    if (node.kind === "subagent" && ownedChildren.has(node.id)) continue;
    const source = `project:${keyForProject(node)}`;
    if (!projectIds.has(source)) continue;
    projectEdges.push({
      id: `project_member:${source}:${node.id}`,
      source,
      target: node.id,
      kind: "project_member",
      evidence: "workspace_repository",
    });
  }
  return { projects, edges: [...projectEdges, ...edges] };
}

function treePositions(projects: ProjectMapNode[], observations: AgentGraphNode[], edges: AgentGraphEdge[]) {
  const positions = new Map<string, { x: number; y: number }>();
  const byProject = new Map<string, AgentGraphNode[]>();
  for (const node of observations) {
    const key = keyForProject(node);
    const group = byProject.get(key);
    if (group) group.push(node);
    else byProject.set(key, [node]);
  }
  const parentByChild = new Map<string, string>();
  for (const edge of edges) {
    if (edge.kind === "explicit_parent" || edge.kind === "native_subagent") {
      parentByChild.set(edge.target, edge.source);
    }
  }
  const projectGap = 96;
  const agentGap = 32;
  const subagentGap = 24;
  let projectX = 0;
  for (const project of projects) {
    const members = [...(byProject.get(project.projectKey) ?? [])].sort(compareNodes);
    const roots = members.filter((node) => !parentByChild.has(node.id));
    const childrenByParent = new Map<string, AgentGraphNode[]>();
    for (const node of members) {
      const parent = parentByChild.get(node.id);
      if (!parent) continue;
      const children = childrenByParent.get(parent);
      if (children) children.push(node);
      else childrenByParent.set(parent, [node]);
    }
    const rootWidths = roots.map((root) => {
      const children = childrenByParent.get(root.id) ?? [];
      return Math.max(AGENT_SIZE.width, children.length * (SUBAGENT_SIZE.width + subagentGap) - subagentGap);
    });
    const projectWidth = Math.max(
      PROJECT_SIZE.width,
      rootWidths.reduce((sum, width) => sum + width, 0) + Math.max(0, roots.length - 1) * agentGap,
    );
    const projectLeft = projectX;
    positions.set(project.id, { x: projectLeft + (projectWidth - PROJECT_SIZE.width) / 2, y: 0 });
    let rootX = projectLeft;
    for (const [index, root] of roots.entries()) {
      const width = rootWidths[index] ?? AGENT_SIZE.width;
      positions.set(root.id, { x: rootX + (width - AGENT_SIZE.width) / 2, y: PROJECT_SIZE.height + 96 });
      const children = childrenByParent.get(root.id) ?? [];
      const childWidth = children.length * (SUBAGENT_SIZE.width + subagentGap) - subagentGap;
      let childX = rootX + (width - Math.max(0, childWidth)) / 2;
      for (const child of children) {
        positions.set(child.id, { x: childX, y: PROJECT_SIZE.height + 96 + AGENT_SIZE.height + 72 });
        childX += SUBAGENT_SIZE.width + subagentGap;
      }
      rootX += width + agentGap;
    }
    const unplaced = members.filter((node) => !positions.has(node.id));
    for (const [index, node] of unplaced.entries()) {
      positions.set(node.id, {
        x: projectLeft + index * (SUBAGENT_SIZE.width + subagentGap),
        y: PROJECT_SIZE.height + 96 + AGENT_SIZE.height + 72,
      });
    }
    projectX += projectWidth + projectGap;
  }
  return positions;
}

const relationLabel = (edge: AgentGraphEdge): string | undefined => {
  if (edge.kind === "explicit_parent") return "Delegation";
  if (edge.kind === "native_subagent") return "Supervisor context";
  return undefined;
};

const edgeActive = (edge: AgentGraphEdge, nodes: AgentGraphNode[]) =>
  nodes.some((node) => node.id === edge.source && node.status.toLowerCase() === "running") ||
  nodes.some((node) => node.id === edge.target && node.status.toLowerCase() === "running");

export function layoutGraph(
  observations: AgentGraphNode[],
  inputEdges: AgentGraphEdge[],
  hiddenRelations: Map<string, number>,
  allNodes: AgentGraphNode[],
  selectedId: string | null,
  onActivate: (id: string) => void,
  onOpenSession: (id: string) => void,
): MapGraph {
  const { projects, edges } = projectGraph(observations, inputEdges);
  const mapNodes: MapObservation[] = [...projects, ...observations.sort(compareNodes)];
  const positions = treePositions(projects, observations, edges);
  const adjacent = new Set<string>();
  if (selectedId) {
    adjacent.add(selectedId);
    for (const edge of edges) {
      if (edge.source === selectedId) adjacent.add(edge.target);
      if (edge.target === selectedId) adjacent.add(edge.source);
    }
  }
  const selected = mapNodes.find((node) => node.id === selectedId) ?? null;
  return {
    nodes: mapNodes.map((node) => ({
      id: node.id,
      type: "agent",
      position: positions.get(node.id) ?? { x: 0, y: 0 },
      data: {
        observation: node,
        hiddenRelations: "projectKey" in node ? (hiddenRelations.get(node.id) ?? 0) : 0,
        openSessionId: "projectKey" in node ? null : sessionForNode(node, allNodes),
        focused: node.id === selectedId,
        muted: selectedId !== null && !adjacent.has(node.id),
        onActivate,
        onOpenSession,
      },
    })),
    edges: edges.map((edge) => {
      const focused = selectedId !== null && (edge.source === selectedId || edge.target === selectedId);
      return {
        id: edge.id,
        source: edge.source,
        target: edge.target,
        sourceHandle: "bottom",
        targetHandle: "top",
        type: "smoothstep",
        label: relationLabel(edge),
        data: { evidence: edge.evidence, kind: edge.kind },
        title: edge.evidence,
        selectable: false,
        animated: edge.kind !== "project_member" && edgeActive(edge, allNodes),
        className:
          selectedId !== null && !focused
            ? "opacity-20"
            : focused && edge.kind !== "project_member"
              ? "motion-safe:animate-pulse"
              : undefined,
        style:
          edge.kind === "project_member"
            ? { stroke: "var(--color-text-muted)", strokeDasharray: "3 4" }
            : edge.kind === "explicit_parent"
              ? { stroke: "var(--color-status-running)", strokeWidth: 2 }
              : { stroke: "var(--color-text-secondary)", strokeWidth: 1.5 },
      };
    }),
    focusedTitle: selected?.title ?? null,
    focusedRelations: selectedId ? Math.max(0, adjacent.size - 1) : 0,
  };
}

export function formatDuration(node: AgentGraphNode, now: number): string {
  const start = Date.parse(node.started_at ?? node.created_at);
  const end = node.ended_at ? Date.parse(node.ended_at) : now;
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return "Unknown";
  const seconds = Math.floor((end - start) / 1000);
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}
