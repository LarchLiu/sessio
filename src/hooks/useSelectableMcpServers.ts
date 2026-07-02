import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";
import {
  getMcpSettings,
  type McpServerConfig,
  type RuntimeCapabilitySet,
} from "../api";
import { getSessioPromptMarkers } from "../promptMarkers";

const SESSIO_PROMPT_MARKERS = getSessioPromptMarkers();

export function normalizeSelectedMcpIds(ids: readonly string[]): string[] {
  const seen = new Set<string>();
  for (const id of ids) {
    const normalized = id.trim();
    if (!normalized) continue;
    seen.add(normalized);
  }
  return Array.from(seen).sort((left, right) => left.localeCompare(right));
}

function supportsMcpTransport(
  server: McpServerConfig,
  capabilities: RuntimeCapabilitySet | null | undefined,
): boolean {
  if (server.transport === "stdio") return true;
  if (server.transport === "http") return Boolean(capabilities?.mcpInjection?.http);
  return Boolean(capabilities?.mcpInjection?.sse);
}

export function useSelectableMcpServers(
  capabilities: RuntimeCapabilitySet | null | undefined,
) {
  const [servers, setServers] = useState<McpServerConfig[]>([]);
  const [selectedMcpIds, setSelectedMcpIds] = useState<string[]>([]);

  useEffect(() => {
    let disposed = false;

    const load = async () => {
      try {
        const settings = await getMcpSettings();
        if (disposed) return;
        setServers(settings.servers);
      } catch {
        if (!disposed) setServers([]);
      }
    };

    void load();
    const unlistenPromise = listen("app_config_updated", () => {
      void load();
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const availableMcpServers = useMemo(
    () =>
      servers
        .filter((server) =>
          server.source === SESSIO_PROMPT_MARKERS.mcpSourceCustom
          && server.enabled
          && supportsMcpTransport(server, capabilities),
        )
        .sort((left, right) =>
          `${left.name}:${left.id}`.localeCompare(`${right.name}:${right.id}`),
        ),
    [capabilities, servers],
  );

  useEffect(() => {
    const availableIds = new Set(availableMcpServers.map((server) => server.id));
    setSelectedMcpIds((current) => current.filter((id) => availableIds.has(id)));
  }, [availableMcpServers]);

  const selectedMcpServers = useMemo(
    () =>
      selectedMcpIds
        .map((id) => availableMcpServers.find((server) => server.id === id) ?? null)
        .filter((server): server is McpServerConfig => Boolean(server)),
    [availableMcpServers, selectedMcpIds],
  );

  const toggleMcpSelection = (mcpId: string) => {
    setSelectedMcpIds((current) =>
      normalizeSelectedMcpIds(
        current.includes(mcpId)
          ? current.filter((id) => id !== mcpId)
          : [...current, mcpId],
      ),
    );
  };

  const clearSelectedMcps = () => {
    setSelectedMcpIds([]);
  };

  return {
    availableMcpServers,
    selectedMcpIds,
    selectedMcpServers,
    toggleMcpSelection,
    clearSelectedMcps,
  };
}
