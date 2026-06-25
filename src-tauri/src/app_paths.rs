use std::path::PathBuf;

use anyhow::{Context, Result};

const PROD_APP_DIR: &str = ".sessio";
const DEV_APP_DIR: &str = ".sessio-dev";

pub fn is_dev_variant() -> bool {
    cfg!(debug_assertions)
        || matches!(
            std::env::var("SESSIO_APP_VARIANT").ok().as_deref(),
            Some("dev")
        )
}

pub fn app_dir_name() -> &'static str {
    if is_dev_variant() {
        DEV_APP_DIR
    } else {
        PROD_APP_DIR
    }
}

pub fn app_home() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok(home.join(app_dir_name()))
}

pub fn app_home_display() -> String {
    format!("~/{}", &app_dir_name()[1..])
}

pub fn db_data_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("db-data"))
}

pub fn db_path() -> Result<PathBuf> {
    Ok(db_data_dir()?.join("sessio-index.db"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_home()?.join("config.toml"))
}

pub fn im_bridge_config_path() -> Result<PathBuf> {
    Ok(app_home()?.join("im-bridge.yaml"))
}

pub fn removed_sessions_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("removed-sessions"))
}

pub fn projects_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("projects"))
}

pub fn cross_context_dir() -> Result<PathBuf> {
    Ok(projects_dir()?.join(".cross-context"))
}

pub fn canvas_root_dir() -> Result<PathBuf> {
    Ok(projects_dir()?.join(".canvas"))
}

pub fn session_canvas_dir(session_id: &str) -> Result<PathBuf> {
    Ok(canvas_root_dir()?.join(session_id))
}

pub fn paste_cache_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("paste-cache"))
}

pub fn memory_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("memory"))
}

pub fn legacy_qmd_memory_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("qmd-memory"))
}

pub fn im_bridge_state_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("im-bridge"))
}

pub fn agent_probe_workspace_dir(agent: &str) -> Result<PathBuf> {
    Ok(projects_dir()?
        .join(format!(".{agent}"))
        .join("tmp-agent-capabilities"))
}

pub fn astra_agent_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("astra-pi-agent"))
}

pub fn astra_sessions_dir() -> Result<PathBuf> {
    Ok(astra_agent_dir()?.join("sessions"))
}

pub fn astra_runtime_session_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("astra-sessions"))
}

pub fn pi_agent_home_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok(home.join(".pi").join("agent"))
}

pub fn pi_agent_sessions_dir() -> Result<PathBuf> {
    Ok(pi_agent_home_dir()?.join("sessions"))
}
