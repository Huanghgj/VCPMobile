const COMMANDS: &[&str] = &[
    "set_keep_screen_on",
    "clear_keep_screen_on",
    "start_streaming_service",
    "stop_streaming_service",
    "check_all_permissions",
    "request_android_permission",
    "move_task_to_back",
    "pick_file",
    "get_battery_status",
    "capture_window_snapshot",
    "open_file_native",
    "save_image_to_gallery",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
