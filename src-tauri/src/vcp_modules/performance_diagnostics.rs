use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};

#[derive(Debug, Serialize, Deserialize)]
pub struct PerformanceReportPayload {
    pub summary: Value,
    pub metrics: Vec<Value>,
    #[serde(rename = "traceEvents")]
    pub trace_events: Option<Vec<Value>>,
    #[serde(rename = "userNotes")]
    pub user_notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SavedPerformanceReport {
    pub path: String,
    pub filename: String,
    #[serde(rename = "savedAt")]
    pub saved_at: String,
}

#[derive(Debug, Serialize)]
pub struct ProcessPerformanceSnapshot {
    pub pid: u32,
    #[serde(rename = "rssBytes")]
    pub rss_bytes: Option<u64>,
    #[serde(rename = "vmSizeBytes")]
    pub vm_size_bytes: Option<u64>,
    pub threads: Option<u32>,
    #[serde(rename = "fdCount")]
    pub fd_count: Option<u32>,
    #[serde(rename = "processCpuTicks")]
    pub process_cpu_ticks: Option<u64>,
    #[serde(rename = "totalCpuTicks")]
    pub total_cpu_ticks: Option<u64>,
    #[serde(rename = "clockTicksPerSecond")]
    pub clock_ticks_per_second: Option<u64>,
    #[serde(rename = "gpuBusyPercent")]
    pub gpu_busy_percent: Option<f64>,
    #[serde(rename = "gpuBusyTicks")]
    pub gpu_busy_ticks: Option<u64>,
    #[serde(rename = "gpuTotalTicks")]
    pub gpu_total_ticks: Option<u64>,
    #[serde(rename = "gpuInfo")]
    pub gpu_info: Option<String>,
}

fn parse_status_kb(content: &str, key: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let rest = line.strip_prefix(key)?;
        let value = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        Some(value * 1024)
    })
}

fn parse_status_u32(content: &str, key: &str) -> Option<u32> {
    content.lines().find_map(|line| {
        let rest = line.strip_prefix(key)?;
        rest.split_whitespace().next()?.parse::<u32>().ok()
    })
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn read_process_cpu_ticks() -> Option<u64> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    let after_name = stat.rsplit_once(") ")?.1;
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some(utime + stime)
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn read_process_cpu_ticks() -> Option<u64> {
    None
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn read_total_cpu_ticks() -> Option<u64> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    let cpu_line = stat.lines().next()?.strip_prefix("cpu ")?;
    Some(
        cpu_line
            .split_whitespace()
            .filter_map(|part| part.parse::<u64>().ok())
            .sum(),
    )
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn read_total_cpu_ticks() -> Option<u64> {
    None
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn read_fd_count() -> Option<u32> {
    Some(
        fs::read_dir("/proc/self/fd")
            .ok()?
            .filter_map(Result::ok)
            .count() as u32,
    )
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn read_fd_count() -> Option<u32> {
    None
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn read_clock_ticks_per_second() -> Option<u64> {
    Some(100)
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn read_gpu_busy_percent() -> (Option<f64>, Option<u64>, Option<u64>, Option<String>) {
    let candidates = [
        "/sys/class/kgsl/kgsl-3d0/gpubusy",
        "/sys/class/kgsl/kgsl-3d0/devfreq/gpu_load",
        "/sys/devices/platform/kgsl-3d0.0/kgsl/kgsl-3d0/gpubusy",
    ];

    for path in candidates {
        if let Ok(content) = fs::read_to_string(path) {
            let parts: Vec<f64> = content
                .split_whitespace()
                .filter_map(|part| part.parse::<f64>().ok())
                .collect();
            if path.ends_with("gpubusy") && parts.len() >= 2 && parts[1] > 0.0 {
                let percent = (parts[0] / parts[1] * 100.0).clamp(0.0, 100.0);
                return (
                    Some((percent * 10.0).round() / 10.0),
                    Some(parts[0] as u64),
                    Some(parts[1] as u64),
                    Some(path.to_string()),
                );
            }
            if let Some(value) = parts.first() {
                let percent = if *value > 100.0 { value / 10.0 } else { *value };
                return (
                    Some(percent.clamp(0.0, 100.0)),
                    None,
                    None,
                    Some(path.to_string()),
                );
            }
        }
    }

    (
        None,
        None,
        None,
        Some("未找到可读 GPU busy 节点；不同 SoC/ROM 可能禁止普通应用读取".to_string()),
    )
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn read_clock_ticks_per_second() -> Option<u64> {
    None
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn read_gpu_busy_percent() -> (Option<f64>, Option<u64>, Option<u64>, Option<String>) {
    (
        None,
        None,
        None,
        Some("当前平台不支持 GPU busy 采样".to_string()),
    )
}

#[tauri::command]
pub fn get_process_performance_snapshot() -> ProcessPerformanceSnapshot {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();

    let (gpu_busy_percent, gpu_busy_ticks, gpu_total_ticks, gpu_info) = read_gpu_busy_percent();

    ProcessPerformanceSnapshot {
        pid: std::process::id(),
        rss_bytes: parse_status_kb(&status, "VmRSS:"),
        vm_size_bytes: parse_status_kb(&status, "VmSize:"),
        threads: parse_status_u32(&status, "Threads:"),
        fd_count: read_fd_count(),
        process_cpu_ticks: read_process_cpu_ticks(),
        total_cpu_ticks: read_total_cpu_ticks(),
        clock_ticks_per_second: read_clock_ticks_per_second(),
        gpu_busy_percent,
        gpu_busy_ticks,
        gpu_total_ticks,
        gpu_info,
    }
}

#[derive(Debug, Serialize)]
pub struct PerformanceDiagnosticsInfo {
    #[serde(rename = "reportsDir")]
    pub reports_dir: String,
    pub platform: String,
    pub arch: String,
    #[serde(rename = "debugBuild")]
    pub debug_build: bool,
}

fn reports_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let mut path = external_app_files_dir(app).unwrap_or_else(|| fallback_app_data_dir(app));
    path.push("VCPMobile");
    path.push("performance-reports");
    fs::create_dir_all(&path).map_err(|error| format!("无法创建性能报告目录: {error}"))?;
    Ok(path)
}

#[cfg(target_os = "android")]
fn external_app_files_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().document_dir().ok()
}

#[cfg(not(target_os = "android"))]
fn external_app_files_dir<R: Runtime>(_app: &AppHandle<R>) -> Option<PathBuf> {
    None
}

fn fallback_app_data_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("AppData"))
}

#[tauri::command]
pub fn get_performance_diagnostics_info<R: Runtime>(
    app: AppHandle<R>,
) -> Result<PerformanceDiagnosticsInfo, String> {
    let reports_dir = reports_dir(&app)?;

    Ok(PerformanceDiagnosticsInfo {
        reports_dir: reports_dir.to_string_lossy().to_string(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        debug_build: cfg!(debug_assertions),
    })
}

#[tauri::command]
pub fn save_performance_report<R: Runtime>(
    app: AppHandle<R>,
    payload: PerformanceReportPayload,
) -> Result<SavedPerformanceReport, String> {
    let reports_dir = reports_dir(&app)?;
    let saved_at = Utc::now().to_rfc3339();
    let filename = format!(
        "vcp-mobile-performance-{}.json",
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    let path = reports_dir.join(&filename);

    let report = serde_json::json!({
        "schemaVersion": 1,
        "app": "vcp-mobile",
        "savedAt": saved_at,
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "debugBuild": cfg!(debug_assertions),
        "summary": payload.summary,
        "metrics": payload.metrics,
        "traceEvents": payload.trace_events.unwrap_or_default(),
        "userNotes": payload.user_notes.unwrap_or_default(),
    });

    let content = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("无法序列化性能报告: {error}"))?;
    fs::write(&path, content).map_err(|error| format!("无法写入性能报告: {error}"))?;

    Ok(SavedPerformanceReport {
        path: path.to_string_lossy().to_string(),
        filename,
        saved_at,
    })
}
