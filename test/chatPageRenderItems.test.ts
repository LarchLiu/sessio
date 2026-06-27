import { describe, expect, it } from "vitest";
import {
  acpViewModelToRenderItems,
  laterTurnEventFlagsForRenderItems,
  liveOrLatestTurnFileEdits,
  parseFileEditSummary,
  renderItemKeys,
} from "../src/acpRenderItems";
import type { AcpViewModel, AcpPermissionRequest } from "../src/runtimeChat";

describe("acpViewModelToRenderItems", () => {
  it("keeps all same-file file_edit details when summaries are merged", () => {
    const summary = parseFileEditSummary({
      source: "session",
      edits: [
        {
          path: "src/example.ts",
          additions: 1,
          deletions: 1,
          kind: "modified",
          patch: "@@ -1 +1 @@\n-old\n+new",
          detail: "first edit",
          oldContent: "old",
          newContent: "new",
        },
        {
          path: "src/example.ts",
          additions: 2,
          deletions: 0,
          kind: "modified",
          patch: "@@ -3,0 +4,2 @@\n+next\n+lines",
          detail: "second edit",
          oldContent: "new",
          newContent: "new\nnext\nlines",
        },
      ],
    });

    expect(summary).not.toBeNull();
    const edit = summary?.edits?.[0];
    expect(summary).toMatchObject({
      files: 1,
      additions: 3,
      deletions: 1,
    });
    expect(edit).toMatchObject({
      path: "src/example.ts",
      additions: 3,
      deletions: 1,
      patch: "@@ -1 +1 @@\n-old\n+new",
      detail: "first edit",
      oldContent: "old",
      newContent: "new",
    });
    expect(edit?.patches).toEqual([
      "@@ -1 +1 @@\n-old\n+new",
      "@@ -3,0 +4,2 @@\n+next\n+lines",
    ]);
    expect(edit?.details).toEqual(["first edit", "second edit"]);
    expect(edit?.contentDiffs).toEqual([
      { oldContent: "old", newContent: "new" },
      { oldContent: "new", newContent: "new\nnext\nlines" },
    ]);
  });

  it("renders unresolved option permissions after edited files within the same turn", () => {
    const permission: AcpPermissionRequest = {
      requestId: "perm-1",
      toolCall: null,
      toolName: "bash",
      input: null,
      options: [
        {
          optionId: "allow",
          name: "Allow",
          kind: "allow_once",
          meta: null,
        },
      ],
      selectedOptionId: null,
      cancelled: false,
      raw: {},
    };
    const viewModel: AcpViewModel = {
      turns: [
        {
          turnId: "turn-1",
          status: "streaming",
          blocks: [
            {
              kind: "assistant",
              blocks: [{ type: "text", text: "Working" }],
              raw: {},
              timestamp: 1,
            },
            {
              kind: "permission",
              requestId: permission.requestId,
              timestamp: 2,
            },
            {
              kind: "sessionUpdate",
              updateType: "file_edit",
              data: {
                source: "session",
                files: 1,
                additions: 3,
                deletions: 1,
                edits: [
                  {
                    path: "src/example.ts",
                    additions: 3,
                    deletions: 1,
                    kind: "modified",
                  },
                ],
              },
              timestamp: 3,
            },
          ],
          tools: [],
          permissions: [permission],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 1,
          updatedAt: 3,
        },
      ],
      sessionState: {
        plan: null,
        availableCommands: [],
        currentModeId: null,
        configOptions: [],
        sessionInfo: null,
      },
      protocolMessages: [],
      ended: false,
    };

    const items = acpViewModelToRenderItems(viewModel, new Set(), "");

    expect(renderItemKeys(items)).toEqual([
      "acp:turn-1:block:0",
      "acp:turn-1:block:1",
      "acp:turn-1:permission:perm-1",
    ]);
    expect(items[1]).toMatchObject({
      kind: "block",
      block: {
        kind: "sessionUpdate",
        updateType: "file_edit",
      },
    });
    expect(items[2]).toMatchObject({
      kind: "permission",
      permission: { requestId: "perm-1" },
    });
  });

  it("keeps resolved permissions in their original block position", () => {
    const permission: AcpPermissionRequest = {
      requestId: "perm-1",
      toolCall: null,
      toolName: "bash",
      input: null,
      options: [
        {
          optionId: "allow",
          name: "Allow",
          kind: "allow_once",
          meta: null,
        },
      ],
      selectedOptionId: "allow",
      cancelled: false,
      raw: {},
    };
    const viewModel: AcpViewModel = {
      turns: [
        {
          turnId: "turn-1",
          status: "streaming",
          blocks: [
            {
              kind: "assistant",
              blocks: [{ type: "text", text: "Working" }],
              raw: {},
              timestamp: 1,
            },
            {
              kind: "permission",
              requestId: permission.requestId,
              timestamp: 2,
            },
            {
              kind: "sessionUpdate",
              updateType: "file_edit",
              data: {
                source: "session",
                files: 1,
                additions: 3,
                deletions: 1,
                edits: [
                  {
                    path: "src/example.ts",
                    additions: 3,
                    deletions: 1,
                    kind: "modified",
                  },
                ],
              },
              timestamp: 3,
            },
          ],
          tools: [],
          permissions: [permission],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 1,
          updatedAt: 3,
        },
      ],
      sessionState: {
        plan: null,
        availableCommands: [],
        currentModeId: null,
        configOptions: [],
        sessionInfo: null,
      },
      protocolMessages: [],
      ended: false,
    };

    const items = acpViewModelToRenderItems(viewModel, new Set(), "");

    expect(renderItemKeys(items)).toEqual([
      "acp:turn-1:block:0",
      "acp:turn-1:permission:perm-1",
      "acp:turn-1:block:1",
    ]);
    expect(items[1]).toMatchObject({
      kind: "permission",
      permission: { requestId: "perm-1", selectedOptionId: "allow" },
    });
    expect(items[2]).toMatchObject({
      kind: "block",
      block: {
        kind: "sessionUpdate",
        updateType: "file_edit",
      },
    });
  });

  it("prefers the live turn for edited files when that turn has file changes", () => {
    const viewModel: AcpViewModel = {
      turns: [
        {
          turnId: "turn-1",
          status: "completed",
          blocks: [
            {
              kind: "sessionUpdate",
              updateType: "file_edit",
              data: {
                source: "session",
                edits: [{ path: "src/old.ts", additions: 1, deletions: 0 }],
              },
              timestamp: 1,
            },
          ],
          tools: [],
          permissions: [],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 1,
          updatedAt: 1,
        },
        {
          turnId: "turn-2",
          status: "streaming",
          blocks: [
            {
              kind: "sessionUpdate",
              updateType: "file_edit",
              data: {
                source: "session",
                edits: [{ path: "src/live.ts", additions: 2, deletions: 1 }],
              },
              timestamp: 2,
            },
          ],
          tools: [],
          permissions: [],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 2,
          updatedAt: 2,
        },
      ],
      sessionState: {
        plan: null,
        availableCommands: [],
        currentModeId: null,
        configOptions: [],
        sessionInfo: null,
      },
      protocolMessages: [],
      ended: false,
    };

    const summary = liveOrLatestTurnFileEdits(viewModel, new Set(["turn-2"]));

    expect(summary).toMatchObject({
      source: "live",
      turnId: "turn-2",
      additions: 2,
      deletions: 1,
    });
    expect(summary.edits.map((edit) => edit.path)).toEqual(["src/live.ts"]);
  });

  it("falls back to the last turn only and hides edited files when that turn has no file changes", () => {
    const viewModel: AcpViewModel = {
      turns: [
        {
          turnId: "turn-1",
          status: "completed",
          blocks: [
            {
              kind: "sessionUpdate",
              updateType: "file_edit",
              data: {
                source: "session",
                edits: [{ path: "src/older.ts", additions: 3, deletions: 0 }],
              },
              timestamp: 1,
            },
          ],
          tools: [],
          permissions: [],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 1,
          updatedAt: 1,
        },
        {
          turnId: "turn-2",
          status: "completed",
          blocks: [
            {
              kind: "assistant",
              blocks: [{ type: "text", text: "No edits here" }],
              raw: {},
              timestamp: 2,
            },
          ],
          tools: [],
          permissions: [],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 2,
          updatedAt: 2,
        },
      ],
      sessionState: {
        plan: null,
        availableCommands: [],
        currentModeId: null,
        configOptions: [],
        sessionInfo: null,
      },
      protocolMessages: [],
      ended: false,
    };

    const summary = liveOrLatestTurnFileEdits(viewModel, new Set());

    expect(summary).toMatchObject({
      source: "none",
      turnId: "turn-2",
      additions: 0,
      deletions: 0,
    });
    expect(summary.edits).toEqual([]);
  });

  it("marks earlier live message items as having later turn events within the same turn", () => {
    const viewModel: AcpViewModel = {
      turns: [
        {
          turnId: "turn-1",
          status: "streaming",
          blocks: [
            {
              kind: "thought",
              blocks: [{ type: "text", text: "Thinking..." }],
              raw: {},
              timestamp: 1,
            },
            {
              kind: "assistant",
              blocks: [{ type: "text", text: "Answer" }],
              raw: {},
              timestamp: 2,
            },
          ],
          tools: [],
          permissions: [],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 1,
          updatedAt: 2,
        },
      ],
      sessionState: {
        plan: null,
        availableCommands: [],
        currentModeId: null,
        configOptions: [],
        sessionInfo: null,
      },
      protocolMessages: [],
      ended: false,
    };

    const items = acpViewModelToRenderItems(viewModel, new Set(["turn-1"]), "");

    expect(items.map((item) => item.kind)).toEqual(["turnStatus", "block", "block"]);
    expect(laterTurnEventFlagsForRenderItems(items)).toEqual([true, true, false]);
  });

  it("uses full render context so preview subsets still cancel older typewriter items", () => {
    const viewModel: AcpViewModel = {
      turns: [
        {
          turnId: "turn-1",
          status: "streaming",
          blocks: [
            {
              kind: "thought",
              blocks: [{ type: "text", text: "Thinking..." }],
              raw: {},
              timestamp: 1,
            },
            {
              kind: "assistant",
              blocks: [{ type: "text", text: "Answer" }],
              raw: {},
              timestamp: 2,
            },
          ],
          tools: [],
          permissions: [],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 1,
          updatedAt: 2,
        },
      ],
      sessionState: {
        plan: null,
        availableCommands: [],
        currentModeId: null,
        configOptions: [],
        sessionInfo: null,
      },
      protocolMessages: [],
      ended: false,
    };

    const items = acpViewModelToRenderItems(viewModel, new Set(["turn-1"]), "");
    const subset = [items[1], items[2]];

    expect(laterTurnEventFlagsForRenderItems(subset)).toEqual([true, false]);
    expect(laterTurnEventFlagsForRenderItems(subset, items)).toEqual([true, false]);
  });

  it("does not treat working indicators as later turn events for the final live message", () => {
    const viewModel: AcpViewModel = {
      turns: [
        {
          turnId: "turn-1",
          status: "streaming",
          blocks: [
            {
              kind: "assistant",
              blocks: [{ type: "text", text: "Answer" }],
              raw: {},
              timestamp: 1,
            },
          ],
          tools: [],
          permissions: [],
          protocolMessages: [],
          stopReason: null,
          error: null,
          startedAt: 1,
          updatedAt: 1,
        },
      ],
      sessionState: {
        plan: null,
        availableCommands: [],
        currentModeId: null,
        configOptions: [],
        sessionInfo: null,
      },
      protocolMessages: [],
      ended: false,
    };

    const items = acpViewModelToRenderItems(viewModel, new Set(["turn-1"]), "turn-1");

    expect(items.map((item) => item.kind)).toEqual(["turnStatus", "block", "workingIndicator"]);
    expect(laterTurnEventFlagsForRenderItems(items)).toEqual([true, false, false]);
  });
});
