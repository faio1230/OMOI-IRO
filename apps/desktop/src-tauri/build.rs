// The app manifest lists every command, so each one gets its own permission
// (`allow-<command>`) and the capability grants exactly these.
const COMMANDS: &[&str] = &[
    "app_info",
    "list_ports",
    "runtime_start",
    "runtime_stop",
    "runtime_status",
    "set_blackout",
    "set_brightness",
    "reset_control_fault",
    "reconnect_ports",
    "preview_subscribe",
    "preview_ack",
    "preview_open",
    "preview_mode",
    "preview_play",
    "preview_seek",
    "preview_set_bpm",
    "preview_pad",
    "settings_get",
    "settings_check",
    "settings_save",
    "settings_listen",
    "editor_open",
    "editor_view",
    "material_preview",
    "editor_apply",
    "editor_undo",
    "editor_redo",
    "preset_import",
    "frames_import",
    "recording_stop",
    "reset_input",
    "recording_start",
    "preset_export",
    "editor_save",
    "editor_save_as",
    "editor_new",
    "close_decision",
];

fn main() {
    // `generate_context!` needs the frontend directory; a workspace build
    // (cargo test) must not depend on `npm run build` having run.
    let dist = std::path::Path::new("../dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(dist).expect("create ../dist");
        std::fs::write(dist.join("index.html"), "<!doctype html><title>OMOI-IRO</title><p>Frontend not built: run <code>npm run build</code> in apps/desktop.</p>\n")
            .expect("write placeholder index.html");
    }
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)))
        .expect("tauri build script");
}
