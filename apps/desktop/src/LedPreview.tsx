// U03: LED preview. Shows only frames computed in Rust (Channel, capacity 1,
// at most 30 fps; one subscription shared with the transport bar, previewStore). The local clock (play / scrub / preview tempo) moves the
// local preview only; it never changes Live's or a device's position.
import { useEffect, useState } from "react";
import * as ipc from "./ipc/commands";
import type { PreviewSource } from "./ipc/types";
import * as preview from "./previewStore";

function fmt(us: number | null): string {
  if (us === null) return "--";
  const s = us / 1e6;
  const m = Math.floor(s / 60);
  return `${m}:${(s - m * 60).toFixed(2).padStart(5, "0")}`;
}

export default function LedPreview(props: { showPath: string; running: boolean; onError: (e: string | null) => void }) {
  const { showPath, running, onError } = props;
  const { frame, info, error } = preview.usePreview();
  const mode: PreviewSource = info?.mode ?? "runtime";
  const [scrubUs, setScrubUs] = useState<number | null>(null);
  const [bpmText, setBpmText] = useState("");
  const [jumpText, setJumpText] = useState("");
  useEffect(() => { if (info?.bpm) setBpmText(String(info.bpm)); }, [info?.bpm]);
  useEffect(() => { onError(error); }, [error, onError]);

  const openLocal = () => preview.open(showPath);
  const switchMode = (m: PreviewSource) => preview.setMode(m);
  // Scrubbing while the runtime follows switches to the independent local preview.
  const scrub = (us: number) => preview.seek(us);

  const showing: PreviewSource | null = frame?.source ?? null;
  const length = info?.length_us ?? 0;
  const position = scrubUs ?? (frame?.source === "local" ? frame.show_time_us ?? 0 : info?.position_us ?? 0);
  const independent = running && showing === "local";

  return (
    <section className="panel">
      <h2>LEDプレビュー</h2>
      <div className="preview">
        <div className="pads" aria-label="LED preview">
          {Array.from({ length: 64 }, (_, k) => {
            // Rows top to bottom on screen; frame index y = 0 is the bottom row.
            const y = 7 - Math.floor(k / 8);
            const x = k % 8;
            const i = (y * 8 + x) * 3;
            const c = frame ? `rgb(${frame.rgb[i]}, ${frame.rgb[i + 1]}, ${frame.rgb[i + 2]})` : "#000";
            // Press / release = pad events for input-driven effects in the shown source (checking only, not device input).
            return <div key={k} className="pad" style={{ background: c }} onPointerDown={() => ipc.previewPad(x, y, true).catch(() => {})} onPointerUp={() => ipc.previewPad(x, y, false).catch(() => {})} onPointerLeave={(e) => e.buttons && ipc.previewPad(x, y, false).catch(() => {})} />;
          })}
        </div>
        <div className="preview-side">
          <div className="segmented">
            <button className={mode === "runtime" && running ? "on" : ""} disabled={!running} onClick={() => switchMode("runtime")}>実機出力（開始中）</button>
            <button className={mode === "local" ? "on" : ""} onClick={() => (info ? switchMode("local") : openLocal())}>手元で再生（Live と無関係）</button>
          </div>
          <p className="source">
            {!running && showing !== "local" && "「開始」していないため実機出力はありません。手元で再生を使ってください"}
            {running && showing === "runtime" && "表示中：実機へ送っている値（Live に追従）"}
            {showing === "local" && (independent ? "表示中：独立プレビュー（Live・機器の位置は変わらない）" : "表示中：手元で再生（ローカル時計）")}
            {running && showing === null && "実機出力のフレーム待ち"}
          </p>
          <div className="mono readout">
            {fmt(frame?.show_time_us ?? null)}
            {frame?.timecode ? ` · ${frame.timecode}` : ""}
            {frame?.dark ? ` · dark=${frame.dark}` : ""}
          </div>
          <div className="actions">
            <button onClick={openLocal} disabled={!showPath}>ショーを読み込む</button>
            <button disabled={!info} onClick={() => preview.play(!(info?.playing ?? false))}>{info?.playing ? "一時停止" : "再生"}</button>
          </div>
          <label className="scrub">
            <span>位置（ローカル時計）</span>
            <input
              type="range"
              min={0}
              max={Math.max(length, 1)}
              step={10_000}
              value={Math.min(position, Math.max(length, 1))}
              disabled={!info || !length}
              onChange={(e) => setScrubUs(Number(e.target.value))}
              onPointerUp={() => {
                if (scrubUs !== null) scrub(scrubUs).finally(() => setScrubUs(null));
              }}
              onKeyUp={() => {
                if (scrubUs !== null) scrub(scrubUs).finally(() => setScrubUs(null));
              }}
            />
            <span className="mono">{fmt(position)} / {fmt(length || null)}</span>
          </label>
          <label className="bpm">
            <span>位置を指定（秒）</span>
            <input value={jumpText} disabled={!info} aria-label="位置を指定（秒）" onChange={(e) => setJumpText(e.target.value)} />
            <button
              disabled={!info || !Number.isFinite(Number(jumpText)) || jumpText === ""}
              onClick={() => scrub(Math.round(Number(jumpText) * 1e6))}
            >
              移動
            </button>
          </label>
          <label className="bpm">
            <span>プレビューのテンポ</span>
            <input value={bpmText} disabled={!info || running} onChange={(e) => setBpmText(e.target.value)} onBlur={() => bpmText && Number(bpmText) !== info?.bpm && preview.setBpm(Number(bpmText))} />
            <span className="hint">{running ? "追従中は変更できない（showの値 " + (info?.show_bpm ?? "--") + "）" : `bpm（showの値 ${info?.show_bpm ?? "--"}）`}</span>
          </label>
          <div className="hint">表示 seq {frame?.seq ?? 0} · 間引き {frame?.skipped ?? 0}</div>
        </div>
      </div>
    </section>
  );
}
