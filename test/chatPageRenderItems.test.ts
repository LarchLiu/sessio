import { describe, expect, it } from "vitest";
import { acpViewModelToRenderItems, renderItemKeys } from "../src/acpRenderItems";
import type { AcpViewModel, AcpPermissionRequest } from "../src/runtimeChat";

describe("acpViewModelToRenderItems", () => {
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
