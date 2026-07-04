import type {
  BuiltinMcpKind,
  BuiltinSkillKind,
  McpServerSource,
  SkillSource,
} from "./api";

export interface SessioPromptMarkers {
  attachmentMarker: string;
  codexRequestMarker: string;
  threadPromptStart: string;
  threadPromptEnd: string;
  assistantPromptStart: string;
  assistantPromptEnd: string;
  skillsPromptStart: string;
  skillsPromptEnd: string;
  mcpsPromptStart: string;
  mcpsPromptEnd: string;
  threadPromptKindWorkContext: string;
  selectedSkillsPromptKind: string;
  selectedMcpsPromptKind: string;
  builtinSkillPromptKind: string;
  skillSourceBuiltin: SkillSource;
  skillSourceUser: SkillSource;
  builtinSkillKindComputerUse: BuiltinSkillKind;
  builtinSkillKindCreateThread: BuiltinSkillKind;
  builtinSkillKindWorkState: BuiltinSkillKind;
  mcpSourceBuiltin: McpServerSource;
  mcpSourceCustom: McpServerSource;
  builtinMcpKindComputerUse: BuiltinMcpKind;
  builtinMcpIdComputerUse: string;
}

const SESSIO_PROMPT_MARKERS: Readonly<SessioPromptMarkers> = Object.freeze({
  attachmentMarker: "__sessio_attachment__:",
  codexRequestMarker: "## My request for Codex:",
  threadPromptStart: "<!-- sessio-thread-prompt:start",
  threadPromptEnd: "<!-- sessio-thread-prompt:end",
  assistantPromptStart: "<!-- sessio-assistant-prompt:start",
  assistantPromptEnd: "<!-- sessio-assistant-prompt:end",
  skillsPromptStart: "<!-- sessio-skills:start",
  skillsPromptEnd: "<!-- sessio-skills:end",
  mcpsPromptStart: "<!-- sessio-mcps:start",
  mcpsPromptEnd: "<!-- sessio-mcps:end",
  threadPromptKindWorkContext: "work_context",
  selectedSkillsPromptKind: "selected_skills",
  selectedMcpsPromptKind: "selected_mcps",
  builtinSkillPromptKind: "builtin_skill",
  skillSourceBuiltin: "builtin",
  skillSourceUser: "user",
  builtinSkillKindComputerUse: "computerUse",
  builtinSkillKindCreateThread: "createThread",
  builtinSkillKindWorkState: "workState",
  mcpSourceBuiltin: "builtin",
  mcpSourceCustom: "custom",
  builtinMcpKindComputerUse: "computerUse",
  builtinMcpIdComputerUse: "builtin:computer-use",
});

export function getSessioPromptMarkers(): Readonly<SessioPromptMarkers> {
  return SESSIO_PROMPT_MARKERS;
}
