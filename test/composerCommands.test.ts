import { describe, expect, it } from "vitest";
import {
  filterComposerSlashCommands,
  parseComposerCommandTrigger,
} from "../src/composerCommands";

describe("composerCommands", () => {
  it("parses slash and assistant triggers from the composer prefix", () => {
    expect(parseComposerCommandTrigger("/", ["slash", "assistant"])).toEqual({
      kind: "slash",
      query: "",
      rest: "",
      raw: "/",
    });
    expect(parseComposerCommandTrigger("@agent follow up", ["slash", "assistant"])).toEqual({
      kind: "assistant",
      query: "agent",
      rest: "follow up",
      raw: "@agent follow up",
    });
  });

  it("filters slash commands by prefix without depending on send availability", () => {
    const filtered = filterComposerSlashCommands([
      { name: "plan", description: "Plan", input: null, meta: null },
      { name: "permissions", description: "Permissions", input: null, meta: null },
      { name: "search", description: "Search", input: null, meta: null },
    ], "p");

    expect(filtered.map((command) => command.name)).toEqual(["plan", "permissions"]);
  });
});
