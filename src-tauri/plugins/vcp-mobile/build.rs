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
    "save_image_from_path",
    "write_temp_file",
    "start_download_notification",
    "update_download_notification",
    "cancel_download_notification",
    "request_overlay_permission",
    "register_shared_files",
    "toggle_floating_ball",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
