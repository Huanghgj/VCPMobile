use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceWidgetBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub z_index: i64,
}

impl Default for SurfaceWidgetBounds {
    fn default() -> Self {
        Self {
            x: 16.0,
            y: 96.0,
            width: 320.0,
            height: 220.0,
            z_index: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceWidget {
    pub id: String,
    pub title: Option<String>,
    pub html: String,
    pub bounds: SurfaceWidgetBounds,
    pub favorite: bool,
    pub source: SurfaceWidgetSource,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SurfaceWidgetSource {
    DesktopPush,
    MobileTool,
    User,
}

impl Default for SurfaceWidgetSource {
    fn default() -> Self {
        Self::DesktopPush
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertSurfaceWidgetRequest {
    pub id: Option<String>,
    pub title: Option<String>,
    pub html: String,
    pub bounds: Option<SurfaceWidgetBounds>,
    pub favorite: Option<bool>,
    pub source: Option<SurfaceWidgetSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "action")]
pub enum SurfaceCommand {
    Create {
        widget_id: String,
        options: Option<SurfaceWidgetBounds>,
    },
    Append {
        widget_id: String,
        content: String,
    },
    Finalize {
        widget_id: String,
    },
    Replace {
        target_selector: Option<String>,
        content: String,
    },
    Remove {
        widget_id: String,
    },
    Clear,
}
