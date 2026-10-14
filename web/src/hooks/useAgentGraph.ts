import { useEffect, useState } from "react";
import { fetchAgentGraph } from "../lib/api";
import { isAgentGraphSnapshot, type AgentGraphSnapshot } from "../lib/agentGraph";
import { getOrCreateDeviceBindingSecret } from "../lib/deviceBinding";
import { getToken } from "../lib/token";
import { retryDelayMs } from "../lib/wsBackoff";

type Connection = "connecting" | "live" | "reconnecting";

function socketProtocols(): string[] {
  const protocols = ["aoe-auth"];
  const token = getToken();
  if (token) protocols.push(token);
  try {
    protocols.push(`aoe-device.${getOrCreateDeviceBindingSecret()}`);
  } catch {
    // The server will reject upgrades missing the device binding.
  }
  return protocols;
}

export function useAgentGraph() {
  const [snapshot, setSnapshot] = useState<AgentGraphSnapshot | null>(null);
  const [connection, setConnection] = useState<Connection>("connecting");
  const [error, setError] = useState(false);

  useEffect(() => {
    let disposed = false;
    let ws: WebSocket | null = null;
    let retry: ReturnType<typeof setTimeout> | null = null;
    let attempt = 0;
    let cursor = -1;
    const controller = new AbortController();

    const apply = (next: unknown) => {
      if (!isAgentGraphSnapshot(next) || next.cursor < cursor) return;
      if (next.cursor === cursor && cursor >= 0) return;
      cursor = next.cursor;
      setSnapshot(next);
      setError(false);
    };

    const connect = () => {
      if (disposed) return;
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const socket = new WebSocket(`${protocol}//${window.location.host}/api/agent-graph/ws`, socketProtocols());
      ws = socket;
      socket.onopen = () => {
        if (disposed || ws !== socket) return;
        attempt = 0;
        setConnection("live");
        setError(false);
      };
      socket.onmessage = (event: MessageEvent) => {
        if (disposed || ws !== socket || typeof event.data !== "string") return;
        try {
          const frame = JSON.parse(event.data) as { type?: string; snapshot?: unknown };
          if (frame.type === "snapshot" || frame.type === "update") apply(frame.snapshot);
        } catch {
          // A malformed frame must not replace the last valid snapshot.
        }
      };
      socket.onclose = () => {
        if (disposed || ws !== socket) return;
        ws = null;
        setConnection("reconnecting");
        retry = setTimeout(connect, retryDelayMs(++attempt));
      };
      socket.onerror = () => {
        if (ws === socket) socket.close();
      };
    };

    // Subscribe immediately. The server sends a fresh snapshot after each upgrade;
    // GET is useful for rendering while the socket handshake is still pending.
    // WebSocket messages are asynchronous transport events, not a synchronous external store.
    // eslint-disable-next-line react-you-might-not-need-an-effect/no-external-store-subscription, react-you-might-not-need-an-effect/no-initialize-state
    connect();
    void fetchAgentGraph(controller.signal).then((initial) => {
      if (disposed) return;
      if (initial) apply(initial);
      else if (cursor < 0) setError(true);
    });
    return () => {
      disposed = true;
      controller.abort();
      if (retry) clearTimeout(retry);
      ws?.close();
    };
  }, []);

  return { snapshot, connection, error };
}
