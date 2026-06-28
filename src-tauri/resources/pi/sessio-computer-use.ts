const TOOL_NAMES = [
  "computer_status",
  "computer_permissions",
  "computer_grant",
  "computer_list_apps",
  "computer_start",
  "computer_launch_app",
  "computer_raise_app",
  "computer_get_app_state",
  "computer_click",
  "computer_click_element",
  "computer_click_at",
  "computer_secondary_click",
  "computer_perform_secondary_action",
  "computer_double_click",
  "computer_drag",
  "computer_set_value",
  "computer_type_text",
  "computer_press_key",
  "computer_scroll",
  "computer_stop",
] as const;

type ToolName = (typeof TOOL_NAMES)[number];

type ComputerUseConfig = {
  mcpUrl: string;
  token: string;
  sessionId: string;
};

const NO_ARGS = {
  type: "object",
  properties: {},
  additionalProperties: false,
};

const APP_ARG = {
  type: "object",
  properties: {
    appId: { type: "string" },
    bundle: { type: "string" },
    windowId: { type: "string" },
  },
  anyOf: [{ required: ["appId"] }, { required: ["bundle"] }],
  additionalProperties: false,
};

const OPTIONAL_APP_ARG = {
  type: "object",
  properties: {
    appId: { type: "string" },
    bundle: { type: "string" },
    windowId: { type: "string" },
  },
  additionalProperties: false,
};

const SNAPSHOT_ARG = {
  type: "object",
  properties: { snapshotId: { type: "string" } },
  required: ["snapshotId"],
  additionalProperties: true,
};

const CLICK_ARG = {
  type: "object",
  properties: {
    snapshotId: { type: "string" },
    elementId: { type: "string" },
    ref: { type: "string" },
    x: { type: "number" },
    y: { type: "number" },
    coordSpace: { type: "string", enum: ["screenshot", "screen"] },
    coord_space: { type: "string", enum: ["screenshot", "screen"] },
  },
  required: ["snapshotId"],
  anyOf: [{ required: ["elementId"] }, { required: ["ref"] }, { required: ["x", "y"] }],
  additionalProperties: false,
};

const POINT_ARG = {
  type: "object",
  properties: {
    snapshotId: { type: "string" },
    x: { type: "number" },
    y: { type: "number" },
    coordSpace: { type: "string", enum: ["screenshot", "screen"] },
    coord_space: { type: "string", enum: ["screenshot", "screen"] },
  },
  required: ["snapshotId", "x", "y"],
  additionalProperties: false,
};

const DRAG_ARG = {
  type: "object",
  properties: {
    snapshotId: { type: "string" },
    fromX: { type: "number" },
    fromY: { type: "number" },
    toX: { type: "number" },
    toY: { type: "number" },
    coordSpace: { type: "string", enum: ["screenshot", "screen"] },
    coord_space: { type: "string", enum: ["screenshot", "screen"] },
  },
  required: ["snapshotId", "fromX", "fromY", "toX", "toY"],
  additionalProperties: false,
};

const SCROLL_ARG = {
  type: "object",
  properties: {
    snapshotId: { type: "string" },
    elementId: { type: "string" },
    ref: { type: "string" },
    direction: { type: "string", enum: ["up", "down", "left", "right"] },
    amount: { type: "integer" },
  },
  required: ["snapshotId", "direction"],
  additionalProperties: false,
};

const TOOL_PARAMETERS: Record<ToolName, unknown> = {
  computer_status: NO_ARGS,
  computer_permissions: NO_ARGS,
  computer_grant: {
    type: "object",
    properties: { permission: { type: "string", enum: ["screenshots", "accessibility"] } },
    required: ["permission"],
    additionalProperties: false,
  },
  computer_list_apps: {
    type: "object",
    properties: { days: { type: "integer", minimum: 1 } },
    additionalProperties: false,
  },
  computer_start: APP_ARG,
  computer_launch_app: APP_ARG,
  computer_raise_app: APP_ARG,
  computer_get_app_state: OPTIONAL_APP_ARG,
  computer_click: CLICK_ARG,
  computer_click_element: {
    ...SNAPSHOT_ARG,
    properties: { snapshotId: { type: "string" }, elementId: { type: "string" }, ref: { type: "string" } },
    additionalProperties: false,
  },
  computer_click_at: POINT_ARG,
  computer_secondary_click: CLICK_ARG,
  computer_perform_secondary_action: CLICK_ARG,
  computer_double_click: POINT_ARG,
  computer_drag: DRAG_ARG,
  computer_set_value: {
    ...SNAPSHOT_ARG,
    properties: {
      snapshotId: { type: "string" },
      elementId: { type: "string" },
      ref: { type: "string" },
      value: { type: "string" },
    },
    required: ["snapshotId", "value"],
    additionalProperties: false,
  },
  computer_type_text: {
    ...SNAPSHOT_ARG,
    properties: { snapshotId: { type: "string" }, text: { type: "string" } },
    required: ["snapshotId", "text"],
    additionalProperties: false,
  },
  computer_press_key: {
    ...SNAPSHOT_ARG,
    properties: { snapshotId: { type: "string" }, key: { type: "string" } },
    required: ["snapshotId", "key"],
    additionalProperties: false,
  },
  computer_scroll: SCROLL_ARG,
  computer_stop: NO_ARGS,
};

const TOOL_DESCRIPTIONS: Record<ToolName, string> = {
  computer_status: "Report whether Sessio computer use is available for this Pi session.",
  computer_permissions: "Report screen capture, accessibility, and input-control permission status.",
  computer_grant: "Open the relevant OS permission settings page when supported.",
  computer_list_apps: "List targetable apps, ordered by recent use when available.",
  computer_start: "Open a control lease on a target app or window.",
  computer_launch_app: "Launch a target app and open a lease after approval.",
  computer_raise_app: "Restore or foreground a hidden or Dock-minimized target app.",
  computer_get_app_state: "Capture screenshot metadata, AX elements, allowed actions, and a fresh snapshot id.",
  computer_click: "Click by AX element ref when available, or screenshot coordinates as fallback.",
  computer_click_element: "Click an accessibility element from the latest snapshot.",
  computer_click_at: "Fallback click by screenshot coordinate from the latest snapshot.",
  computer_secondary_click: "Secondary-click an AX element or coordinate from the latest snapshot.",
  computer_perform_secondary_action: "Skill-compatible alias for secondary-click.",
  computer_double_click: "Double-click a screenshot coordinate from the latest snapshot.",
  computer_drag: "Drag between two screenshot coordinates from the latest snapshot.",
  computer_set_value: "Set an accessibility element value directly.",
  computer_type_text: "Type text into the focused element of the latest snapshot.",
  computer_press_key: "Press a named key or chord against the latest snapshot.",
  computer_scroll: "Scroll the target or an AX element from the latest snapshot.",
  computer_stop: "Release the current computer-use lease.",
};

let config: ComputerUseConfig | null = null;
let nextRequestId = 1;

export default function sessioComputerUse(pi: any) {
  for (const name of TOOL_NAMES) {
    pi.registerTool({
      name,
      label: name,
      description: TOOL_DESCRIPTIONS[name],
      promptSnippet: `${name}: ${TOOL_DESCRIPTIONS[name]}`,
      promptGuidelines: [
        `${name} is a Sessio computer-use tool; call computer_status first if availability or approval is unclear.`,
        `${name} uses Sessio's AX-first desktop broker. Prefer elementId/ref over screenshot coordinates whenever available.`,
        "Do not write raw Swift/CoreGraphics/CGEvent, cliclick, AppleScript mouse, or other direct input scripts; they bypass Sessio approvals, snapshot mapping, and pointer overlay.",
      ],
      parameters: TOOL_PARAMETERS[name],
      async execute(_toolCallId: string, params: unknown, signal?: AbortSignal) {
        return await callComputerTool(name, params ?? {}, signal);
      },
    });
  }

  pi.on("session_start", async (_event: unknown, ctx: any) => {
    config = readConfig();
    setComputerToolsActive(pi, Boolean(config));
    if (config && ctx?.ui?.notify) {
      ctx.ui.notify("Sessio computer use is available for this Pi session.");
    }
  });

  pi.on("session_shutdown", async () => {
    if (config) {
      try {
        await callComputerTool("computer_stop", {}, undefined);
      } catch {
        // Best-effort lease release; Sessio also revokes the session token.
      }
    }
    setComputerToolsActive(pi, false);
    config = null;
  });
}

function readConfig(): ComputerUseConfig | null {
  const mcpUrl = process.env.SESSIO_COMPUTER_USE_MCP_URL?.trim();
  const token = process.env.SESSIO_COMPUTER_USE_TOKEN?.trim();
  const sessionId = process.env.SESSIO_COMPUTER_USE_SESSION_ID?.trim();
  if (!mcpUrl || !token || !sessionId) return null;
  return { mcpUrl, token, sessionId };
}

function setComputerToolsActive(pi: any, active: boolean) {
  if (typeof pi.getActiveTools !== "function" || typeof pi.setActiveTools !== "function") return;
  const current = pi.getActiveTools() as string[];
  const computer = new Set<string>(TOOL_NAMES);
  const next = active
    ? Array.from(new Set([...current, ...TOOL_NAMES]))
    : current.filter((tool) => !computer.has(tool));
  pi.setActiveTools(next);
}

async function callComputerTool(name: ToolName, params: unknown, signal?: AbortSignal) {
  if (!config) {
    throw new Error("Sessio computer use is not enabled for this Pi session.");
  }
  const response = await fetch(config.mcpUrl, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${config.token}`,
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: nextRequestId++,
      method: "tools/call",
      params: { name, arguments: params ?? {} },
    }),
    signal,
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`Sessio computer-use broker returned HTTP ${response.status}: ${truncate(text)}`);
  }
  const payload = JSON.parse(text);
  if (payload.error) {
    throw new Error(payload.error.message ?? JSON.stringify(payload.error));
  }
  const result = payload.result ?? {};
  const resultText = resultTextContent(result);
  if (result.isError) {
    throw new Error(truncate(resultText || "Sessio computer-use tool failed."));
  }
  return {
    content: [{ type: "text", text: truncate(resultText || JSON.stringify(result.structuredContent ?? result)) }],
    details: {
      sessionId: config.sessionId,
      tool: name,
      structuredContent: result.structuredContent ?? null,
    },
  };
}

function resultTextContent(result: any): string {
  const content = Array.isArray(result?.content) ? result.content : [];
  const parts = content
    .filter((item: any) => item?.type === "text" && typeof item.text === "string")
    .map((item: any) => item.text);
  return parts.join("\n");
}

function truncate(value: string): string {
  const max = 50_000;
  if (value.length <= max) return value;
  return `${value.slice(0, max)}\n...[truncated by sessio-computer-use]`;
}
