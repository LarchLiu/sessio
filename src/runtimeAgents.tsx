import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import {
  getLastRuntimeAgentSelection,
  listRuntimeAgents,
  setLastRuntimeAgentSelection,
  type RuntimeAgentMetadata,
  type RuntimeAgentSelection,
  type SetRuntimeAgentSelectionRequest,
} from "./api";

type RuntimeAgentsContextValue = {
  agents: RuntimeAgentMetadata[];
  lastSelection: RuntimeAgentSelection | null;
  loading: boolean;
  refresh: () => Promise<void>;
  rememberSelection: (selection: SetRuntimeAgentSelectionRequest) => Promise<void>;
};

const RuntimeAgentsContext = createContext<RuntimeAgentsContextValue | null>(null);

export function RuntimeAgentsProvider({ children }: { children: ReactNode }) {
  const [agents, setAgents] = useState<RuntimeAgentMetadata[]>([]);
  const [lastSelection, setLastSelection] = useState<RuntimeAgentSelection | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    const [rows, selection] = await Promise.all([
      listRuntimeAgents(),
      getLastRuntimeAgentSelection(),
    ]);
    setAgents(rows.filter((agent) => agent.enabled && agent.configured));
    setLastSelection(selection);
  };

  const rememberSelection = async (selection: SetRuntimeAgentSelectionRequest) => {
    setLastSelection((prev) => ({
      agent: selection.agent,
      model: selection.model ?? null,
      effort: selection.effort ?? null,
      permissionMode: selection.permissionMode ?? null,
      updatedAt: prev?.updatedAt ?? Date.now(),
    }));
    const saved = await setLastRuntimeAgentSelection(selection);
    setLastSelection(saved);
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
    () => ({ agents, lastSelection, loading, refresh, rememberSelection }),
    [agents, lastSelection, loading],
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

export function runtimeAgentForSelection(
  agents: RuntimeAgentMetadata[],
  selection: RuntimeAgentSelection | null,
): RuntimeAgentMetadata | null {
  if (!selection) return null;
  return agents.find((agent) => agent.agent === selection.agent) ?? null;
}

export function selectionModel(
  agent: RuntimeAgentMetadata,
  selection: RuntimeAgentSelection | null,
): string {
  const selected = selection?.agent === agent.agent ? selection.model : null;
  if (selected && agent.models.some((option) => option.value === selected)) return selected;
  return agent.model ?? agent.models[0]?.value ?? "";
}

export function selectionEffort(
  agent: RuntimeAgentMetadata,
  selection: RuntimeAgentSelection | null,
  fallback: (agent: RuntimeAgentMetadata | null) => string,
): string {
  const selected = selection?.agent === agent.agent ? selection.effort : null;
  if (selected && agent.efforts.some((option) => option.value === selected)) return selected;
  return fallback(agent);
}

export function selectionPermissionMode(
  agent: RuntimeAgentMetadata,
  selection: RuntimeAgentSelection | null,
): string {
  const selected = selection?.agent === agent.agent ? selection.permissionMode : null;
  if (selected && agent.permissionModes.some((option) => option.value === selected)) return selected;
  return agent.permissionMode ?? agent.permissionModes[0]?.value ?? "";
}
