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
  computerUsePromptStart: string;
  computerUsePromptEnd: string;
  skillsPromptStart: string;
  skillsPromptEnd: string;
  workStateSkillPromptStart: string;
  workStateSkillPromptEnd: string;
  threadPromptKindWorkContext: string;
  computerUsePromptKind: string;
  selectedSkillsPromptKind: string;
  builtinSkillPromptKind: string;
  skillSourceBuiltin: SkillSource;
  skillSourceUser: SkillSource;
  builtinSkillKindComputerUse: BuiltinSkillKind;
  builtinSkillKindWorkState: BuiltinSkillKind;
  mcpSourceBuiltin: McpServerSource;
  mcpSourceCustom: McpServerSource;
  builtinMcpKindComputerUse: BuiltinMcpKind;
}

const SESSIO_PROMPT_MARKERS: Readonly<SessioPromptMarkers> = Object.freeze({
  attachmentMarker: "__sessio_attachment__:",
  codexRequestMarker: "## My request for Codex:",
  threadPromptStart: "<!-- sessio-thread-prompt:start",
  threadPromptEnd: "<!-- sessio-thread-prompt:end",
  assistantPromptStart: "<!-- sessio-assistant-prompt:start",
  assistantPromptEnd: "<!-- sessio-assistant-prompt:end",
  computerUsePromptStart: "<!-- sessio-computer-use:start",
  computerUsePromptEnd: "<!-- sessio-computer-use:end",
  skillsPromptStart: "<!-- sessio-skills:start",
  skillsPromptEnd: "<!-- sessio-skills:end",
  workStateSkillPromptStart: "<!-- sessio-work-state-skill:start",
  workStateSkillPromptEnd: "<!-- sessio-work-state-skill:end",
  threadPromptKindWorkContext: "work_context",
  computerUsePromptKind: "computer_use",
  selectedSkillsPromptKind: "selected_skills",
  builtinSkillPromptKind: "builtin_skill",
  skillSourceBuiltin: "builtin",
  skillSourceUser: "user",
  builtinSkillKindComputerUse: "computerUse",
  builtinSkillKindWorkState: "workState",
  mcpSourceBuiltin: "builtin",
  mcpSourceCustom: "custom",
  builtinMcpKindComputerUse: "computerUse",
});

export function getSessioPromptMarkers(): Readonly<SessioPromptMarkers> {
  return SESSIO_PROMPT_MARKERS;
}
