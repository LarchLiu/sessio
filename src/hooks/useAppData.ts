import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  getIndexStatus,
  getMemoryBackendStatus,
  listProjects,
  listSessions,
  type AstraEvent,
  type IndexPhase,
  type MemoryBackendStatus,
  type ProjectInfo,
  type SessionInfo,
} from "../api";

export function useAppData({
  setError,
}: {
  setError: (error: string | null) => void;
}) {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [indexPhase, setIndexPhase] = useState<IndexPhase>("indexing");
  const [memoryBackendStatus, setMemoryBackendStatus] =
    useState<MemoryBackendStatus | null>(null);

  const refreshMemoryBackend = useCallback(() => {
    return getMemoryBackendStatus()
      .then(setMemoryBackendStatus)
      .catch((err) => {
        console.error("memory backend status check failed", err);
        setMemoryBackendStatus(null);
      });
  }, []);

  const refreshProjects = useCallback(() => {
    return listProjects()
      .then(setProjects)
      .catch((err) => {
        setError(String(err));
      });
  }, [setError]);

  const refreshSessions = useCallback(() => {
    return listSessions()
      .then(setSessions)
      .catch((err) => {
        setError(String(err));
      });
  }, [setError]);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    Promise.all([listSessions(), listProjects()])
      .then(([sessionRows, projectRows]) => {
        if (cancelled) return;
        setSessions(sessionRows);
        setProjects(projectRows);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [setError]);

  useEffect(() => {
    getIndexStatus()
      .then((status) => {
        setIndexPhase(status.phase);
        if (status.lastError) setError(status.lastError);
      })
      .catch(() => {});
    refreshMemoryBackend();

    const unlisten = listen("sessions_index_updated", () => {
      setIndexPhase("idle");
      void refreshSessions();
      void refreshProjects();
      refreshMemoryBackend();
    });
    const projectsUnlisten = listen("projects_updated", () => {
      void refreshProjects();
      void refreshSessions();
    });
    const astraUnlisten = listen<AstraEvent>("thread-astra-event", (event) => {
      if (event.payload.eventType !== "delegated") return;
      void refreshSessions();
      void refreshProjects();
    });
    const statusUnlisten = listen("sessions_index_status", (event) => {
      const payload = event.payload as {
        phase?: IndexPhase;
        lastError?: string | null;
      };
      if (payload.phase) setIndexPhase(payload.phase);
      if (payload.lastError !== undefined) setError(payload.lastError);
    });
    return () => {
      unlisten.then((f) => f()).catch(() => {});
      projectsUnlisten.then((f) => f()).catch(() => {});
      astraUnlisten.then((f) => f()).catch(() => {});
      statusUnlisten.then((f) => f()).catch(() => {});
    };
  }, [refreshMemoryBackend, refreshProjects, refreshSessions, setError]);

  return {
    sessions,
    setSessions,
    projects,
    setProjects,
    indexPhase,
    memoryBackendStatus,
    refreshSessions,
    refreshMemoryBackend,
  };
}
