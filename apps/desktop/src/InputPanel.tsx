// 0.1.3 stage 8: the pad input on the performance tab. "Reset input" forgets
// every press (pads / cycle / constellation start empty). Recording takes the
// presses from the current show time until stopped, then adds them as a new
// clip of the chosen triggered layer in the open show (through the history,
// like any edit). Both need the runtime running.
import { useEffect, useState } from "react";
import * as ipc from "./ipc/commands";
import type { EditorView } from "./ipc/types";

const secs = (us: number) => `${(us / 1e6).toFixed(2)} s`;

export default function InputPanel(props: { running: boolean; recordingSinceUs: number | null; docKey: string; run: (f: () => Promise<unknown>) => void }) {
  const { running, recordingSinceUs, run } = props;
  const [view, setView] = useState<EditorView | null>(null);
  const [layerId, setLayerId] = useState("");
  const [presetId, setPresetId] = useState("");
  const [note, setNote] = useState<string | null>(null);
  useEffect(() => {
    ipc.editorView().then((v) => setView(v ?? null)).catch(() => setView(null));
  }, [props.docKey, recordingSinceUs]);
  const layers = view?.layers.filter((l) => l.kind === "triggered") ?? [];
  const layer = layers.find((l) => l.id === layerId) ?? layers[0];
  // Default preset: the one the layer already draws.
  const preset = presetId || layer?.clips[0]?.preset_id || view?.presets[0]?.id || "";
  const recording = recordingSinceUs !== null;
  const stop = () =>
    run(async () => {
      if (!layer || !preset) return;
      const v = await ipc.recordingStop(layer.id, preset);
      setView(v);
      setNote(`録音を ${layer.name} に追加した（${v.outcome?.clip_id ?? ""}）。保存はタイムラインで。`);
    });
  return (
    <section className="panel">
      <h2>パッド入力</h2>
      <div className="actions">
        <button disabled={!running} onClick={() => run(() => ipc.resetInput())} title="叩いた回数・溜まった熱・置いた色・星座の線を消す">入力をリセット</button>
        <span className="hint">「パッドの状態」「叩くたびに色」「星座」は入力履歴に依存する（巻き戻しでは再現しない）。</span>
      </div>
      <div className="actions">
        <label className="inline">
          録音先（発火レイヤー）
          <select value={layer?.id ?? ""} disabled={recording || !layers.length} onChange={(e) => setLayerId(e.target.value)}>
            {layers.length ? layers.map((l) => <option key={l.id} value={l.id}>{l.name}</option>) : <option value="">（タイムラインで発火レイヤーを作る）</option>}
          </select>
        </label>
        <label className="inline">
          描く効果
          <select value={preset} disabled={recording || !view} onChange={(e) => setPresetId(e.target.value)}>
            {view?.presets.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
        </label>
        {recording ? (
          <button className="blackout on" onClick={stop}>■ 録音を止めて追加（{secs(recordingSinceUs!)} から）</button>
        ) : (
          <button disabled={!running || !layer || !preset} onClick={() => { setNote(null); run(() => ipc.recordingStart()); }}>● 録音</button>
        )}
      </div>
      {recording && <p className="hint warn">録音中：パッドの押下を曲の時刻で記録している。</p>}
      {note && <p className="hint">{note}</p>}
    </section>
  );
}