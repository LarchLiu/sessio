import { describe, expect, it } from "vitest";
import type { PendingNewChatSession } from "../src/navigation";
import { shouldAutoSelectPendingSession } from "../src/hooks/usePendingNewChats";

describe("shouldAutoSelectPendingSession", () => {
  it("keeps normal new chats auto-selecting", () => {
    expect(shouldAutoSelectPendingSession(pending())).toBe(true);
  });

  it("keeps suppressed thread multi-session chats on the current page", () => {
    expect(shouldAutoSelectPendingSession(pending({
      suppressAutoSelect: true,
      origin: "thread_multi_session",
    }))).toBe(false);
  });
});

function pending(patch: Partial<PendingNewChatSession> = {}): PendingNewChatSession {
  return {
    sessioRuntimeSessionId: "runtime-1",
    agent: "codex",
    projectPath: "/tmp/project",
    projectName: "Project",
    prompt: "hello",
    timestamp: 1,
    ...patch,
  };
}
