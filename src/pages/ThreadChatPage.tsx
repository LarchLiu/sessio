import { useMemo } from "react";
import type { ThreadWorkSnapshotResult } from "../api";
import ThreadWorkSnapshotPanel, { useThreadWorkSnapshot } from "../components/ThreadWorkSnapshotPanel";
import type { ThreadPromptDisplayMeta } from "../threadPromptDisplay";
import ChatPage, { type ChatPageProps } from "./ChatPage";

export default function ThreadChatPage(props: ChatPageProps) {
  const { snapshot, sources } = useThreadWorkSnapshot(props.session.agent, props.session.id);
  const beforeMessages = useMemo(
    () =>
      snapshot ? (
        <ThreadWorkSnapshotPanel
          snapshot={snapshot}
          sources={sources?.sources ?? []}
        />
      ) : null,
    [snapshot, sources],
  );
  const threadPromptFallbacks = useMemo(
    () => threadPromptFallbacksFromSnapshot(snapshot),
    [snapshot],
  );

  return (
    <ChatPage
      {...props}
      beforeMessages={beforeMessages}
      showThreadPromptPlaceholders
      threadPromptFallbacks={threadPromptFallbacks}
    />
  );
}

function threadPromptFallbacksFromSnapshot(
  snapshot: ThreadWorkSnapshotResult | null,
): ThreadPromptDisplayMeta[] {
  if (!snapshot) return [];
  const work = asRecord(snapshot.snapshot);
  const task = asRecord(work.task);
  const assistantSnapshot = asRecord(work.assistantSnapshot);
  const agentSnapshot = asRecord(work.agentSnapshot);
  const agentInfo = asRecord(agentSnapshot.agentInfo);
  const taskTitle = pickString(task.title) ?? pickString(task.id);
  const assistantName = pickString(assistantSnapshot.name);
  const stageName =
    pickString(work.stageName) ??
    pickString(asRecord(work.stageSnapshot).name);
  const agentLabel =
    pickString(agentInfo.displayName) ??
    pickString(agentInfo.name);
  const attrs: Record<string, string> = {};
  if (taskTitle) attrs.task_title = taskTitle;
  if (assistantName) attrs.assistant_name = assistantName;
  if (stageName) attrs.stage_name = stageName;
  if (agentLabel) attrs.target_agent = agentLabel;
  return Object.keys(attrs).length > 0
    ? [{ kind: null, attrs }]
    : [];
}

function pickString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
