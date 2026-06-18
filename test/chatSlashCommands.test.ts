import { describe, expect, it } from "vitest";
import {
  filterChatSlashCommands,
  formatChatSlashCommandText,
  parseChatSlashCommandTrigger,
  parseRuntimeSessionAvailableCommands,
} from "../src/chatSlashCommands";

describe("chatSlashCommands", () => {
  it("parses slash triggers only while editing the command token", () => {
    expect(parseChatSlashCommandTrigger("/")).toEqual({
      query: "",
      raw: "/",
    });
    expect(parseChatSlashCommandTrigger("/pla")).toEqual({
      query: "pla",
      raw: "/pla",
    });
    expect(parseChatSlashCommandTrigger("/plan now")).toBeNull();
    expect(parseChatSlashCommandTrigger("hello /plan")).toBeNull();
  });

  it("filters commands by prefix and preserves command order", () => {
    expect(
      filterChatSlashCommands(
        [
          { name: "plan", description: "Plan", input: null, meta: null },
          { name: "permissions", description: "Permissions", input: null, meta: null },
          { name: "search", description: "Search", input: null, meta: null },
        ],
        "p",
      ).map((command) => command.name),
    ).toEqual(["plan", "permissions"]);
  });

  it("formats selected command text based on input kind", () => {
    expect(
      formatChatSlashCommandText({
        name: "plan",
        input: { kind: "unstructured", hint: "Goal", meta: null, raw: {} },
      }),
    ).toBe("/plan ");
    expect(
      formatChatSlashCommandText({
        name: "status",
        input: null,
      }),
    ).toBe("/status");
  });

  it("parses persisted runtime commands JSON", () => {
    expect(
      parseRuntimeSessionAvailableCommands({
        availableCommandsJson: JSON.stringify([
          {
            name: "plan",
            description: "Plan the work",
            input: {
              kind: "unstructured",
              hint: "What should I plan?",
            },
          },
          {
            name: "status",
            description: "Show status",
          },
          {
            description: "Missing name",
          },
        ]),
      }),
    ).toEqual([
      {
        name: "plan",
        description: "Plan the work",
        input: {
          kind: "unstructured",
          hint: "What should I plan?",
          meta: null,
          raw: {
            kind: "unstructured",
            hint: "What should I plan?",
          },
        },
        meta: null,
      },
      {
        name: "status",
        description: "Show status",
        input: null,
        meta: null,
      },
    ]);
    expect(
      parseRuntimeSessionAvailableCommands({
        availableCommandsJson: "{not json}",
      }),
    ).toEqual([]);
  });
});
