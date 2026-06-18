import type { AcpToolCall } from "./runtimeChat";

export type ToolTitleParts = {
  main: string;
  detail?: string;
};

export interface AcpToolStripDisplay {
  iconName: string;
  name: string;
  description: string | null;
  tooltip: string | null;
  hidden: boolean;
}

export function acpToolStripDisplay(tool: AcpToolCall): AcpToolStripDisplay {
  const displayTool = canonicalizeAcpTool(tool);
  const planTool = isPlanTool(displayTool);
  const todoTool = isTodoTool(displayTool);
  const detail = acpToolDisplayDetail(displayTool);
  const title = planTool
    ? { main: "Update Plan" }
    : todoTool
      ? { main: todoToolTitle() }
      : detail.title;
  const description = compactToolDescription(title.detail ?? detail.command);
  const tooltip = [title.main, title.detail ?? detail.command]
    .filter((part) => typeof part === "string" && part.trim())
    .join(" ")
    .trim() || null;
  return {
    iconName: planTool ? "TaskUpdate" : todoTool ? "TodoWrite" : displayTool.title,
    name: title.main,
    description,
    tooltip,
    hidden: isHiddenHistoryTool(displayTool),
  };
}

function compactToolDescription(value: string | undefined): string | null {
  const line = value
    ?.split("\n")
    .map((part) => part.trim())
    .find(Boolean);
  return line || null;
}

function canonicalizeAcpTool(tool: AcpToolCall): AcpToolCall {
  const inlineTitle = splitInlineToolTitle(tool.title);
  const webActionDisplay = webActionToolDisplay(tool.rawInput);
  const display =
    webActionDisplay ??
    (inlineTitle
      ? {
          ...canonicalToolDisplay(inlineTitle.main, tool.rawInput ?? inlineTitle.detail, tool.kind),
          detail: inlineTitle.detail,
        }
      : canonicalToolDisplay(tool.title, tool.rawInput ?? "", tool.kind));
  const hidden = shouldHideTool(tool.title, tool.rawInput ?? "");
  if (display.main === tool.title && !display.detail && !hidden) return tool;
  const meta = parseObjectLike(tool.meta) ?? {};
  return {
    ...tool,
    title: toolDisplayName(display.main),
    meta: {
      ...meta,
      titleDetail: display.detail ?? pickString(meta.titleDetail) ?? undefined,
      hidden: hidden || meta.hidden === true,
    },
  };
}

function acpToolDisplayDetail(tool: AcpToolCall): { title: ToolTitleParts; command: string } {
  const input = parseObjectLike(tool.rawInput);
  if (!input) {
    const metaDetail = historyToolTitleDetail(tool);
    return {
      title: { main: tool.title, detail: metaDetail ?? undefined },
      command: acpToolInputText(tool),
    };
  }
  const title = acpToolTitle(tool, input);
  const command = pickToolInputDisplayText(input) ?? acpToolInputText(tool);
  return { title, command };
}

function acpToolTitle(tool: AcpToolCall, input: Record<string, unknown>): ToolTitleParts {
  const metaDetail = historyToolTitleDetail(tool);
  if (tool.title === "Read") {
    return metaDetail ? { main: tool.title, detail: metaDetail } : fileToolTitle(tool.title, input, true);
  }
  if (isFileMutationTool(tool.title)) {
    return metaDetail ? { main: tool.title, detail: metaDetail } : fileToolTitle(tool.title, input, false);
  }
  const description = pickString(input.description);
  return { main: tool.title, detail: metaDetail ?? description ?? undefined };
}

function fileToolTitle(title: string, input: Record<string, unknown>, includeRange: boolean): ToolTitleParts {
  const path = toolInputFilePath(input) ?? "";
  const basename = path ? basenameFromUri(path) ?? path : "";
  const range = includeRange ? readLineRange(input) : null;
  return { main: title, detail: [basename, range].filter(Boolean).join(" ") };
}

function isFileMutationTool(title: string): boolean {
  return title === "Write" || title === "Edit" || title === "MultiEdit" || title === "Delete" || title === "Move";
}

function historyToolTitleDetail(tool: AcpToolCall): string | null {
  const meta = parseObjectLike(tool.meta);
  return meta ? pickString(meta.titleDetail) : null;
}

function isHiddenHistoryTool(tool: AcpToolCall): boolean {
  if (isTodoTool(tool)) return false;
  const meta = parseObjectLike(tool.meta);
  return meta?.hidden === true;
}

function canonicalToolDisplay(name: string, body: unknown, kind?: string): ToolTitleParts {
  if (name === "apply_patch") {
    return { main: "Edit", detail: patchDisplayFile(patchInputText(body)) ?? undefined };
  }
  if (name === "write_stdin") {
    return { main: humanizeToolName(name), detail: writeStdinDisplayTarget(body) ?? undefined };
  }
  if (name === "web_search" || name === "WebSearch") return webSearchToolDisplay(body);
  if (isShellToolName(name)) {
    const cmd = toolInputCommand(body);
    if (!cmd) return { main: "Bash" };
    return commandToolDisplay(cmd);
  }
  if (isReadToolName(name)) {
    return { main: "Read", detail: fileToolDisplayDetail(body, true) ?? undefined };
  }
  if (isEditToolName(name)) {
    return { main: "Edit", detail: fileToolDisplayDetail(body, false) ?? undefined };
  }
  if (isGrepToolName(name)) {
    return { main: "Grep" };
  }
  if (name !== "exec_command" && name !== "shell_command") {
    const knownName = canonicalKnownToolName(name);
    if (knownName) return { main: knownName };
    const kindDisplay = canonicalToolKindDisplay(kind, body);
    if (kindDisplay) return kindDisplay;
    return { main: humanizeToolName(name) };
  }
  const cmd = toolInputCommand(body);
  if (!cmd) return { main: "Bash" };
  return commandToolDisplay(cmd);
}

function isShellToolName(name: string): boolean {
  return [
    "Shell",
    "Run Shell Command",
    "run_shell_command",
    "bash",
    "shell",
    "terminal",
  ].includes(name);
}

function isReadToolName(name: string): boolean {
  return ["ReadFile", "read_file"].includes(name);
}

function isEditToolName(name: string): boolean {
  return ["replace", "write_file", "WriteFile"].includes(name);
}

function isGrepToolName(name: string): boolean {
  return ["SearchText", "grep_search"].includes(name);
}

function splitInlineToolTitle(title: string): ToolTitleParts | null {
  const normalized = title.replace(/\s+/g, " ").trim();
  if (!normalized) return null;
  const match = normalized.match(/^(Read|List|LS|Edit|Write|MultiEdit|Search|Grep|Glob|Bash|WebFetch|WebSearch)\s+(.+)$/);
  if (!match) return null;
  const main = inlineToolMainName(match[1]);
  const detail = inlineToolDetail(match[2]);
  return detail ? { main, detail } : { main };
}

function inlineToolMainName(name: string): string {
  if (name === "LS") return "List";
  return name;
}

function inlineToolDetail(detail: string): string | undefined {
  const normalized = detail
    .split("|")
    .map((part) => part.trim())
    .filter((part) => part && !/^click(?:\s+to\s+copy)?$/i.test(part))
    .join(" ");
  if (!normalized) return undefined;
  return basenameFromUri(normalized) ?? normalized;
}

function toolInputCommand(body: unknown): string {
  const input = parseObjectLike(body);
  return input
    ? pickString(input.cmd) ?? pickString(input.command) ?? ""
    : typeof body === "string" ? body : "";
}

function fileToolDisplayDetail(body: unknown, includeRange: boolean): string | null {
  const input = parseObjectLike(body);
  if (!input) return null;
  const path = toolInputFilePath(input);
  const basename = path ? basenameFromUri(path) ?? path : "";
  const range = includeRange ? readLineRange(input) : null;
  const detail = [basename, range].filter(Boolean).join(" ");
  return detail || null;
}

function toolInputFilePath(input: Record<string, unknown>): string | null {
  return (
    pickString(input.file_path) ??
    pickString(input.filePath) ??
    pickString(input.path) ??
    pickString(input.absolute_path) ??
    pickString(input.absolutePath)
  );
}

function patchInputText(value: unknown): string {
  if (typeof value === "string") return value;
  const input = parseObjectLike(value);
  return input
    ? pickString(input.input) ?? pickString(input.patch) ?? pickString(input.text) ?? ""
    : "";
}

function shouldHideTool(name: string, body: unknown): boolean {
  const input = parseObjectLike(body);
  const action = input ? webActionRecord(input) : null;
  const actionType = action ? pickString(action.type) : null;
  if (actionType === "open_page") return !action || !firstWebActionUrl(action);
  if (name !== "web_search" && name !== "WebSearch") return false;
  return false;
}

function commandToolDisplay(command: string): ToolTitleParts {
  const first = firstShellCommandToken(command);
  if (["cat", "sed", "tail", "head", "nl"].includes(first)) {
    return { main: "Read", detail: commandDisplayFile(command) ?? undefined };
  }
  if (first === "rg" || first === "grep") return { main: "Grep" };
  if (first === "ls" || first === "find") return { main: "LS" };
  return { main: "Bash" };
}

function canonicalKnownToolName(name: string): string | null {
  const inlineTitle = splitInlineToolTitle(name);
  if (inlineTitle) return inlineTitle.main;
  switch (name) {
    case "Read":
    case "Write":
    case "Edit":
    case "MultiEdit":
    case "Delete":
    case "Move":
    case "Search":
    case "Grep":
    case "Glob":
    case "Bash":
    case "WebFetch":
    case "WebSearch":
    case "NotebookEdit":
    case "TodoWrite":
    case "ToolSearch":
    case "AskUserQuestion":
    case "TaskUpdate":
    case "Task":
    case "View Image":
      return name;
    case "LS":
    case "List":
      return "List";
    case "web_fetch":
    case "webfetch":
      return "WebFetch";
    case "web_search":
    case "websearch":
      return "WebSearch";
    case "read_file":
    case "ReadFile":
    case "read":
      return "Read";
    case "write":
      return "Write";
    case "replace":
    case "write_file":
    case "WriteFile":
    case "edit":
      return "Edit";
    case "multi_edit":
    case "MultiEditTool":
      return "MultiEdit";
    case "delete":
    case "remove":
    case "remove_file":
    case "delete_file":
    case "DeleteFile":
      return "Delete";
    case "move":
    case "move_file":
    case "MoveFile":
      return "Move";
    case "glob":
    case "find_files":
      return "Glob";
    case "list":
    case "ls":
    case "list_dir":
    case "list_directory":
    case "list_files":
      return "List";
    case "shell":
    case "bash":
    case "terminal":
      return "Bash";
    case "grep_search":
    case "SearchText":
      return "Grep";
    case "tool_search":
    case "toolsearch":
      return "ToolSearch";
    case "load_workspace_dependencies":
    case "install_workspace_dependencies":
      return "Read";
    case "automation_update":
      return "TaskUpdate";
    case "request_user_input":
      return "AskUserQuestion";
    case "read_thread_terminal":
      return "Bash";
    case "update_plan":
      return "TaskUpdate";
    case "task":
      return "Task";
    case "todo_write":
      return "TodoWrite";
    case "notebook_edit":
      return "NotebookEdit";
    case "view_image":
      return "View Image";
    default:
      return null;
  }
}

function canonicalToolKindDisplay(kind: string | undefined, body: unknown): ToolTitleParts | null {
  switch (kind) {
    case "read":
      return { main: "Read", detail: fileToolDisplayDetail(body, true) ?? undefined };
    case "edit":
      return { main: "Edit", detail: fileToolDisplayDetail(body, false) ?? undefined };
    case "delete":
      return { main: "Delete", detail: fileToolDisplayDetail(body, false) ?? undefined };
    case "move":
      return { main: "Move", detail: moveToolDisplayDetail(body) ?? fileToolDisplayDetail(body, false) ?? undefined };
    case "search":
      return { main: "Search" };
    case "execute": {
      const command = toolInputCommand(body);
      return command ? commandToolDisplay(command) : { main: "Bash" };
    }
    case "fetch":
      return webActionToolDisplay(body) ?? { main: "WebFetch" };
    case "think":
      return { main: "Think" };
    case "switch_mode":
      return { main: "Switch Mode" };
    default:
      return null;
  }
}

function moveToolDisplayDetail(body: unknown): string | null {
  const input = parseObjectLike(body);
  if (!input) return null;
  const source = toolInputFilePath(input);
  const target =
    pickString(input.new_path) ??
    pickString(input.newPath) ??
    pickString(input.destination) ??
    pickString(input.dest) ??
    pickString(input.target);
  const sourceLabel = source ? basenameFromUri(source) ?? source : "";
  const targetLabel = target ? basenameFromUri(target) ?? target : "";
  const detail = [sourceLabel, targetLabel].filter(Boolean).join(" -> ");
  return detail || null;
}

function humanizeToolName(name: string): string {
  const text = name.replace(/_/g, " ").trim();
  return text ? text.charAt(0).toUpperCase() + text.slice(1) : name;
}

function webSearchToolDisplay(body: unknown): ToolTitleParts {
  return webActionToolDisplay(body) ?? { main: "WebSearch" };
}

function webActionToolDisplay(body: unknown): ToolTitleParts | null {
  const input = parseObjectLike(body);
  const action = input ? webActionRecord(input) : null;
  const actionType = action ? pickString(action.type) : null;
  if (action && actionType === "open_page") {
    return { main: "WebFetch", detail: firstWebActionUrl(action) ?? undefined };
  }
  if (action && actionType === "search") {
    return { main: "WebSearch", detail: firstWebActionQuery(action) ?? undefined };
  }
  return null;
}

function webActionDisplayText(record: Record<string, unknown>): string | null {
  const action = webActionRecord(record);
  const actionType = pickString(action.type);
  if (actionType === "open_page") return firstWebActionUrl(action);
  if (actionType === "search") return webActionQueries(action).join("\n");
  return null;
}

function webActionRecord(record: Record<string, unknown>): Record<string, unknown> {
  const action = asRecord(record.action);
  return Object.keys(action).length > 0 ? action : record;
}

function firstWebActionUrl(action: Record<string, unknown>): string | null {
  return pickString(action.url) ?? webActionStringList(action, "urls")[0] ?? null;
}

function firstWebActionQuery(action: Record<string, unknown>): string | null {
  return webActionQueries(action)[0] ?? null;
}

function webActionQueries(action: Record<string, unknown>): string[] {
  return [
    ...webActionStringList(action, "queries"),
    ...webActionStringList(action, "query"),
  ];
}

function webActionStringList(record: Record<string, unknown>, key: string): string[] {
  const value = record[key];
  if (Array.isArray(value)) {
    return value
      .map((item) => pickString(item))
      .filter((item): item is string => Boolean(item));
  }
  const single = pickString(value);
  return single ? [single] : [];
}

function writeStdinDisplayTarget(body: unknown): string | null {
  const input = parseObjectLike(body);
  const sessionId = input ? sessionIdFromRecord(input) : null;
  return sessionId ? `session ${sessionId}` : null;
}

function sessionIdFromRecord(input: Record<string, unknown>): string | null {
  const sessionId = input.session_id ?? input.sessionId;
  return typeof sessionId === "number" || typeof sessionId === "string"
    ? String(sessionId)
    : null;
}

function commandDisplayFile(command: string): string | null {
  const tokens = shellTokens(command);
  for (let i = tokens.length - 1; i >= 0; i -= 1) {
    const token = stripShellTokenQuotes(tokens[i]);
    if (!token || token.startsWith("-") || /^[0-9,]+p$/.test(token)) continue;
    if (["cat", "sed", "tail", "head", "nl", "|"].includes(token)) continue;
    return basenameFromUri(token) ?? token;
  }
  return null;
}

function patchDisplayFile(patch: string): string | null {
  const match = patch.match(/^\*\*\* (?:Update|Add|Delete) File: (.+)$/m);
  const path = match?.[1]?.trim();
  return path ? basenameFromUri(path) ?? path : null;
}

function shellTokens(command: string): string[] {
  return command
    .trim()
    .split(/\s+/)
    .filter(Boolean);
}

function stripShellTokenQuotes(token: string): string {
  return token.replace(/^['"]|['"]$/g, "");
}

function firstShellCommandToken(command: string): string {
  const trimmed = command.trim();
  if (!trimmed) return "";
  const firstSegment = trimmed.split(/[|;&]/, 1)[0]?.trim() ?? trimmed;
  const tokens = firstSegment.split(/\s+/).filter(Boolean);
  const commandToken =
    tokens.find((token) => !/^[A-Za-z_][A-Za-z0-9_]*=/.test(token) && token !== "sudo") ?? "";
  return commandToken.split(/[/\\]/).pop() ?? commandToken;
}

function toolDisplayName(name: string): string {
  if (name === "web_search") return "Searching web";
  if (name === "LS") return "List";
  return name;
}

function acpToolInputText(tool: AcpToolCall): string {
  if (typeof tool.rawInput === "string") {
    return formatToolInputValue(tool.rawInput);
  }
  if (tool.rawInput !== null) return formatToolInputValue(tool.rawInput);
  return "";
}

function pickToolInputDisplayText(record: Record<string, unknown>): string | null {
  const webInput = webActionDisplayText(record);
  if (webInput) return webInput;
  const command = pickCommandText(record);
  if (command) return command;
  return formatToolInputValue(record);
}

function pickCommandText(record: Record<string, unknown>): string | null {
  const direct =
    pickString(record.command) ??
    pickString(record.cmd) ??
    pickString(record.input);
  if (direct) return formatToolInputValue(direct);
  for (const key of ["command", "cmd"]) {
    const value = record[key];
    if (Array.isArray(value)) {
      const parts = value
        .map((item) => pickString(item))
        .filter((item): item is string => Boolean(item));
      if (parts.length > 0) return parts.join(" ");
    }
  }
  return null;
}

function formatToolInputValue(value: unknown): string {
  const parsed = typeof value === "string" ? parseJsonInputString(value) : value;
  if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
    return formatObjectEntries(parsed as Record<string, unknown>);
  }
  if (parsed !== value) return JSON.stringify(parsed, null, 2);
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function parseJsonInputString(text: string): unknown {
  const trimmed = text.trim();
  if (!trimmed) return text;
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return text;
  }
}

function formatObjectEntries(record: Record<string, unknown>): string {
  return Object.entries(record)
    .map(([key, value]) => `${key}: ${formatObjectEntryValue(value)}`)
    .join("\n");
}

function formatObjectEntryValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === null || typeof value !== "object") return String(value);
  return JSON.stringify(value, null, 2);
}

function readLineRange(input: Record<string, unknown>): string | null {
  const offset = pickNumber(input.offset);
  const limit = pickNumber(input.limit);
  const startLine = pickNumber(input.start_line) ?? pickNumber(input.startLine);
  const endLine = pickNumber(input.end_line) ?? pickNumber(input.endLine);
  if (startLine !== null) {
    if (endLine !== null && endLine > startLine) return `(lines ${startLine}-${endLine})`;
    return `(line ${startLine})`;
  }
  if (offset === null) return null;
  if (limit === null || limit <= 1) return `(line ${offset})`;
  return `(lines ${offset}-${offset + limit - 1})`;
}

function pickString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function pickNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function parseObjectLike(value: unknown): Record<string, unknown> | null {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  if (typeof value !== "string") return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function basenameFromUri(uri: string): string | null {
  if (!uri) return null;
  const decoded = uri.startsWith("file://") ? uri.slice("file://".length) : uri;
  const name = decoded.split(/[/\\]/).filter(Boolean).pop();
  return name || null;
}

function isTodoTool(tool: AcpToolCall): boolean {
  const meta = tool.meta;
  if (meta && typeof meta === "object" && !Array.isArray(meta)) {
    const role = (meta as Record<string, unknown>).role;
    if (role === "todo") return true;
  }
  return tool.kind === "todo" || tool.title === "TodoWrite";
}

function isPlanTool(tool: AcpToolCall): boolean {
  return tool.title === "TaskUpdate" || tool.title === "update_plan";
}

function todoToolTitle(): string {
  return "Update Todos";
}
