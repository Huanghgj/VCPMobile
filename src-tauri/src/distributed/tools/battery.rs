// distributed/tools/battery.rs
// [Streaming] MobileBatteryInfo — battery level, charging status, temperature.
// Reads /sys/class/power_supply/battery/{capacity, status, temp}

use crate::distributed::tool_registry::StreamingTool;
use crate::distributed::types::ToolManifest;

pub struct BatteryInfoTool;

impl StreamingTool for BatteryInfoTool {
    fn manifest(&self) -> ToolManifest {
        ToolManifest {
            name: "MobileBatteryInfo".to_string(),
            description: "读取电量、充电状态与低电量模式；系统未提供的电池温度等字段明确报告缺失。"
                .to_string(),
            display_name: "电池状态".to_string(),
            placeholder: Some("{{MobileBattery}}".to_string()),
            invocation_commands: vec![],
        }
    }

    fn placeholder_key(&self) -> &str {
        "{{MobileBattery}}"
    }

    fn poll_interval_secs(&self) -> u64 {
        60
    }

    fn read_current(&self, app: &tauri::AppHandle) -> Result<String, String> {
        use tauri::Manager;
        let dist_state = app.state::<crate::distributed::DistributedState>();
        let battery_info = dist_state.telemetry.get_battery_info(app);
        Ok(battery_info)
    }
}
