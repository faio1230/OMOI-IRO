// Timeline transport bar and playhead. Both read the shared preview state
// (previewStore), so only they re-render per frame. Play / seek move the local
// clock only; while the runtime follows Live this is the independent preview.
import { useEffect, useRef, useState } from "react";
import * as preview from "./previewStore";
import { fmtFrames, positionUs, usePreview } from "./previewStore";

const editable = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "SELECT", "TEXTAREA", "BUTTON"].includes(t.tagName));

export function TransportBar({ running, blocked }: { running: boolean; blocked: string | null }) {
  const s = usePreview();
  const { info, frame } = s;
  const [bpmText, setBpmText] = useState("");
  useEffect(() => { if (info?.bpm) setBpmText(String(info.bpm)); }, [info?.bpm]);
  const enabled = !!info && !blocked;
  const playing = frame?.source === "local" ? frame.playing : info?.mode === "local" && info.playing;
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
  const badge = frame?.source === "runtime" ? "実機出力（開始中）" : frame?.source === "local" ? (running ? "独立プレビュー" : "手元で再生") : null;
  const commitBpm = () => { const v = Number(bpmText); if (bpmText && Number.isFinite(v) && v !== info?.bpm) preview.setBpm(v); };
  return (
    <div className="tl-transport" role="toolbar" aria-label="再生操作">
      <button disabled={!enabled} title="先頭へ" aria-label="先頭へ" onClick={() => preview.seek(0)}>|◀</button>
      <button disabled={!enabled} className={playing ? "on" : ""} title="再生／一時停止（Space）" aria-label={playing ? "一時停止" : "再生"} onClick={() => toggle.current()}>{playing ? "❚❚" : "▶"}</button>
      <button disabled={!enabled} title="停止（先頭へ戻して停止）" aria-label="停止" onClick={() => preview.stop()}>■</button>
      <span className="mono tl-transport-time">{blocked ? "-:--.--" : fmtFrames(positionUs(s))} / {fmtFrames(info?.length_us ?? null)}</span>
      <label className="inline">
        テンポ
        <input className="tl-transport-bpm" value={bpmText} disabled={!info || running} title={running ? "追従中は変更できない" : "プレビューのテンポ（bpm）"}
          onChange={(e) => setBpmText(e.target.value)} onBlur={commitBpm} onKeyDown={(e) => { if (e.key === "Enter") commitBpm(); }} />
        bpm
      </label>
      {badge && <span className={frame?.source === "runtime" ? "badge good" : "badge"}>{badge}</span>}
      {blocked && <span className="hint">{blocked}</span>}
      {!info && !blocked && <span className="hint">プレビューを準備中…</span>}
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
