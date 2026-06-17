import { describe, expect, it } from "vitest";
import { acpViewModelToRenderItems, parseFileEditSummary, renderItemKeys } from "../src/acpRenderItems";
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
});
