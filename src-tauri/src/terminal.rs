use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{Child, ChildKiller, CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const DEFAULT_TERMINAL_TITLE: &str = "Terminal";
const TERMINAL_EVENT_NAME: &str = "terminal-event";
const TERMINAL_OUTPUT_CHUNK_LIMIT: usize = 16 * 1024;
const TERMINAL_SCROLLBACK_LIMIT: usize = 256 * 1024;
const DEFAULT_SHELL_UNIX: &str = "/bin/zsh";
const DEFAULT_SHELL_WINDOWS: &str = "cmd.exe";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub shell: String,
    pub cols: u16,
    pub rows: u16,
    pub output: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTerminalRequest {
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeTerminalRequest {
    pub terminal_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteTerminalInputRequest {
    pub terminal_id: String,
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTerminalRequest {
    pub terminal_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalEventEnvelope {
    pub terminal_id: String,
    pub event: TerminalEvent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TerminalEvent {
    Created {
        session: TerminalSessionSummary,
    },
    Output {
        data: String,
    },
    Resized {
        cols: u16,
        rows: u16,
    },
    Closed {
        exit_code: Option<i32>,
    },
    Removed,
}

#[derive(Clone)]
pub struct TerminalService {
    app: AppHandle,
    sessions: Arc<Mutex<HashMap<String, TerminalSessionHandle>>>,
    next_index: Arc<AtomicUsize>,
}

struct TerminalSessionHandle {
    state: Arc<Mutex<TerminalSessionState>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

struct TerminalSessionState {
    summary: TerminalSessionSummary,
}

impl TerminalService {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_index: Arc::new(AtomicUsize::new(1)),
        }
    }

    pub fn list_sessions(&self) -> Result<Vec<TerminalSessionSummary>, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "terminal sessions lock poisoned".to_string())?;
        let mut items = sessions
            .values()
            .map(|session| {
                session
                    .state
                    .lock()
                    .map_err(|_| "terminal session state lock poisoned".to_string())
                    .map(|state| state.summary.clone())
            })
            .collect::<Result<Vec<_>, _>>()?;
        items.sort_by_key(|session| session.created_at_ms);
        Ok(items)
    }

    pub fn create_session(
        &self,
        request: CreateTerminalRequest,
    ) -> Result<TerminalSessionSummary, String> {
        let cwd = resolve_cwd(request.cwd.as_deref())?;
        let cols = normalize_cols(request.cols);
        let rows = normalize_rows(request.rows);
        let shell = resolve_shell(request.shell.as_deref());
        let session_id = Uuid::new_v4().to_string();
        let title = format!(
            "{DEFAULT_TERMINAL_TITLE} {}",
            self.next_index.fetch_add(1, Ordering::Relaxed)
        );
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;

        let mut command = CommandBuilder::new(shell.clone());
        command.cwd(cwd.clone());
        if request.shell.is_none() && cfg!(unix) {
            command.arg("-l");
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;

        let summary = TerminalSessionSummary {
            id: session_id.clone(),
            title,
            cwd: cwd.to_string_lossy().to_string(),
            shell,
            cols,
            rows,
            output: String::new(),
            running: true,
            exit_code: None,
            created_at_ms: now_ms(),
        };
        let state = Arc::new(Mutex::new(TerminalSessionState {
            summary: summary.clone(),
        }));
        let killer = child.clone_killer();

        self.sessions
            .lock()
            .map_err(|_| "terminal sessions lock poisoned".to_string())?
            .insert(
                session_id.clone(),
                TerminalSessionHandle {
                    state: Arc::clone(&state),
                    writer,
                    master: pair.master,
                    killer,
                },
            );

        self.emit(
            &session_id,
            TerminalEvent::Created {
                session: summary.clone(),
            },
        )?;
        self.spawn_reader(session_id.clone(), state, reader);
        self.spawn_exit_watcher(session_id.clone(), child);

        Ok(summary)
    }

    pub fn write_input(&self, request: WriteTerminalInputRequest) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "terminal sessions lock poisoned".to_string())?;
        let session = sessions
            .get_mut(&request.terminal_id)
            .ok_or_else(|| "terminal session not found".to_string())?;
        session
            .writer
            .write_all(request.data.as_bytes())
            .map_err(|error| error.to_string())?;
        session.writer.flush().map_err(|error| error.to_string())
    }

    pub fn resize_session(&self, request: ResizeTerminalRequest) -> Result<(), String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "terminal sessions lock poisoned".to_string())?;
        let session = sessions
            .get(&request.terminal_id)
            .ok_or_else(|| "terminal session not found".to_string())?;
        let cols = normalize_cols(Some(request.cols));
        let rows = normalize_rows(Some(request.rows));
        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        {
            let mut state = session
                .state
                .lock()
                .map_err(|_| "terminal session state lock poisoned".to_string())?;
            state.summary.cols = cols;
            state.summary.rows = rows;
        }
        drop(sessions);
        self.emit(&request.terminal_id, TerminalEvent::Resized { cols, rows })
    }

    pub fn close_session(&self, request: CloseTerminalRequest) -> Result<(), String> {
        let removed = self
            .sessions
            .lock()
            .map_err(|_| "terminal sessions lock poisoned".to_string())?
            .remove(&request.terminal_id);
        let Some(mut session) = removed else {
            return Ok(());
        };
        let _ = session.killer.kill();
        self.emit(&request.terminal_id, TerminalEvent::Removed)
    }

    fn spawn_reader(
        &self,
        terminal_id: String,
        state: Arc<Mutex<TerminalSessionState>>,
        mut reader: Box<dyn Read + Send>,
    ) {
        let app = self.app.clone();
        thread::spawn(move || {
            let mut buffer = [0_u8; TERMINAL_OUTPUT_CHUNK_LIMIT];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read_count) => {
                        let chunk = String::from_utf8_lossy(&buffer[..read_count]).to_string();
                        if chunk.is_empty() {
                            continue;
                        }
                        if let Ok(mut session) = state.lock() {
                            push_output(&mut session.summary.output, &chunk);
                        }
                        let _ = app.emit(
                            TERMINAL_EVENT_NAME,
                            TerminalEventEnvelope {
                                terminal_id: terminal_id.clone(),
                                event: TerminalEvent::Output { data: chunk },
                            },
                        );
                    }
                    Err(error) => {
                        log::warn!("[terminal] read failed for {terminal_id}: {error}");
                        break;
                    }
                }
            }
        });
    }

    fn spawn_exit_watcher(
        &self,
        terminal_id: String,
        mut child: Box<dyn Child + Send + Sync>,
    ) {
        let app = self.app.clone();
        let sessions = Arc::clone(&self.sessions);
        thread::spawn(move || {
            let exit_result = child.wait();
            let exit_code = match exit_result {
                Ok(status) => i32::try_from(status.exit_code()).ok(),
                Err(error) => {
                    log::warn!("[terminal] wait failed for {terminal_id}: {error}");
                    None
                }
            };
            if let Ok(mut sessions_guard) = sessions.lock() {
                if let Some(session) = sessions_guard.get_mut(&terminal_id) {
                    if let Ok(mut session_state) = session.state.lock() {
                        session_state.summary.running = false;
                        session_state.summary.exit_code = exit_code;
                    }
                }
            }
            let _ = app.emit(
                TERMINAL_EVENT_NAME,
                TerminalEventEnvelope {
                    terminal_id,
                    event: TerminalEvent::Closed { exit_code },
                },
            );
        });
    }

    fn emit(&self, terminal_id: &str, event: TerminalEvent) -> Result<(), String> {
        self.app
            .emit(
                TERMINAL_EVENT_NAME,
                TerminalEventEnvelope {
                    terminal_id: terminal_id.to_string(),
                    event,
                },
            )
            .map_err(|error| error.to_string())
    }
}

fn resolve_cwd(cwd: Option<&str>) -> Result<PathBuf, String> {
    let candidate = cwd
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let path = if candidate == "~" {
        dirs::home_dir().ok_or_else(|| "home directory not found".to_string())?
    } else if let Some(suffix) = candidate.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or_else(|| "home directory not found".to_string())?
            .join(suffix)
    } else {
        PathBuf::from(candidate)
    };
    normalize_existing_directory(&path)
}

fn normalize_existing_directory(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    let canonical = absolute
        .canonicalize()
        .map_err(|error| format!("invalid terminal cwd {}: {error}", absolute.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "terminal cwd is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn resolve_shell(shell: Option<&str>) -> String {
    let requested = shell
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(shell) = requested {
        return shell;
    }
    std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(windows) {
            DEFAULT_SHELL_WINDOWS.to_string()
        } else {
            DEFAULT_SHELL_UNIX.to_string()
        }
    })
}

fn normalize_cols(cols: Option<u16>) -> u16 {
    cols.unwrap_or(120).max(20)
}

fn normalize_rows(rows: Option<u16>) -> u16 {
    rows.unwrap_or(32).max(8)
}

fn push_output(buffer: &mut String, chunk: &str) {
    buffer.push_str(chunk);
    if buffer.len() <= TERMINAL_SCROLLBACK_LIMIT {
        return;
    }
    let trim_to = buffer.len().saturating_sub(TERMINAL_SCROLLBACK_LIMIT);
    let trim_start = buffer
        .char_indices()
        .find_map(|(index, _)| (index >= trim_to).then_some(index))
        .unwrap_or(trim_to);
    buffer.drain(..trim_start);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
