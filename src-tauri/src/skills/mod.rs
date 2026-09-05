use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{bail, Context, Result};
use notify::{recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use walkdir::WalkDir;

use crate::prompt_markers::sessio_prompt_markers;

pub mod computer_use;
pub mod create_sessio_app;
pub mod create_thread;
pub mod work_state;

const SKILL_MD_FILE_NAME: &str = "SKILL.md";
pub const SKILLS_UPDATED_EVENT: &str = "skills_updated";
pub const SELECTED_SKILL_IDS_OPTION: &str = "selectedSkillIds";
pub const SELECTED_SKILLS_OPTION: &str = "selectedSkills";

const BUILTIN_COMPUTER_USE_SKILL_ID: &str = "builtin:computer-use";
const BUILTIN_CREATE_SESSIO_APP_SKILL_ID: &str = "builtin:create-sessio-app";
const BUILTIN_CREATE_THREAD_SKILL_ID: &str = "builtin:create-thread";
const BUILTIN_WORK_STATE_SKILL_ID: &str = "builtin:sessio-work-state";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum SkillSource {
    Builtin,
    User,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinSkillKind {
    ComputerUse,
    CreateSessioApp,
    CreateThread,
    WorkState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_kind: Option<BuiltinSkillKind>,
    pub skill_md_path: String,
    pub root_dir: String,
    pub skill_dir_name: String,
    pub frontmatter: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SelectedSkillMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_kind: Option<BuiltinSkillKind>,
    pub skill_md_path: String,
    pub root_dir: String,
    pub skill_dir_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSkillRequest {
    pub source_path: String,
    #[serde(default)]
    pub directory_name: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Default)]
pub struct SkillsCache {
    inner: RwLock<Vec<SkillMetadata>>,
}

impl SkillsCache {
    pub fn get(&self) -> Vec<SkillMetadata> {
        self.inner
            .read()
            .map(|skills| skills.clone())
            .unwrap_or_default()
    }

    pub fn set(&self, skills: Vec<SkillMetadata>) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = skills;
        }
    }

    pub fn refresh_from_disk(&self) -> Result<Vec<SkillMetadata>> {
        let skills = load_skills()?;
        self.set(skills.clone());
        Ok(skills)
    }
}

#[derive(Clone)]
pub struct SkillsWatcher {
    _watcher: Arc<Mutex<RecommendedWatcher>>,
    _watched_roots: Vec<(PathBuf, RecursiveMode)>,
}

pub fn load_skills() -> Result<Vec<SkillMetadata>> {
    let mut skills = scan_builtin_skills();
    skills.extend(scan_user_skills()?);
    skills.sort_by(|left, right| {
        (
            left.source,
            left.builtin_kind,
            left.name.to_ascii_lowercase(),
            left.id.clone(),
        )
            .cmp(&(
                right.source,
                right.builtin_kind,
                right.name.to_ascii_lowercase(),
                right.id.clone(),
            ))
    });
    Ok(skills)
}

pub fn hydrate_selected_skills_option(
    options: &mut crate::agents::runtime::types::RuntimeMetadata,
    available_skills: &[SkillMetadata],
) {
    let Some(selected_ids) = selected_skill_ids_from_options(options) else {
        return;
    };
    let selected_skills = selected_ids
        .iter()
        .filter_map(|id| {
            available_skills
                .iter()
                .find(|skill| skill.id == *id)
                .map(selected_skill_metadata)
        })
        .collect::<Vec<_>>();
    options.insert(
        SELECTED_SKILL_IDS_OPTION.to_string(),
        serde_json::json!(selected_ids),
    );
    options.insert(
        SELECTED_SKILLS_OPTION.to_string(),
        serde_json::to_value(selected_skills).unwrap_or_else(|_| serde_json::json!([])),
    );
}

pub fn inject_selected_skills_prompt_block(
    text: &str,
    options: &crate::agents::runtime::types::RuntimeMetadata,
) -> String {
    let markers = sessio_prompt_markers();
    let Some(skills) = selected_skills_from_options(options) else {
        return text.to_string();
    };
    if skills.is_empty() {
        return text.to_string();
    }
    prepend_skills_prompt_block(
        text,
        markers.selected_skills_prompt_kind,
        "Selected Sessio skills are available for this conversation.\nUse the metadata below to decide which skill is relevant. When you need the full instructions, read the resolved `skillMdPath`. The canonical packaged layout is `rootDir/<skillDirName>/SKILL.md`.",
        &skills,
        None,
    )
}

pub fn builtin_skill_metadata(kind: BuiltinSkillKind) -> Option<SelectedSkillMetadata> {
    scan_builtin_skills()
        .into_iter()
        .find(|skill| skill.builtin_kind == Some(kind))
        .map(|skill| selected_skill_metadata(&skill))
}

pub fn inject_builtin_skill_prompt_block(
    text: &str,
    builtin_kind: BuiltinSkillKind,
    guidance: &str,
) -> String {
    let markers = sessio_prompt_markers();
    let Some(skill) = builtin_skill_metadata(builtin_kind) else {
        return text.to_string();
    };
    if text.contains(&format!("kind=\"{}\"", markers.builtin_skill_prompt_kind))
        && text.contains(&format!("id: `{}`", skill.id))
    {
        return text.to_string();
    }
    prepend_skills_prompt_block(
        text,
        markers.builtin_skill_prompt_kind,
        "A built-in Sessio skill is active for this conversation.\nUse the metadata below to locate the original `SKILL.md`. Built-in skills may live under a different `rootDir` than user-installed skills, so prefer the resolved `skillMdPath` when loading the full instructions.",
        &[skill],
        Some(guidance),
    )
}

pub fn install_skill(request: InstallSkillRequest) -> Result<SkillMetadata> {
    let source_path = crate::config::expand_path(&request.source_path)
        .with_context(|| format!("expand skill source path {}", request.source_path))?;
    if !source_path.exists() {
        bail!(
            "skill source path does not exist: {}",
            source_path.display()
        );
    }

    let user_root = ensure_user_skills_dir()?;
    let source_skill_md = resolve_install_source_skill_md(&source_path)?;
    let parsed = parse_skill_file(
        &source_skill_md,
        SkillSource::User,
        None,
        None,
        Some(&user_root),
    )?;

    let target_dir_name = request
        .directory_name
        .as_deref()
        .map(sanitize_skill_dir_name)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            if source_path.is_dir() {
                source_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(sanitize_skill_dir_name)
                    .filter(|value| !value.is_empty())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| sanitize_skill_dir_name(&parsed.name));
    let target_dir = user_root.join(target_dir_name);
    let source_canonical = fs::canonicalize(&source_path)
        .with_context(|| format!("canonicalize skill source {}", source_path.display()))?;
    let target_dir_canonical = canonicalize_target_path(&target_dir)?;

    if source_canonical == target_dir_canonical
        || source_canonical.starts_with(&target_dir_canonical)
        || (source_path.is_dir() && target_dir_canonical.starts_with(&source_canonical))
    {
        bail!(
            "skill source path overlaps install target: {}",
            target_dir.display()
        );
    }

    if target_dir.exists() {
        if !request.overwrite {
            bail!(
                "skill install target already exists: {}",
                target_dir.display()
            );
        }
        fs::remove_dir_all(&target_dir)
            .with_context(|| format!("remove existing skill dir {}", target_dir.display()))?;
    }

    if source_path.is_dir() {
        copy_skill_dir(&source_path, &target_dir)?;
    } else {
        fs::create_dir_all(&target_dir)
            .with_context(|| format!("create skill dir {}", target_dir.display()))?;
        fs::copy(&source_path, target_dir.join(SKILL_MD_FILE_NAME)).with_context(|| {
            format!(
                "copy skill file {} -> {}",
                source_path.display(),
                target_dir.join(SKILL_MD_FILE_NAME).display()
            )
        })?;
    }

    parse_skill_file(
        &target_dir.join(SKILL_MD_FILE_NAME),
        SkillSource::User,
        None,
        None,
        Some(&user_root),
    )
}

impl SkillsWatcher {
    pub fn new(app: AppHandle) -> Result<Self> {
        let watched_roots = watch_roots()?;
        let callback_app = app.clone();
        let mut watcher = recommended_watcher(move |result: notify::Result<Event>| match result {
            Ok(event) => handle_skill_event(&callback_app, event),
            Err(error) => log::warn!("[skills-watch] watcher error: {error}"),
        })
        .context("create skills watcher")?;

        for (path, mode) in &watched_roots {
            watcher
                .watch(path, *mode)
                .with_context(|| format!("watch skills root {}", path.display()))?;
        }

        Ok(Self {
            _watcher: Arc::new(Mutex::new(watcher)),
            _watched_roots: watched_roots,
        })
    }
}

fn handle_skill_event(app: &AppHandle, event: Event) {
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return;
    }
    if event.paths.iter().all(|path| is_platform_junk(path)) {
        return;
    }
    refresh_skills(app);
}

pub fn refresh_skills(app: &AppHandle) {
    let Some(cache) = app.try_state::<SkillsCache>() else {
        return;
    };
    match load_skills() {
        Ok(skills) => {
            cache.set(skills);
            let _ = app.emit(SKILLS_UPDATED_EVENT, ());
        }
        Err(error) => {
            log::warn!("[skills-watch] ignoring invalid skill update: {error:#}");
        }
    }
}

fn scan_builtin_skills() -> Vec<SkillMetadata> {
    let mut skills = Vec::new();
    let builtin_specs = [
        (
            BUILTIN_COMPUTER_USE_SKILL_ID,
            Some(BuiltinSkillKind::ComputerUse),
            computer_use::computer_use_skill_path(),
        ),
        (
            BUILTIN_CREATE_SESSIO_APP_SKILL_ID,
            Some(BuiltinSkillKind::CreateSessioApp),
            create_sessio_app::create_sessio_app_skill_path(),
        ),
        (
            BUILTIN_CREATE_THREAD_SKILL_ID,
            Some(BuiltinSkillKind::CreateThread),
            create_thread::create_thread_skill_path(),
        ),
        (
            BUILTIN_WORK_STATE_SKILL_ID,
            Some(BuiltinSkillKind::WorkState),
            work_state::work_state_skill_path(),
        ),
    ];

    for (id, builtin_kind, path) in builtin_specs {
        let Some(path) = path else {
            continue;
        };
        match parse_skill_file(&path, SkillSource::Builtin, builtin_kind, Some(id), None) {
            Ok(skill) => skills.push(skill),
            Err(error) => {
                log::warn!(
                    "[skills] failed to parse built-in skill {}: {error:#}",
                    path.display()
                );
            }
        }
    }

    skills
}

fn scan_user_skills() -> Result<Vec<SkillMetadata>> {
    let user_root = ensure_user_skills_dir()?;
    let mut skills = Vec::new();
    for entry in fs::read_dir(&user_root)
        .with_context(|| format!("read skills dir {}", user_root.display()))?
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!(
                    "[skills] failed to read {} entry: {error}",
                    user_root.display()
                );
                continue;
            }
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md_path = path.join(SKILL_MD_FILE_NAME);
        match parse_skill_file(
            &skill_md_path,
            SkillSource::User,
            None,
            None,
            Some(&user_root),
        ) {
            Ok(skill) => skills.push(skill),
            Err(error) => {
                log::warn!(
                    "[skills] failed to parse user skill {}: {error:#}",
                    skill_md_path.display()
                );
            }
        }
    }
    Ok(skills)
}

fn parse_skill_file(
    skill_md_path: &Path,
    source: SkillSource,
    builtin_kind: Option<BuiltinSkillKind>,
    explicit_id: Option<&str>,
    user_root: Option<&Path>,
) -> Result<SkillMetadata> {
    let contents = fs::read_to_string(skill_md_path)
        .with_context(|| format!("read skill file {}", skill_md_path.display()))?;
    let frontmatter = parse_frontmatter_map(&contents)
        .with_context(|| format!("parse skill frontmatter {}", skill_md_path.display()))?;

    let name = frontmatter
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("skill frontmatter `name` is required")?
        .to_string();
    let description = frontmatter
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("skill frontmatter `description` is required")?
        .to_string();

    let (root_dir, skill_dir_name) = skill_root_components(skill_md_path, user_root, &name)?;
    let id = explicit_id
        .map(str::to_string)
        .unwrap_or_else(|| derive_skill_id(source, &skill_dir_name));

    Ok(SkillMetadata {
        id,
        name,
        description,
        source,
        builtin_kind,
        skill_md_path: skill_md_path.to_string_lossy().to_string(),
        root_dir: root_dir.to_string_lossy().to_string(),
        skill_dir_name,
        frontmatter: serde_json::to_value(frontmatter).context("serialize skill frontmatter")?,
    })
}

fn parse_frontmatter_map(contents: &str) -> Result<BTreeMap<String, serde_yaml::Value>> {
    let mut lines = contents.lines();
    let Some(first_line) = lines.next() else {
        bail!("skill file is empty");
    };
    if first_line.trim() != "---" {
        bail!("skill file is missing YAML frontmatter");
    }

    let mut yaml = Vec::new();
    for line in lines {
        if line.trim() == "---" {
            let yaml = yaml.join("\n");
            return serde_yaml::from_str(&yaml).context("decode frontmatter YAML");
        }
        yaml.push(line);
    }

    bail!("skill frontmatter is not terminated");
}

fn selected_skill_ids_from_options(
    options: &crate::agents::runtime::types::RuntimeMetadata,
) -> Option<Vec<String>> {
    let value = options
        .get(SELECTED_SKILL_IDS_OPTION)
        .or_else(|| options.get("selected_skill_ids"))?;
    let ids = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .fold(Vec::new(), |mut out, value| {
            if !out.iter().any(|existing| existing == value) {
                out.push(value.to_string());
            }
            out
        });
    Some(ids)
}

fn selected_skills_from_options(
    options: &crate::agents::runtime::types::RuntimeMetadata,
) -> Option<Vec<SelectedSkillMetadata>> {
    let value = options
        .get(SELECTED_SKILLS_OPTION)
        .or_else(|| options.get("selected_skills"))?;
    serde_json::from_value::<Vec<SelectedSkillMetadata>>(value.clone()).ok()
}

fn selected_skill_metadata(skill: &SkillMetadata) -> SelectedSkillMetadata {
    SelectedSkillMetadata {
        id: skill.id.clone(),
        name: skill.name.clone(),
        description: skill.description.clone(),
        source: skill.source,
        builtin_kind: skill.builtin_kind,
        skill_md_path: skill.skill_md_path.clone(),
        root_dir: skill.root_dir.clone(),
        skill_dir_name: skill.skill_dir_name.clone(),
    }
}

fn prepend_skills_prompt_block(
    text: &str,
    kind: &str,
    intro: &str,
    skills: &[SelectedSkillMetadata],
    guidance: Option<&str>,
) -> String {
    if skills.is_empty() {
        return text.to_string();
    }

    let nonce = uuid::Uuid::new_v4().to_string();
    let markers = sessio_prompt_markers();
    let mut block = String::new();
    block.push_str(&format!(
        "{} nonce=\"{nonce}\" kind=\"{kind}\" -->\n\n",
        markers.skills_prompt_start
    ));
    block.push_str(intro.trim());
    block.push_str("\n\n");
    for skill in skills {
        block.push_str(&render_skill_metadata(skill));
    }
    if let Some(guidance) = guidance.map(str::trim).filter(|value| !value.is_empty()) {
        block.push('\n');
        block.push_str(guidance);
        block.push('\n');
    }
    block.push_str(&format!(
        "\n{} nonce=\"{nonce}\" -->",
        markers.skills_prompt_end
    ));
    if text.trim().is_empty() {
        block
    } else {
        format!("{block}\n\n{text}")
    }
}

fn render_skill_metadata(skill: &SelectedSkillMetadata) -> String {
    let mut lines = vec![format!("- `{}`: {}", skill.name, skill.description)];
    lines.push(format!("  id: `{}`", skill.id));
    lines.push(format!("  source: `{}`", skill_source_label(skill.source)));
    if let Some(builtin_kind) = skill.builtin_kind {
        lines.push(format!(
            "  builtinKind: `{}`",
            builtin_skill_kind_label(builtin_kind)
        ));
    }
    lines.push(format!("  rootDir: `{}`", skill.root_dir));
    lines.push(format!("  skillDirName: `{}`", skill.skill_dir_name));
    lines.push(format!("  skillMdPath: `{}`", skill.skill_md_path));
    format!("{}\n", lines.join("\n"))
}

fn skill_source_label(source: SkillSource) -> &'static str {
    let markers = sessio_prompt_markers();
    match source {
        SkillSource::Builtin => markers.skill_source_builtin,
        SkillSource::User => markers.skill_source_user,
    }
}

fn builtin_skill_kind_label(kind: BuiltinSkillKind) -> &'static str {
    let markers = sessio_prompt_markers();
    match kind {
        BuiltinSkillKind::ComputerUse => markers.builtin_skill_kind_computer_use,
        BuiltinSkillKind::CreateSessioApp => markers.builtin_skill_kind_create_sessio_app,
        BuiltinSkillKind::CreateThread => markers.builtin_skill_kind_create_thread,
        BuiltinSkillKind::WorkState => markers.builtin_skill_kind_work_state,
    }
}

fn derive_skill_id(source: SkillSource, skill_dir_name: &str) -> String {
    match source {
        SkillSource::Builtin => format!("builtin:{}", sanitize_skill_dir_name(skill_dir_name)),
        SkillSource::User => format!("user:{}", sanitize_skill_dir_name(skill_dir_name)),
    }
}

fn skill_root_components(
    skill_md_path: &Path,
    collection_root: Option<&Path>,
    skill_name: &str,
) -> Result<(PathBuf, String)> {
    if skill_md_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == SKILL_MD_FILE_NAME)
    {
        let skill_dir = skill_md_path
            .parent()
            .context("skill file has no parent directory")?;
        let skill_dir_name = skill_dir
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| sanitize_skill_dir_name(skill_name));
        let root_dir = collection_root
            .map(Path::to_path_buf)
            .or_else(|| skill_dir.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| skill_dir.to_path_buf());
        return Ok((root_dir, skill_dir_name));
    }

    let parent = skill_md_path
        .parent()
        .context("skill file has no parent directory")?;
    let skill_dir_name = skill_md_path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| sanitize_skill_dir_name(skill_name));
    Ok((parent.to_path_buf(), skill_dir_name))
}

fn resolve_install_source_skill_md(source_path: &Path) -> Result<PathBuf> {
    if source_path.is_dir() {
        let skill_md = source_path.join(SKILL_MD_FILE_NAME);
        if !skill_md.is_file() {
            bail!(
                "skill directory must contain {} at its root: {}",
                SKILL_MD_FILE_NAME,
                source_path.display()
            );
        }
        return Ok(skill_md);
    }
    if !source_path.is_file() {
        bail!(
            "skill source is not a file or directory: {}",
            source_path.display()
        );
    }
    Ok(source_path.to_path_buf())
}

fn canonicalize_target_path(target_dir: &Path) -> Result<PathBuf> {
    let parent = target_dir
        .parent()
        .context("skill install target has no parent directory")?;
    let parent_canonical = fs::canonicalize(parent)
        .with_context(|| format!("canonicalize target parent {}", parent.display()))?;
    Ok(parent_canonical.join(
        target_dir
            .file_name()
            .context("skill install target has no directory name")?,
    ))
}

fn copy_skill_dir(source_dir: &Path, target_dir: &Path) -> Result<()> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("create skill dir {}", target_dir.display()))?;
    for entry in WalkDir::new(source_dir).into_iter() {
        let entry =
            entry.with_context(|| format!("walk skill directory {}", source_dir.display()))?;
        let relative = entry
            .path()
            .strip_prefix(source_dir)
            .with_context(|| format!("strip skill prefix {}", source_dir.display()))?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let destination = target_dir.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&destination)
                .with_context(|| format!("create skill subdir {}", destination.display()))?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create skill parent dir {}", parent.display()))?;
        }
        fs::copy(entry.path(), &destination).with_context(|| {
            format!(
                "copy skill file {} -> {}",
                entry.path().display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn sanitize_skill_dir_name(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;
    for ch in value.chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if matches!(ch, '-' | '_' | ' ' | '/' | '\\' | '.') {
            Some('-')
        } else {
            None
        };

        let Some(ch) = normalized else {
            continue;
        };
        if ch == '-' {
            if out.is_empty() || last_was_dash {
                continue;
            }
            last_was_dash = true;
            out.push(ch);
            continue;
        }
        last_was_dash = false;
        out.push(ch);
    }
    out.trim_matches('-').to_string()
}

fn watch_roots() -> Result<Vec<(PathBuf, RecursiveMode)>> {
    let mut roots = Vec::new();
    let user_root = ensure_user_skills_dir()?;
    roots.push((user_root, RecursiveMode::Recursive));

    for skill_file in builtin_skill_files() {
        let Some(parent) = skill_file.parent() else {
            continue;
        };
        let parent = parent.to_path_buf();
        if roots.iter().any(|(path, _)| path == &parent) {
            continue;
        }
        roots.push((parent, RecursiveMode::NonRecursive));
    }

    Ok(roots)
}

fn builtin_skill_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in [
        computer_use::computer_use_skill_path(),
        create_sessio_app::create_sessio_app_skill_path(),
        create_thread::create_thread_skill_path(),
        work_state::work_state_skill_path(),
    ]
    .into_iter()
    .flatten()
    {
        if !out.iter().any(|existing| existing == &path) {
            out.push(path);
        }
    }
    out
}

fn ensure_user_skills_dir() -> Result<PathBuf> {
    let dir = crate::app_paths::skills_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("create skills dir {}", dir.display()))?;
    Ok(dir)
}

fn is_platform_junk(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        name,
        ".DS_Store" | "Thumbs.db" | "ehthumbs.db" | "desktop.ini" | ".directory"
    ) || name.starts_with("._")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sessio-skills-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn parses_frontmatter_with_extra_metadata() {
        let frontmatter = parse_frontmatter_map(
            r#"---
name: demo-skill
description: Demo description
metadata:
  short-description: Demo
allowed-tools:
  - Bash
---

# Demo
"#,
        )
        .expect("frontmatter");

        assert_eq!(
            frontmatter.get("name").and_then(|value| value.as_str()),
            Some("demo-skill")
        );
        assert_eq!(
            frontmatter
                .get("metadata")
                .and_then(|value| value.as_mapping())
                .and_then(|value| value.get(serde_yaml::Value::String("short-description".into())))
                .and_then(|value| value.as_str()),
            Some("Demo"),
        );
    }

    #[test]
    fn parses_user_skill_root_layout() {
        let root = temp_dir("scan");
        let nested = root.join("demo");
        fs::create_dir_all(&nested).expect("create skill dir");
        fs::write(
            nested.join(SKILL_MD_FILE_NAME),
            r#"---
name: recursive-demo
description: Recursive skill
---
"#,
        )
        .expect("write skill");

        let skill = parse_skill_file(
            &nested.join(SKILL_MD_FILE_NAME),
            SkillSource::User,
            None,
            None,
            Some(&root),
        )
        .expect("parse skill");

        assert_eq!(skill.id, "user:demo");
        assert_eq!(skill.root_dir, root.to_string_lossy().to_string());
        assert_eq!(skill.skill_dir_name, "demo");

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn sanitizes_skill_dir_names() {
        assert_eq!(sanitize_skill_dir_name("My Cool Skill"), "my-cool-skill");
        assert_eq!(sanitize_skill_dir_name("  weird///skill  "), "weird-skill");
        assert_eq!(sanitize_skill_dir_name("中文 skill"), "skill");
    }

    #[test]
    fn builtin_skill_scan_includes_create_thread() {
        let skills = scan_builtin_skills();
        let ids = skills
            .iter()
            .map(|skill| skill.id.as_str())
            .collect::<Vec<_>>();

        let create_thread = skills
            .iter()
            .find(|skill| skill.id == BUILTIN_CREATE_THREAD_SKILL_ID)
            .unwrap_or_else(|| panic!("missing create-thread skill; found ids: {ids:?}"));
        assert_eq!(
            create_thread.builtin_kind,
            Some(BuiltinSkillKind::CreateThread)
        );
        assert_eq!(create_thread.name, "create-thread");
        assert!(create_thread.skill_md_path.ends_with("SKILL.md"));
        assert!(skills
            .iter()
            .any(|skill| skill.id == BUILTIN_COMPUTER_USE_SKILL_ID));
        assert!(skills
            .iter()
            .any(|skill| skill.id == BUILTIN_WORK_STATE_SKILL_ID));
        let create_sessio_app = skills
            .iter()
            .find(|skill| skill.id == BUILTIN_CREATE_SESSIO_APP_SKILL_ID)
            .expect("missing create-sessio-app skill");
        assert_eq!(
            create_sessio_app.builtin_kind,
            Some(BuiltinSkillKind::CreateSessioApp)
        );
        assert_eq!(create_sessio_app.name, "create-sessio-app");
    }

    #[test]
    fn hydrates_selected_skills_from_ids() {
        let skills = vec![SkillMetadata {
            id: "user:demo".to_string(),
            name: "demo".to_string(),
            description: "Demo skill".to_string(),
            source: SkillSource::User,
            builtin_kind: None,
            skill_md_path: "/tmp/skills/demo/SKILL.md".to_string(),
            root_dir: "/tmp/skills".to_string(),
            skill_dir_name: "demo".to_string(),
            frontmatter: serde_json::json!({}),
        }];
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_SKILL_IDS_OPTION.to_string(),
            serde_json::json!(["user:demo"]),
        );

        hydrate_selected_skills_option(&mut options, &skills);

        assert_eq!(
            options.get(SELECTED_SKILLS_OPTION),
            Some(&serde_json::json!([{
                "id": "user:demo",
                "name": "demo",
                "description": "Demo skill",
                "source": "user",
                "skillMdPath": "/tmp/skills/demo/SKILL.md",
                "rootDir": "/tmp/skills",
                "skillDirName": "demo",
            }]))
        );
    }

    #[test]
    fn injects_selected_skills_prompt_block() {
        let markers = sessio_prompt_markers();
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_SKILLS_OPTION.to_string(),
            serde_json::json!([{
                "id": "user:demo",
                "name": "demo",
                "description": "Demo skill",
                "source": "user",
                "skillMdPath": "/tmp/skills/demo/SKILL.md",
                "rootDir": "/tmp/skills",
                "skillDirName": "demo",
            }]),
        );

        let output = inject_selected_skills_prompt_block("solve the task", &options);

        assert!(output.contains("Selected Sessio skills are available"));
        assert!(output.contains(&format!("kind=\"{}\"", markers.selected_skills_prompt_kind)));
        assert!(output.contains(&format!("source: `{}`", markers.skill_source_user)));
        assert!(output.contains("rootDir/<skillDirName>/SKILL.md"));
        assert!(output.contains("skillDirName: `demo`"));
        assert!(output.contains("skillMdPath: `/tmp/skills/demo/SKILL.md`"));
        assert!(output.ends_with("solve the task"));
    }

    #[test]
    fn injects_builtin_skill_prompt_block_with_shared_wrapper() {
        let markers = sessio_prompt_markers();
        let output = inject_builtin_skill_prompt_block(
            "continue the task",
            BuiltinSkillKind::ComputerUse,
            "Use `computer_get_app_state` first.",
        );

        assert!(output.contains(markers.skills_prompt_start));
        assert!(output.contains(&format!("kind=\"{}\"", markers.builtin_skill_prompt_kind)));
        assert!(output.contains("id: `builtin:computer-use`"));
        assert!(output.contains(&format!("source: `{}`", markers.skill_source_builtin)));
        assert!(output.contains(&format!(
            "builtinKind: `{}`",
            markers.builtin_skill_kind_computer_use
        )));
        assert!(output.contains("computer_get_app_state"));
        assert!(output.ends_with("continue the task"));
    }
}
