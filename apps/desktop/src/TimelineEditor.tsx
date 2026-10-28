// U04: timeline editing. Every change is an EditCommand applied and
// validated in Rust (history, Undo / Redo there too); this view only draws
// the returned EditorView. Drags show a local ghost until release, then send
// one command. Nothing here moves the show clock.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as ipc from "./ipc/commands";
import type { ClipDto, EditCommand, EditorView, LayerDto, Recording, Snap } from "./ipc/types";
import { LayerInspector, PresetPanel, ShowSyncPanel, TRIGGER_JA, type PresetEditMode } from "./ShowInspectors";
import UnsavedChoice from "./UnsavedChoice";
import MaterialBrowser from "./MaterialBrowser";
import { CompoundClipFields, CompoundPanel } from "./Compounds";

const ROW_H = 46;
const HEAD_W = 190;
const DEFAULT_CLIP_US = 4_000_000;

type Drag = { kind: "move" | "resize"; clip: ClipDto; layerId: string; x0: number; dx: number };

const sec = (us: number) => (us / 1e6).toFixed(3);
const frames30 = (us: number) => Math.round((us * 30) / 1e6);

function NumField(props: { label: string; value: string; onChange: (v: string) => void; hint?: string }) {
  return (
    <label className="field">
      <span>{props.label}</span>
      <input value={props.value} onChange={(e) => props.onChange(e.target.value)} inputMode="decimal" />
      {props.hint && <span className="hint">{props.hint}</span>}
    </label>
  );
}

function RecordingLength({ clipId, recording, apply }: { clipId: string; recording: Recording; apply: (c: EditCommand) => void }) {
  const lengthUs = recording.length_us ?? Math.max(1, ...recording.events.map((e) => e.t_us + (e.duration_us ?? 0)));
  const [seconds, setSeconds] = useState(String(lengthUs / 1e6));
  useEffect(() => setSeconds(String(lengthUs / 1e6)), [clipId, lengthUs]);
  const value = Math.round(Number(seconds) * 1e6);
  const valid = seconds.trim() !== "" && Number.isSafeInteger(value) && value > 0;
  return (
    <div className="actions">
      <NumField label="録音ループ長（秒）" value={seconds} onChange={setSeconds} hint="配置クリップの長さは変わりません。" />
      <button disabled={!valid || value === lengthUs} onClick={() => apply({ op: "set_clip_recording", clip_id: clipId, recording: { ...recording, length_us: value }, looping: null })}>ループ長を適用</button>
      {!valid && <span className="error">0より大きい秒数を入力してください。</span>}
    </div>
  );
}

export default function TimelineEditor(props: { showPath: string; playheadUs: number | null; docKey: string; newRequested: boolean; onNewHandled: () => void }) {
  const { showPath, playheadUs, docKey, newRequested, onNewHandled } = props;
  const [view, setView] = useState<EditorView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [snap, setSnap] = useState<Snap>("frame30");
  const [pxPerSec, setPxPerSec] = useState(24);
  const [selClip, setSelClip] = useState<string | null>(null);
  const [selLayer, setSelLayer] = useState<string | null>(null);
  const [presetId, setPresetId] = useState<string>("");
  const [drag, setDrag] = useState<Drag | null>(null);
  const [form, setForm] = useState({ start: "", duration: "", fadeIn: "", fadeOut: "" });
  const [presetMode, setPresetMode] = useState<PresetEditMode | null>(null);
  // The timeline shows the show's layers, or a compound's inner layers.
  const [scope, setScope] = useState<string | null>(null);
  const busy = useRef(false);

  // Refresh when the document changes elsewhere (opened, saved from the header).
  useEffect(() => {
    ipc.editorView().then((v) => v && setView(v)).catch(() => {});
  }, [docKey]);

  const accept = (v: EditorView) => {
    setView(v);
    setError(null);
    if (v.outcome?.clip_id) setSelClip(v.outcome.clip_id);
    if (v.outcome?.layer_id) setSelLayer(v.outcome.layer_id);
    // A new preset (added or imported) opens in the preset editor.
    if (v.outcome?.preset_id && !v.outcome.clip_id && v.presets.some((p) => p.id === v.outcome!.preset_id)) {
      setPresetMode({ kind: "shared", presetId: v.outcome.preset_id });
    }
    // After a copy-on-write edit the clip's editor follows the new preset.
    setPresetMode((m) => (m && m.kind === "clip" && v.outcome?.clip_id === m.clipId && v.outcome.preset_id ? { ...m, presetId: v.outcome.preset_id } : m));
    if (!presetId && v.presets.length) setPresetId(v.presets[0].id);
  };

  const call = useCallback(async (f: () => Promise<EditorView>) => {
    if (busy.current) return;
    busy.current = true;
    try {
      accept(await f());
    } catch (e) {
      setError(ipc.errorText(e));
    } finally {
      busy.current = false;
    }
  }, [presetId]);

  const apply = (c: EditCommand) => call(() => ipc.editorApply(c, snap));
  // Open / new with unsaved changes: ask (save first / discard / cancel).
  const [pending, setPending] = useState<"open" | "new" | null>(null);
  const load = (what: "open" | "new", discard = false) => (what === "open" ? ipc.editorOpen(showPath, discard) : ipc.editorNew("", discard));
  const ask = (what: "open" | "new") =>
    call(async () => {
      try {
        return await load(what);
      } catch (e) {
        if (!ipc.isUnsaved(e)) throw e;
        // Ask instead of showing the refusal; the open document stays.
        setPending(what);
        const v = await ipc.editorView();
        if (!v) throw e;
        return v;
      }
    });
  const resolve = (choice: "save" | "discard") => {
    const what = pending!;
    call(async () => {
      if (choice === "save") {
        // A new show has no file yet: "save as" first; cancelling the dialog keeps the question.
        const doc = await ipc.editorView();
        if (doc?.path) {
          await ipc.editorSave();
        } else {
          const p = await ipc.pickShowSave();
          if (!p) return doc!;
          await ipc.editorSaveAs(p);
        }
      }
      const v = await load(what, choice === "discard");
      setPending(null);
      return v;
    });
  };
  const open = () => ask("open");
  // "新規…" in the header: same flow as the toolbar button, once per request.
  useEffect(() => {
    if (!newRequested) return;
    onNewHandled();
    ask("new");
  }, [newRequested]);

  const clips = useMemo(() => {
    const m = new Map<string, { clip: ClipDto; layer: LayerDto }>();
    view?.layers.forEach((l) => l.clips.forEach((c) => m.set(c.id, { clip: c, layer: l })));
    view?.compounds.forEach((cp) => cp.layers.forEach((l) => l.clips.forEach((c) => m.set(c.id, { clip: c, layer: l }))));
    return m;
  }, [view]);
  const selected = selClip ? clips.get(selClip) : undefined;

  // Inspector fields follow the selected clip.
  useEffect(() => {
    if (!selected) return;
    const c = selected.clip;
    setForm({ start: sec(c.start_us), duration: sec(c.duration_us), fadeIn: sec(c.fade_in_us), fadeOut: sec(c.fade_out_us) });
  }, [selected?.clip.start_us, selected?.clip.duration_us, selected?.clip.fade_in_us, selected?.clip.fade_out_us, selClip]);

  // Keyboard: Ctrl+Z / Ctrl+Y (Ctrl+Shift+Z), Delete, Ctrl+D.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT" || (e.target as HTMLElement)?.tagName === "SELECT") return;
      if (e.ctrlKey && e.key.toLowerCase() === "z" && !e.shiftKey) {
        e.preventDefault();
        if (view?.can_undo) call(() => ipc.editorUndo());
      } else if (e.ctrlKey && (e.key.toLowerCase() === "y" || (e.shiftKey && e.key.toLowerCase() === "z"))) {
        e.preventDefault();
        if (view?.can_redo) call(() => ipc.editorRedo());
      } else if (e.key === "Delete" && selClip) {
        apply({ op: "delete_clip", clip_id: selClip });
        setSelClip(null);
      } else if (e.ctrlKey && e.key.toLowerCase() === "d" && selClip) {
        e.preventDefault();
        apply({ op: "duplicate_clip", clip_id: selClip, start_us: null });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Drag: ghost locally, one command on release.
  useEffect(() => {
    if (!drag) return;
    const move = (e: PointerEvent) => setDrag((d) => (d ? { ...d, dx: e.clientX - d.x0 } : d));
    const up = () => {
      const d = drag;
      setDrag(null);
      const deltaUs = Math.round((d.dx / pxPerSec) * 1e6);
      if (Math.abs(d.dx) < 2) return;
      if (d.kind === "move") apply({ op: "move_clip", clip_id: d.clip.id, start_us: Math.max(0, d.clip.start_us + deltaUs), to_layer_id: null });
      else apply({ op: "resize_clip", clip_id: d.clip.id, duration_us: Math.max(1, d.clip.duration_us + deltaUs) });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  });

  if (!view) {
    return (
      <section className="panel">
        <h2>タイムライン</h2>
        <div className="actions">
          <button className="primary" disabled={!showPath} onClick={open}>ショーを読み込んで編集</button>
          <button onClick={() => ask("new")}>新規ショー</button>
          <span className="hint">{showPath || "演奏タブでショーファイルを指定する"}</span>
        </div>
        {pending && (
          <UnsavedChoice
            action={pending === "open" ? "開く" : "新規を作る"}
            error={error}
            onSave={() => resolve("save")}
            onDiscard={() => resolve("discard")}
            onCancel={() => { setPending(null); setError(null); }}
          />
        )}
        {!pending && error && <p className="error">{error}</p>}
      </section>
    );
  }

  const compound = scope ? view.compounds.find((c) => c.id === scope) : undefined;
  if (scope && !compound) setScope(null);
  const layers = compound ? compound.layers : view.layers;
  const compoundOf = (id: string | null) => (id ? view.compounds.find((c) => c.id === id) : undefined);
  // Inside a compound the ruler is its own time (0 = its start) and there is no playhead.
  const playhead = compound ? null : playheadUs;
  const lengthUs = compound ? Math.max(compound.length_us, 4_000_000) + 2_000_000 : Math.max(view.length_us ?? 0, 60_000_000) + 10_000_000;
  const widthPx = (lengthUs / 1e6) * pxPerSec;
  const presetOf = (id: string) => view.presets.find((p) => p.id === id);
  // Clips and continuous layers whose preset flashes above this terminal's limit (at preview.bpm).
  const flashing = new Set(view.flash_warnings.flatMap((w) => (w.clips.length ? w.clips : [w.layer_id])));
  const tickEvery = pxPerSec >= 40 ? 1 : pxPerSec >= 12 ? 5 : 10;
  const ticks = Array.from({ length: Math.floor(lengthUs / 1e6 / tickEvery) + 1 }, (_, i) => i * tickEvery);
  // Display top layer first (array order = compositing order, last = front).
  const rows = [...layers].reverse();

  const applyForm = () => {
    if (!selected) return;
    const us = (v: string) => Math.round(Number(v) * 1e6);
    const vals = [form.start, form.duration, form.fadeIn, form.fadeOut].map(Number);
    if (vals.some((v) => !Number.isFinite(v) || v < 0)) {
      setError("秒の値は 0 以上の数で入力する");
      return;
    }
    apply({ op: "set_clip_timing", clip_id: selected.clip.id, start_us: us(form.start), duration_us: us(form.duration), fade_in_us: us(form.fadeIn), fade_out_us: us(form.fadeOut) });
  };

  const addClip = (layer: LayerDto) => {
    if (!presetId) return;
    const end = layer.clips.reduce((m, c) => Math.max(m, c.start_us + c.duration_us), 0);
    const at = playhead ?? end;
    apply({ op: "add_clip", layer_id: layer.id, preset_id: presetId, start_us: Math.max(0, Math.round(at)), duration_us: DEFAULT_CLIP_US });
  };

  return (
    <>
    {pending && (
      <UnsavedChoice
        action={pending === "open" ? "開く" : "新規を作る"}
        path={view.path || "新規ショー（未保存）"}
        error={error}
        onSave={() => resolve("save")}
        onDiscard={() => resolve("discard")}
        onCancel={() => { setPending(null); setError(null); }}
      />
    )}
    <MaterialBrowser key={view.path} view={view} apply={apply}
      onPick={(kind, id) => { if (kind === "preset") setPresetId(id); }}
      onEdit={(kind, id) => {
        if (kind === "preset") setPresetMode({ kind: "shared", presetId: id });
        else { setScope(id); setSelClip(null); setSelLayer(null); }
        requestAnimationFrame(() => document.getElementById(kind === "preset" ? "preset-editor" : "timeline-editor")?.scrollIntoView({ block: "start", behavior: "instant" }));
      }}
      targetName={layers.find(l => l.id === selLayer)?.name ?? null}
      canPlace={kind => !!layers.find(l => l.id === selLayer && (kind === "compound" ? !scope && l.kind === "timeline" : l.kind !== "continuous"))}
      onPlace={(kind, id) => {
        const target = layers.find(l => l.id === selLayer);
        if (!target) return;
        const end = target.clips.reduce((n, c) => Math.max(n, c.start_us + c.duration_us), 0);
        const start_us = Math.max(0, Math.round(playhead ?? end));
        if (kind === "compound") apply({ op: "add_compound_clip", layer_id: target.id, compound_id: id, start_us, duration_us: null });
        else apply({ op: "add_clip", layer_id: target.id, preset_id: id, start_us, duration_us: DEFAULT_CLIP_US });
      }}
      onImport={async (path, kind, id) => {
        if (busy.current) return null;
        busy.current = true;
        try {
          const v = await ipc.libraryImport(path, kind, id);
          accept(v);
          return v;
        } catch (e) {
          setError(ipc.errorText(e));
          return null;
        } finally {
          busy.current = false;
        }
      }}
    />
    <section id="timeline-editor" className="panel timeline-panel">
      <h2>タイムライン：{view.name}{view.path ? "" : "（新規・未保存）"}{compound ? ` ／ 素材「${compound.name}」の中` : ""}</h2>
      <div className="tl-toolbar">
        <button disabled={!view.can_undo} onClick={() => call(() => ipc.editorUndo())} title="Ctrl+Z">元に戻す{view.undo_label ? `（${view.undo_label}）` : ""}</button>
        <button disabled={!view.can_redo} onClick={() => call(() => ipc.editorRedo())} title="Ctrl+Y">やり直す{view.redo_label ? `（${view.redo_label}）` : ""}</button>
        <button onClick={() => ask("new")}>新規ショー</button>
        <label className="inline">
          スナップ
          <select value={snap} onChange={(e) => setSnap(e.target.value as Snap)}>
            <option value="none">なし</option>
            <option value="second">1 秒</option>
            <option value="frame30">フレーム（30 fps）</option>
          </select>
        </label>
        <label className="inline">
          追加するプリセット
          <select value={presetId} onChange={(e) => setPresetId(e.target.value)}>
            {view.presets.map((p) => (
              <option key={p.id} value={p.id}>{p.name}（{p.effect_type.replace("builtin.", "")}）</option>
            ))}
          </select>
        </label>
        <label className="inline">
          拡大
          <input type="range" min={4} max={120} value={pxPerSec} onChange={(e) => setPxPerSec(Number(e.target.value))} />
        </label>
        <label className="inline">
          編集対象
          <select value={scope ?? ""} onChange={(e) => { setScope(e.target.value || null); setSelClip(null); setSelLayer(null); }}>
            <option value="">ショー</option>
            {view.compounds.map((c) => <option key={c.id} value={c.id}>素材：{c.name}</option>)}
          </select>
        </label>
        {compound ? (
          <>
            <button onClick={() => apply({ op: "add_compound_layer", compound_id: compound.id, name: "", preset_id: null })} disabled={compound.layers.length >= 4}>素材にレイヤーを追加</button>
            <button disabled={!presetId || compound.layers.length >= 4} onClick={() => apply({ op: "add_compound_layer", compound_id: compound.id, name: "", preset_id: presetId })}>素材に常時レイヤーを追加</button>
          </>
        ) : (
          <>
            <button onClick={() => apply({ op: "add_layer", name: "", index: null })}>レイヤーを追加</button>
            <button disabled={!presetId} onClick={() => apply({ op: "add_continuous_layer", name: "", preset_id: presetId, index: null })}>常時レイヤーを追加</button>
            <button onClick={() => apply({ op: "add_triggered_layer", name: "", on: "press", index: null })} title="パッドなどで発火する演出のレイヤー。クリップが有効な区間">発火レイヤーを追加</button>
          </>
        )}
        <span className={view.dirty ? "badge warn" : "badge"}>{view.dirty ? "未保存の変更あり" : "保存済みと同じ"}</span>
        {view.runtime_revision !== null && <span className="badge good">Runtime に反映 rev {view.runtime_revision}</span>}
        <span className={view.flash_warnings.length ? "badge warn" : "badge"}>点滅 {view.flash_warnings.length} 件（{view.preview_bpm} bpm、上限 {view.max_flash_hz} Hz）</span>
      </div>

      {view.flash_warnings.length > 0 && (
        <ul className="issues">
          {view.flash_warnings.map((w) => (
            <li key={`${w.layer_id}-${w.preset_id}`}>⚠ {w.layer_name} / {w.preset_name}：{w.reason}{w.clips.length ? `（クリップ ${w.clips.join(", ")}）` : "（常時レイヤー）"}</li>
          ))}
        </ul>
      )}
      <div className="tl" style={{ ["--head" as string]: `${HEAD_W}px` }}>
        <div className="tl-scroll">
          <div className="tl-ruler" style={{ width: HEAD_W + widthPx }}>
            <div className="tl-head-spacer" style={{ width: HEAD_W }}>上ほど手前に合成</div>
            {ticks.map((t) => (
              <div key={t} className="tl-tick" style={{ left: HEAD_W + t * pxPerSec }}>{t % 60 === 0 ? `${t / 60}:00` : `${t}s`}</div>
            ))}
          </div>
          {rows.map((layer) => {
            const idx = layers.findIndex((l) => l.id === layer.id);
            return (
              <div key={layer.id} className={selLayer === layer.id ? "tl-row sel" : "tl-row"} style={{ width: HEAD_W + widthPx, height: ROW_H }} onPointerDown={() => { setSelLayer(layer.id); setSelClip(null); }}>
                <div className="tl-head" style={{ width: HEAD_W }}>
                  <div className="tl-name" title={layer.id}>{layer.name}</div>
                  <div className="tl-meta">{layer.kind === "continuous" ? `常時 · ${presetOf(layer.preset_id ?? "")?.name ?? ""}` : layer.kind === "triggered" ? `発火 · ${TRIGGER_JA[layer.trigger?.on ?? "press"]}` : `${layer.blend} · ${Math.round(layer.opacity * 100)}%`}</div>
                  <div className="tl-btns">
                    <button className="tiny" title="手前へ" disabled={idx === layers.length - 1} onClick={() => apply({ op: "move_layer", layer_id: layer.id, to_index: idx + 1 })}>▲</button>
                    <button className="tiny" title="奥へ" disabled={idx === 0} onClick={() => apply({ op: "move_layer", layer_id: layer.id, to_index: idx - 1 })}>▼</button>
                    {layer.kind !== "continuous" && <button className="tiny" title="クリップを追加（再生位置、なければ末尾）" onClick={() => addClip(layer)}>＋</button>}
                    <button className="tiny danger" title="レイヤーを削除" onClick={() => apply({ op: "delete_layer", layer_id: layer.id })}>×</button>
                  </div>
                </div>
                <div className="tl-track">
                  {layer.kind === "continuous" && <div className={flashing.has(layer.id) ? "tl-continuous flash" : "tl-continuous"} style={{ background: presetOf(layer.preset_id ?? "")?.color }} />}
                  {layer.clips.map((c) => {
                    const p = presetOf(c.preset_id);
                    const cp = compoundOf(c.compound_id);
                    const rec = c.recording ? ` · 記録 ${c.recording.events.length}件${c.looping ? " ↻" : ""}` : "";
                    const label = cp ? `素材：${cp.name}${c.time_scale && c.time_scale !== 1 ? ` ×${c.time_scale}` : ""}${c.looping ? " ↻" : ""}` : `${p?.name ?? c.preset_id}${rec}`;
                    const d = drag && drag.clip.id === c.id ? drag : null;
                    const startUs = d?.kind === "move" ? Math.max(0, c.start_us + (d.dx / pxPerSec) * 1e6) : c.start_us;
                    const durUs = d?.kind === "resize" ? Math.max(1, c.duration_us + (d.dx / pxPerSec) * 1e6) : c.duration_us;
                    const w = (durUs / 1e6) * pxPerSec;
                    return (
                      <div
                        key={c.id}
                        className={`tl-clip${selClip === c.id ? " sel" : ""}${flashing.has(c.id) ? " flash" : ""}`}
                        style={{ left: (startUs / 1e6) * pxPerSec, width: Math.max(w, 4), ["--c" as string]: cp ? c.tint ?? "#9A8CFF" : p?.color ?? "#888" }}
                        title={`${c.id} · ${label} · ${sec(c.start_us)}s + ${sec(c.duration_us)}s`}
                        onPointerDown={(e) => {
                          e.stopPropagation();
                          setSelClip(c.id);
                          setSelLayer(layer.id);
                          setDrag({ kind: "move", clip: c, layerId: layer.id, x0: e.clientX, dx: 0 });
                        }}
                      >
                        <div className="fade in" style={{ width: (c.fade_in_us / durUs) * 100 + "%" }} />
                        <div className="fade out" style={{ width: (c.fade_out_us / durUs) * 100 + "%" }} />
                        <span className="tl-clip-label">{label}</span>
                        <div
                          className="tl-resize"
                          onPointerDown={(e) => {
                            e.stopPropagation();
                            setSelClip(c.id);
                            setDrag({ kind: "resize", clip: c, layerId: layer.id, x0: e.clientX, dx: 0 });
                          }}
                        />
                      </div>
                    );
                  })}
                </div>
              </div>
            );
          })}
          {playhead !== null && <div className="tl-playhead" style={{ left: HEAD_W + (playhead / 1e6) * pxPerSec }} />}
        </div>
      </div>

      {selected ? (
        <div className="inspector">
          <div className="insp-title">
            クリップ <span className="mono">{selected.clip.id}</span> · {selected.clip.compound_id ? `素材：${compoundOf(selected.clip.compound_id)?.name ?? selected.clip.compound_id}` : presetOf(selected.clip.preset_id)?.name}
          </div>
          <div className="insp-grid">
            <NumField label="開始（秒）" value={form.start} onChange={(v) => setForm({ ...form, start: v })} hint={`${frames30(selected.clip.start_us)} フレーム`} />
            <NumField label="長さ（秒）" value={form.duration} onChange={(v) => setForm({ ...form, duration: v })} hint={`${frames30(selected.clip.duration_us)} フレーム`} />
            <NumField label="フェードイン（秒）" value={form.fadeIn} onChange={(v) => setForm({ ...form, fadeIn: v })} />
            <NumField label="フェードアウト（秒）" value={form.fadeOut} onChange={(v) => setForm({ ...form, fadeOut: v })} />
            {selected.clip.compound_id ? (
              <CompoundClipFields clip={selected.clip} compound={compoundOf(selected.clip.compound_id)} apply={apply} />
            ) : (
              <label className="field">
                <span>プリセット</span>
                <select value={selected.clip.preset_id} onChange={(e) => apply({ op: "set_clip_preset", clip_id: selected.clip.id, preset_id: e.target.value })}>
                  {view.presets.map((p) => (
                    <option key={p.id} value={p.id}>{p.name}</option>
                  ))}
                </select>
              </label>
            )}
            <label className="field">
              <span>レイヤー</span>
              <select value={selected.layer.id} onChange={(e) => apply({ op: "move_clip", clip_id: selected.clip.id, start_us: selected.clip.start_us, to_layer_id: e.target.value })}>
                {(compound && compound.layers.some((l) => l.id === selected.layer.id) ? compound.layers : view.layers).filter((l) => l.kind === selected.layer.kind).map((l) => (
                  <option key={l.id} value={l.id}>{l.name}</option>
                ))}
              </select>
            </label>
          </div>
          {selected.clip.recording && <RecordingLength clipId={selected.clip.id} recording={selected.clip.recording} apply={apply} />}
          <div className="actions">
            <button className="primary" onClick={applyForm}>数値を適用</button>
            {!selected.clip.compound_id && <button onClick={() => setPresetMode({ kind: "clip", clipId: selected.clip.id, presetId: selected.clip.preset_id })}>このクリップだけ効果を編集</button>}
            {selected.clip.recording && (
              <>
                <label className="inline">
                  <input type="checkbox" checked={selected.clip.looping ?? false} onChange={(e) => apply({ op: "set_clip_recording", clip_id: selected.clip.id, recording: selected.clip.recording, looping: e.target.checked })} />
                  記録を繰り返す（1周 {((selected.clip.recording.length_us ?? Math.max(1, ...selected.clip.recording.events.map((ev) => ev.t_us + (ev.duration_us ?? 0)))) / 1e6).toFixed(2)} s）
                </label>
                <button onClick={() => apply({ op: "set_clip_recording", clip_id: selected.clip.id, recording: { ...selected.clip.recording!, events: [] }, looping: null })}>記録をクリア（{selected.clip.recording.events.length}件）</button>
                <button className="danger" onClick={() => apply({ op: "set_clip_recording", clip_id: selected.clip.id, recording: null, looping: null })}>記録を削除</button>
              </>
            )}
            <button onClick={() => apply({ op: "duplicate_clip", clip_id: selected.clip.id, start_us: null })} title="Ctrl+D">複製</button>
            <button className="danger" onClick={() => { apply({ op: "delete_clip", clip_id: selected.clip.id }); setSelClip(null); }} title="Delete">削除</button>
            <span className="hint">ドラッグで移動、右端で長さ。数値の開始・長さもスナップに合わせる。</span>
          </div>
        </div>
      ) : selLayer && layers.find((l) => l.id === selLayer) ? (
        <LayerInspector view={view} layer={layers.find((l) => l.id === selLayer)!} apply={apply} />
      ) : (
        <p className="hint">クリップを選ぶと数値で編集できる。レイヤーの行を選ぶとレイヤーの設定。見出しの＋でクリップを追加（再生位置、なければ末尾に4秒）。</p>
      )}
      {error && <p className="error">{error}</p>}
      <p className="hint">編集は Rust 側で検証してから反映する（不正な編集は何も変えない）。Runtime が同じショーを追従中なら、通った版を次の frame 境界で差し替える。{view.path}</p>
    </section>
    <CompoundPanel view={view} scope={scope} setScope={(id) => { setScope(id); setSelClip(null); setSelLayer(null); }} selLayer={selLayer} playheadUs={playheadUs} apply={apply} />
    <ShowSyncPanel view={view} apply={apply} />
    <PresetPanel view={view} mode={presetMode} setMode={setPresetMode} apply={apply} applyView={(f) => call(f)} />
    </>
  );
}
