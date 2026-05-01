use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager, Runtime};
use uuid::Uuid;

use super::surface_types::{
    SurfaceCommand, SurfaceWidget, SurfaceWidgetBounds, SurfaceWidgetSource,
    UpsertSurfaceWidgetRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SurfaceStateFile {
    widgets: Vec<SurfaceWidget>,
}

fn surface_state_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let mut dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("Surface config dir unavailable: {error}"))?;
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Create Surface config dir failed: {error}"))?;
    dir.push("mobile_surface_widgets.json");
    Ok(dir)
}

fn load_state<R: Runtime>(app: &AppHandle<R>) -> Result<SurfaceStateFile, String> {
    let path = surface_state_path(app)?;
    if !path.exists() {
        return Ok(SurfaceStateFile::default());
    }

    let raw =
        fs::read_to_string(&path).map_err(|error| format!("Read Surface state failed: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("Parse Surface state failed: {error}"))
}

fn save_state<R: Runtime>(app: &AppHandle<R>, state: &SurfaceStateFile) -> Result<(), String> {
    let path = surface_state_path(app)?;
    let raw = serde_json::to_string_pretty(state)
        .map_err(|error| format!("Serialize Surface state failed: {error}"))?;
    fs::write(&path, raw).map_err(|error| format!("Write Surface state failed: {error}"))
}

fn find_widget_mut<'a>(
    state: &'a mut SurfaceStateFile,
    widget_id: &str,
) -> Option<&'a mut SurfaceWidget> {
    state
        .widgets
        .iter_mut()
        .find(|widget| widget.id == widget_id)
}

#[tauri::command]
pub fn list_surface_widgets<R: Runtime>(app: AppHandle<R>) -> Result<Vec<SurfaceWidget>, String> {
    let mut widgets = load_state(&app)?.widgets;
    widgets.sort_by_key(|widget| widget.bounds.z_index);
    Ok(widgets)
}

#[tauri::command]
pub fn upsert_surface_widget<R: Runtime>(
    app: AppHandle<R>,
    request: UpsertSurfaceWidgetRequest,
) -> Result<SurfaceWidget, String> {
    let mut state = load_state(&app)?;
    let now = Utc::now().timestamp_millis();
    let id = request
        .id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("surface-{}", Uuid::new_v4()));

    let updated = if let Some(widget) = find_widget_mut(&mut state, &id) {
        widget.title = request.title;
        widget.html = request.html;
        if let Some(bounds) = request.bounds {
            widget.bounds = bounds;
        }
        if let Some(favorite) = request.favorite {
            widget.favorite = favorite;
        }
        if let Some(source) = request.source {
            widget.source = source;
        }
        widget.updated_at = now;
        widget.clone()
    } else {
        let widget = SurfaceWidget {
            id,
            title: request.title,
            html: request.html,
            bounds: request.bounds.unwrap_or_default(),
            favorite: request.favorite.unwrap_or(false),
            source: request.source.unwrap_or_default(),
            created_at: now,
            updated_at: now,
        };
        state.widgets.push(widget.clone());
        widget
    };

    save_state(&app, &state)?;
    Ok(updated)
}

#[tauri::command]
pub fn remove_surface_widget<R: Runtime>(
    app: AppHandle<R>,
    widget_id: String,
) -> Result<bool, String> {
    let mut state = load_state(&app)?;
    let before = state.widgets.len();
    state.widgets.retain(|widget| widget.id != widget_id);
    let removed = before != state.widgets.len();
    if removed {
        save_state(&app, &state)?;
    }
    Ok(removed)
}

#[tauri::command]
pub fn clear_surface_widgets<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    save_state(&app, &SurfaceStateFile::default())
}

#[tauri::command]
pub fn apply_surface_command<R: Runtime>(
    app: AppHandle<R>,
    command: SurfaceCommand,
) -> Result<Vec<SurfaceWidget>, String> {
    let mut state = load_state(&app)?;
    let now = Utc::now().timestamp_millis();

    match command {
        SurfaceCommand::Create { widget_id, options } => {
            if find_widget_mut(&mut state, &widget_id).is_none() {
                state.widgets.push(SurfaceWidget {
                    id: widget_id,
                    title: None,
                    html: String::new(),
                    bounds: options.unwrap_or_default(),
                    favorite: false,
                    source: SurfaceWidgetSource::DesktopPush,
                    created_at: now,
                    updated_at: now,
                });
            }
        }
        SurfaceCommand::Append { widget_id, content } => {
            if let Some(widget) = find_widget_mut(&mut state, &widget_id) {
                widget.html.push_str(&content);
                widget.updated_at = now;
            } else {
                state.widgets.push(SurfaceWidget {
                    id: widget_id,
                    title: None,
                    html: content,
                    bounds: SurfaceWidgetBounds::default(),
                    favorite: false,
                    source: SurfaceWidgetSource::DesktopPush,
                    created_at: now,
                    updated_at: now,
                });
            }
        }
        SurfaceCommand::Finalize { widget_id } => {
            if let Some(widget) = find_widget_mut(&mut state, &widget_id) {
                widget.updated_at = now;
            }
        }
        SurfaceCommand::Replace {
            target_selector: _,
            content,
        } => {
            if let Some(widget) = state
                .widgets
                .iter_mut()
                .max_by_key(|widget| widget.updated_at)
            {
                widget.html = content;
                widget.updated_at = now;
            }
        }
        SurfaceCommand::Remove { widget_id } => {
            state.widgets.retain(|widget| widget.id != widget_id);
        }
        SurfaceCommand::Clear => {
            state.widgets.clear();
        }
    }

    save_state(&app, &state)?;
    let mut widgets = state.widgets;
    widgets.sort_by_key(|widget| widget.bounds.z_index);
    Ok(widgets)
}
