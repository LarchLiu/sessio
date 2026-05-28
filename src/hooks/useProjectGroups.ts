import { useEffect, useMemo, useState } from "react";
import type { ProjectInfo, SessionInfo } from "../api";
import { isSubagentOnly, sessionIdentityKey } from "../appUtils";
import type { ProjectGroup } from "../navigation";
import { liveSessionUpdatedAt, type LiveRuntimeState } from "../runtimeChat";

export function useProjectGroups({
  availableSessions,
  projects,
  liveSessions,
  runtimeSessionAliases,
  selected,
}: {
  availableSessions: SessionInfo[];
  projects: ProjectInfo[];
  liveSessions: LiveRuntimeState["sessions"];
  runtimeSessionAliases: Record<string, string>;
  selected: SessionInfo | null;
}) {
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () => new Set(),
  );
  const [expandedProjectSessions, setExpandedProjectSessions] = useState<Set<string>>(
    () => new Set(),
  );

  const projectGroups = useMemo<ProjectGroup[]>(() => {
    const m = new Map<
      string,
      {
        project: ProjectInfo;
        label: string;
        count: number;
        path: string;
        latest: number;
        sessions: SessionInfo[];
      }
    >();
    for (const project of projects) {
      m.set(project.id, {
        project,
        label: project.name,
        count: 0,
        path: project.path,
        latest: project.updatedAt,
        sessions: [],
      });
    }
    const projectByPath = new Map(projects.map((project) => [project.path, project]));
    for (const s of availableSessions) {
      if (isSubagentOnly(s) || !s.projectPath) continue;
      const project = projectByPath.get(s.projectPath);
      if (!project) continue;
      const key = project.id;
      const ts = s.updatedAt ?? s.startedAt ?? 0;
      const e = m.get(key);
      if (e) {
        e.count += 1;
        if (ts > e.latest) e.latest = ts;
        e.sessions.push(s);
      }
    }
    return [...m.entries()]
      .map(([key, v]) => ({
        key,
        ...v,
        sessions: v.sessions.sort((a, b) => {
          const aRuntimeSessionId = runtimeSessionAliases[sessionIdentityKey(a)] ?? a.id;
          const bRuntimeSessionId = runtimeSessionAliases[sessionIdentityKey(b)] ?? b.id;
          const aLive = liveSessionUpdatedAt(liveSessions[aRuntimeSessionId]) ?? 0;
          const bLive = liveSessionUpdatedAt(liveSessions[bRuntimeSessionId]) ?? 0;
          return (
            Math.max(b.updatedAt ?? b.startedAt ?? 0, bLive) -
            Math.max(a.updatedAt ?? a.startedAt ?? 0, aLive)
          );
        }),
      }))
      .sort((a, b) => b.latest - a.latest || a.label.localeCompare(b.label));
  }, [availableSessions, liveSessions, projects, runtimeSessionAliases]);

  useEffect(() => {
    setExpandedProjects((prev) => {
      const keys = new Set(projectGroups.map((p) => p.key));
      let changed = false;
      const next = new Set<string>();
      for (const key of prev) {
        if (keys.has(key)) next.add(key);
        else changed = true;
      }
      if (next.size === 0 && projectGroups[0]) {
        next.add(projectGroups[0].key);
        changed = true;
      }
      if (
        selected &&
        selected.projectPath &&
        projects.some((project) => project.path === selected.projectPath)
      ) {
        const project = projects.find((item) => item.path === selected.projectPath);
        if (project && !next.has(project.id)) {
          next.add(project.id);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [projectGroups, projects, selected]);

  useEffect(() => {
    setExpandedProjectSessions((prev) => {
      const keys = new Set(projectGroups.map((p) => p.key));
      let changed = false;
      const next = new Set<string>();
      for (const key of prev) {
        if (keys.has(key)) next.add(key);
        else changed = true;
      }
      return changed ? next : prev;
    });
  }, [projectGroups]);

  return {
    projectGroups,
    expandedProjects,
    setExpandedProjects,
    expandedProjectSessions,
    setExpandedProjectSessions,
  };
}
