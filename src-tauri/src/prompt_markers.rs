#[derive(Debug, Clone, Copy)]
pub struct SessioPromptMarkers {
    pub attachment_marker: &'static str,
    pub codex_request_marker: &'static str,
    pub thread_prompt_start: &'static str,
    pub thread_prompt_end: &'static str,
    pub assistant_prompt_start: &'static str,
    pub assistant_prompt_end: &'static str,
    pub skills_prompt_start: &'static str,
    pub skills_prompt_end: &'static str,
    pub mcps_prompt_start: &'static str,
    pub mcps_prompt_end: &'static str,
    pub thread_prompt_kind_work_context: &'static str,
    pub selected_skills_prompt_kind: &'static str,
    pub selected_mcps_prompt_kind: &'static str,
    pub builtin_skill_prompt_kind: &'static str,
    pub skill_source_builtin: &'static str,
    pub skill_source_user: &'static str,
    pub builtin_skill_kind_computer_use: &'static str,
    pub builtin_skill_kind_create_thread: &'static str,
    pub builtin_skill_kind_work_state: &'static str,
    pub mcp_source_builtin: &'static str,
    pub mcp_source_custom: &'static str,
    pub builtin_mcp_kind_computer_use: &'static str,
}

static SESSIO_PROMPT_MARKERS: SessioPromptMarkers = SessioPromptMarkers {
    attachment_marker: "__sessio_attachment__:",
    codex_request_marker: "## My request for Codex:",
    thread_prompt_start: "<!-- sessio-thread-prompt:start",
    thread_prompt_end: "<!-- sessio-thread-prompt:end",
    assistant_prompt_start: "<!-- sessio-assistant-prompt:start",
    assistant_prompt_end: "<!-- sessio-assistant-prompt:end",
    skills_prompt_start: "<!-- sessio-skills:start",
    skills_prompt_end: "<!-- sessio-skills:end",
    mcps_prompt_start: "<!-- sessio-mcps:start",
    mcps_prompt_end: "<!-- sessio-mcps:end",
    thread_prompt_kind_work_context: "work_context",
    selected_skills_prompt_kind: "selected_skills",
    selected_mcps_prompt_kind: "selected_mcps",
    builtin_skill_prompt_kind: "builtin_skill",
    skill_source_builtin: "builtin",
    skill_source_user: "user",
    builtin_skill_kind_computer_use: "computerUse",
    builtin_skill_kind_create_thread: "createThread",
    builtin_skill_kind_work_state: "workState",
    mcp_source_builtin: "builtin",
    mcp_source_custom: "custom",
    builtin_mcp_kind_computer_use: "computerUse",
};

pub fn sessio_prompt_markers() -> &'static SessioPromptMarkers {
    &SESSIO_PROMPT_MARKERS
}
