import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { listRuntimeAgents, type RuntimeAgentMetadata } from "./api";

type RuntimeAgentsContextValue = {
  agents: RuntimeAgentMetadata[];
  loading: boolean;
  refresh: () => Promise<void>;
};

const RuntimeAgentsContext = createContext<RuntimeAgentsContextValue | null>(null);

export function RuntimeAgentsProvider({ children }: { children: ReactNode }) {
  const [agents, setAgents] = useState<RuntimeAgentMetadata[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    const rows = await listRuntimeAgents();
    setAgents(rows.filter((agent) => agent.enabled && agent.configured));
  };

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    refresh()
      .catch((err) => {
        console.error("runtime agents load failed", err);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    let unlisten: (() => void) | null = null;
    listen("runtime_agents_updated", () => {
      refresh().catch((err) => {
        console.error("runtime agents refresh failed", err);
      });
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((err) => {
        console.error("runtime agents event listen failed", err);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const value = useMemo(
    () => ({ agents, loading, refresh }),
    [agents, loading],
  );

  return (
    <RuntimeAgentsContext.Provider value={value}>
      {children}
    </RuntimeAgentsContext.Provider>
  );
}

export function useRuntimeAgents() {
  const value = useContext(RuntimeAgentsContext);
  if (!value) throw new Error("useRuntimeAgents must be used within RuntimeAgentsProvider");
  return value;
}
