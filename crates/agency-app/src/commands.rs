use agency_core::registry::Project;
use agency_core::supervisor::AgentStatus;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::state::{AppState, TaskInfo};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalChunk {
    pub b64: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub state: String,
    pub code: Option<i32>,
}

fn status_dto(status: AgentStatus) -> StatusDto {
    match status {
        AgentStatus::Running => StatusDto { state: "running".into(), code: None },
        AgentStatus::Idle => StatusDto { state: "idle".into(), code: None },
        AgentStatus::Exited(c) => StatusDto { state: "exited".into(), code: Some(c) },
        AgentStatus::Crashed => StatusDto { state: "crashed".into(), code: None },
    }
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    state.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_project(
    state: State<'_, AppState>,
    name: String,
    repo_path: String,
) -> Result<Project, String> {
    state
        .add_project(&name, std::path::Path::new(&repo_path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.remove_project(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_task(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    profile: String,
    on_chunk: Channel<TerminalChunk>,
) -> Result<TaskInfo, String> {
    state
        .start_task(&project_id, &prompt, &profile, "HEAD", move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn send_input(
    state: State<'_, AppState>,
    task_id: String,
    data: String,
) -> Result<(), String> {
    state
        .send_input(&task_id, data.as_bytes())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn task_status(state: State<'_, AppState>, task_id: String) -> Result<StatusDto, String> {
    state
        .task_status(&task_id)
        .map(status_dto)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.stop_task(&task_id).map_err(|e| e.to_string())
}
