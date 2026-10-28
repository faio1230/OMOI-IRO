// Typed wrappers over the Rust commands (apps/desktop/src-tauri/src/lib.rs).
import { Channel, invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { AppInfo, CommandError, EditCommand, EditorView, OnConflict, ListenResult, PortsDto, PreviewFrame, PreviewInfo, PreviewSource, SettingsCheck, SettingsView, Snap, StartRequest, StartResult, StatusDto, StopResult, TerminalSettings } from "./types";

export const appInfo = () => invoke<AppInfo>("app_info");
export const listPorts = () => invoke<PortsDto>("list_ports");
export const runtimeStart = (req: StartRequest) => invoke<StartResult>("runtime_start", { req });
export const runtimeStop = () => invoke<StopResult>("runtime_stop");
export const runtimeStatus = () => invoke<StatusDto>("runtime_status");
export const setBlackout = (on: boolean) => invoke<void>("set_blackout", { on });
export const setBrightness = (value: number) => invoke<void>("set_brightness", { value });
export const resetControlFault = () => invoke<void>("reset_control_fault");
export const reconnectPorts = () => invoke<void>("reconnect_ports");

/** Subscribe to Rust-rendered preview frames. Call `previewAck(frame.seq)` after drawing each one. */
export function previewSubscribe(onFrame: (f: PreviewFrame) => void): Promise<void> {
  const channel = new Channel<PreviewFrame>();
  channel.onmessage = onFrame;
  return invoke<void>("preview_subscribe", { onFrame: channel });
}
export const previewAck = (seq: number) => invoke<void>("preview_ack", { seq });
export const previewOpen = (showPath: string) => invoke<PreviewInfo>("preview_open", { showPath });
export const previewMode = (mode: PreviewSource) => invoke<PreviewInfo>("preview_mode", { mode });
export const previewPlay = (playing: boolean) => invoke<PreviewInfo>("preview_play", { playing });
export const previewSeek = (positionUs: number) => invoke<PreviewInfo>("preview_seek", { positionUs });
export const previewSetBpm = (bpm: number) => invoke<PreviewInfo>("preview_set_bpm", { bpm });
/** Preview grid click (bottom-left origin): a pad press for Ripple; nothing goes to Live. */
export const previewPad = (x: number, y: number) => invoke<void>("preview_pad", { x, y });

export const settingsGet = () => invoke<SettingsView>("settings_get");
/** Lists ports and checks the roles; sends nothing. */
export const settingsCheck = (settings: TerminalSettings) => invoke<SettingsCheck>("settings_check", { settings });
export const settingsSave = (settings: TerminalSettings) => invoke<SettingsView>("settings_save", { settings });
/** Opens one input role and counts what arrives; sends nothing. */
export const settingsListen = (settings: TerminalSettings, role: "control_in" | "clock_in", durationMs: number) =>
  invoke<ListenResult>("settings_listen", { settings, role, durationMs });

/** `discard`: open even with unsaved changes (otherwise refused with code "unsaved"). */
export const editorOpen = (showPath: string, discard = false) => invoke<EditorView>("editor_open", { showPath, discard });
export const editorView = () => invoke<EditorView | null>("editor_view");
/** Applied and validated in Rust; a rejected edit changes nothing. */
export const editorApply = (command: EditCommand, snap: Snap) => invoke<EditorView>("editor_apply", { command, snap });
export const editorUndo = () => invoke<EditorView>("editor_undo");
export const editorRedo = () => invoke<EditorView>("editor_redo");
/** Import a 0.1.1 preset file (kind omoiiro.preset) through the history. */
export const presetImport = (path: string, onConflict: OnConflict) => invoke<EditorView>("preset_import", { path, onConflict });
export const presetExport = (presetId: string, path: string, overwrite: boolean) => invoke<string>("preset_export", { presetId, path, overwrite });

export const editorSave = () => invoke<EditorView>("editor_save");
/** A new empty show (unsaved until "save as"). */
export const editorNew = (name: string, discard = false) => invoke<EditorView>("editor_new", { name, discard });
/** Rust checks the path (*.show.json, folder exists) and adds the extension if missing. */
export const editorSaveAs = (path: string) => invoke<EditorView>("editor_save_as", { path });
/** Close confirmation: save then quit, quit without saving, or stay. */
export const closeDecision = (action: "save" | "discard" | "cancel") => invoke<void>("close_decision", { action });

// File pickers (dialog plugin: open / save only). They return a path; Rust validates it.
export const pickShow = () => open({ multiple: false, directory: false, filters: [{ name: "OMOI-IRO show (*.show.json)", extensions: ["json"] }] }) as Promise<string | null>;
export const pickShowSave = (defaultPath?: string) => save({ defaultPath, filters: [{ name: "OMOI-IRO show (*.show.json)", extensions: ["json"] }] });
export const pickPreset = () => open({ multiple: false, directory: false, filters: [{ name: "OMOI-IRO preset (*.preset.json)", extensions: ["json"] }] }) as Promise<string | null>;
export const pickPresetSave = (defaultPath?: string) => save({ defaultPath, filters: [{ name: "OMOI-IRO preset (*.preset.json)", extensions: ["json"] }] });

/** Pointer list of a validation error, if any. */
export function errorDetails(e: unknown): { pointer: string; message: string }[] {
  return e && typeof e === "object" && "details" in e && Array.isArray((e as CommandError).details) ? (e as CommandError).details! : [];
}

/** Refused because the open document has unsaved changes (open / new). */
export function isUnsaved(e: unknown): boolean {
  return !!e && typeof e === "object" && (e as CommandError).code === "unsaved";
}

/** Error text for display (Rust returns CommandError; anything else is shown as is). */
export function errorText(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) {
    const c = e as CommandError;
    return c.code ? `${c.message} (${c.code})` : c.message;
  }
  return String(e);
}
