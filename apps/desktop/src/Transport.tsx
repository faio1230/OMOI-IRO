// Timeline transport bar and playhead. Both read the shared preview state
// (previewStore), so only they re-render per frame. Play / seek move the local
// clock only; while the runtime follows Live this is the independent preview.
import { useEffect, useRef, useState } from "react";
import * as preview from "./previewStore";
import { clockView, fmtFrames, positionUs, usePreview } from "./previewStore";

const editable = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "SELECT", "TEXTAREA", "BUTTON"].includes(t.tagName));

export function TransportBar({ running, blocked }: { running: boolean; blocked: string | null }) {
  const s = usePreview();
  const { frame } = s;
  const clock = clockView(s);
  const onRuntime = clock?.kind === "runtime";
  const [bpmText, setBpmText] = useState("");
  useEffect(() => { if (clock?.bpm) setBpmText(String(clock.bpm)); }, [clock?.bpm]);
  const enabled = !!clock && !blocked;
  const playing = clock?.playing ?? false;
  const toggle = useRef(() => {});
  toggle.current = () => { if (enabled) preview.play(!playing); };
  // Space: play / pause (not while typing or on a focused button, which Space already clicks).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== "Space" || e.repeat || e.ctrlKey || e.metaKey || e.altKey || editable(e.target)) return;
      e.preventDefault();
      toggle.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const badge = onRuntime ? "実機出力（手元の時計）" : frame?.source === "runtime" ? "実機出力（Live に追従）" : frame?.source === "local" ? (running ? "独立プレビュー" : "手元で再生") : null;
  // Tempo: the runtime's local clock, or the preview tempo while nothing follows Live.
  const bpmEditable = !!clock && (onRuntime || !running);
  const commitBpm = () => { const v = Number(bpmText); if (bpmText && Number.isFinite(v) && v !== clock?.bpm) preview.setBpm(v); };
  return (
    <div className="tl-transport" role="toolbar" aria-label="再生操作">
      <button disabled={!enabled} title="先頭へ" aria-label="先頭へ" onClick={() => preview.seek(0)}>|◀</button>
      <button disabled={!enabled} className={playing ? "on" : ""} title="再生／一時停止（Space）" aria-label={playing ? "一時停止" : "再生"} onClick={() => toggle.current()}>{playing ? "❚❚" : "▶"}</button>
      <button disabled={!enabled} title="停止（先頭へ戻して停止）" aria-label="停止" onClick={() => preview.stop()}>■</button>
      <span className="mono tl-transport-time">{blocked ? "-:--.--" : fmtFrames(clock?.position_us ?? null)} / {fmtFrames(clock?.length_us ?? null)}</span>
      <label className="inline">
        テンポ
        <input className="tl-transport-bpm" value={bpmText} disabled={!bpmEditable} title={bpmEditable ? (onRuntime ? "手元の時計のテンポ（bpm）" : "プレビューのテンポ（bpm）") : "Live に追従中は変更できない"}
          onChange={(e) => setBpmText(e.target.value)} onBlur={commitBpm} onKeyDown={(e) => { if (e.key === "Enter") commitBpm(); }} />
        bpm
      </label>
      {badge && <span className={onRuntime || frame?.source === "runtime" ? "badge good" : "badge"}>{badge}</span>}
      {blocked && <span className="hint">{blocked}</span>}
      {!clock && !blocked && <span className="hint">プレビューを準備中…</span>}
      {s.error && <span className="error">{s.error}</span>}
    </div>
  );
}

/** The timeline playhead at the shown position (`fallbackUs` before any frame). */
export function Playhead({ left, pxPerSec, fallbackUs }: { left: number; pxPerSec: number; fallbackUs: number | null }) {
  const s = usePreview();
  const us = positionUs(s) ?? fallbackUs;
  if (us === null) return null;
  return <div className="tl-playhead" style={{ left: left + (us / 1e6) * pxPerSec }} />;
}
