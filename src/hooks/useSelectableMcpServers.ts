import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";
import {
  getMcpSettings,
  type McpServerConfig,
  type RuntimeCapabilitySet,
} from "../api";

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
  options: { filterByCapabilities?: boolean } = {},
) {
  const [servers, setServers] = useState<McpServerConfig[]>([]);
  const [serversLoaded, setServersLoaded] = useState(false);
  const [selectedMcpIds, setSelectedMcpIds] = useState<string[]>([]);

  useEffect(() => {
    let disposed = false;

    const load = async () => {
      try {
        const settings = await getMcpSettings();
        if (disposed) return;
        setServers(settings.servers);
        setServersLoaded(true);
      } catch {
        if (!disposed) {
          setServers([]);
          setServersLoaded(true);
        }
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
          server.enabled
          && (
            options.filterByCapabilities === false
            || supportsMcpTransport(server, capabilities)
          ),
        )
        .sort((left, right) =>
          `${left.name}:${left.id}`.localeCompare(`${right.name}:${right.id}`),
        ),
    [capabilities, options.filterByCapabilities, servers],
  );

  useEffect(() => {
    if (!serversLoaded) return;
    if (options.filterByCapabilities !== false && capabilities == null) return;
    const availableIds = new Set(availableMcpServers.map((server) => server.id));
    setSelectedMcpIds((current) => current.filter((id) => availableIds.has(id)));
  }, [availableMcpServers, capabilities, options.filterByCapabilities, serversLoaded]);

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
    setSelectedMcpIds,
    toggleMcpSelection,
    clearSelectedMcps,
  };
}
