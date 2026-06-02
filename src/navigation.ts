export type ViewMode = "native" | "cross";
export type DetailMode = "chat" | "project";

import type { Agent, KanbanStatus, ProjectInfo, SessionHistorySnapshotGroup, SessionInfo, ThreadWorkSnapshot } from "./api";

export interface ProjectGroup {
  key: string;
  project: ProjectInfo;
  label: string;
  count: number;
  path: string;
  latest: number;
  sessions: SessionInfo[];
}

export interface PendingNewChatSession {
  sessioRuntimeSessionId: string;
  agent: Agent;
  forkedFromAgent?: Agent | null;
  forkedFromId?: string | null;
  projectPath: string;
  projectName: string;
  prompt: string;
  timestamp: number;
  kanbanItemId?: string;
  kanbanItemStatus?: KanbanStatus;
  historySnapshots?: SessionHistorySnapshotGroup[];
  workSnapshot?: {
    threadId: string;
    stageId: string | null;
    snapshot: ThreadWorkSnapshot;
  };
}
