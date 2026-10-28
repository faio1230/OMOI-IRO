// 0.1.3 stage 6: compounds (several layers and their timing as one
// material). The list here creates, renames, sizes, places and deletes them;
// their inner layers are edited on the timeline after switching its target
// to the compound. Every change is an EditCommand validated in Rust.
import { useEffect, useState } from "react";
import type { ClipDto, CompoundDto, EditCommand, EditorView } from "./ipc/types";

const sec = (us: number) => (us / 1e6).toFixed(3);

/** Name and own length of one compound; applied when a field is left. */
function CompoundRow(props: { c: CompoundDto; editing: boolean; canPlace: boolean; onEdit: () => void; onPlace: () => void; apply: (c: EditCommand) => void }) {
  const { c, apply } = props;
  const [name, setName] = useState(c.name);
  const [len, setLen] = useState(c.duration_us === null ? "" : sec(c.duration_us));
  useEffect(() => setName(c.name), [c.name]);
  useEffect(() => setLen(c.duration_us === null ? "" : sec(c.duration_us)), [c.duration_us]);
  const commitLen = () => {
    const v = len.trim() === "" ? 0 : Math.round(Number(len) * 1e6);
    if (!Number.isFinite(v) || v < 0) return setLen(c.duration_us === null ? "" : sec(c.duration_us));
    if (v !== (c.duration_us ?? 0)) apply({ op: "set_compound", compound_id: c.id, name: null, duration_us: v });
  };
  return (
    <tr className={props.editing ? "sel" : undefined}>
      <td><input value={name} onChange={(e) => setName(e.target.value)} onBlur={() => name !== c.name && apply({ op: "set_compound", compound_id: c.id, name, duration_us: null })} /></td>
      <td>
        <input value={len} placeholder={`自動 ${sec(c.length_us)}`} inputMode="decimal" onChange={(e) => setLen(e.target.value)} onBlur={commitLen} onKeyDown={(e) => e.key === "Enter" && commitLen()} />
      </td>
      <td>{c.layers.length}</td>
      <td>{c.used_by_clips.length ? c.used_by_clips.join(", ") : "—"}</td>
      <td className="actions">
        <button className="small" onClick={props.onEdit} disabled={props.editing}>中身を編集</button>
        <button className="small" onClick={props.onPlace} disabled={!props.canPlace} title="選択中のタイムラインレイヤーの再生位置（なければ末尾）に置く">置く</button>
        <button className="small danger" disabled={c.used_by_clips.length > 0} title={c.used_by_clips.length ? "使用中は削除できない" : "削除"} onClick={() => apply({ op: "delete_compound", compound_id: c.id })}>削除</button>
      </td>
    </tr>
  );
}

/** The compound list. `scope` is the compound whose layers the timeline shows (null = the show). */
export function CompoundPanel(props: { view: EditorView; scope: string | null; setScope: (id: string | null) => void; selLayer: string | null; playheadUs: number | null; apply: (c: EditCommand) => void }) {
  const { view, scope, apply } = props;
  const target = view.layers.find((l) => l.id === props.selLayer && l.kind === "timeline");
  const place = (c: CompoundDto) => {
    if (!target) return;
    const end = target.clips.reduce((m, x) => Math.max(m, x.start_us + x.duration_us), 0);
    apply({ op: "add_compound_clip", layer_id: target.id, compound_id: c.id, start_us: Math.max(0, Math.round(props.playheadUs ?? end)), duration_us: null });
  };
  return (
    <section className="panel">
      <h2>素材（複合）</h2>
      <div className="actions">
        <button onClick={() => apply({ op: "add_compound", name: "" })}>新しい素材</button>
        {scope && <button className="primary" onClick={() => props.setScope(null)}>ショーの編集に戻る</button>}
        <span className="hint">
          {target ? `「置く」は ${target.name} へ` : "置くには、ショーのタイムラインレイヤーを選ぶ"}。中のレイヤーは4枚まで（タイムラインか常時）。素材を直すと、置いた所すべてが変わる。
        </span>
      </div>
      {view.compounds.length > 0 && (
        <table className="compounds">
          <thead>
            <tr><th>名前</th><th>長さ（秒、空で自動）</th><th>層</th><th>置いたクリップ</th><th /></tr>
          </thead>
          <tbody>
            {view.compounds.map((c) => (
              <CompoundRow key={c.id} c={c} editing={scope === c.id} canPlace={!!target && !scope} onEdit={() => props.setScope(c.id)} onPlace={() => place(c)} apply={apply} />
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

/** Tint, time scale, loop and detach of a compound clip. */
export function CompoundClipFields(props: { clip: ClipDto; compound: CompoundDto | undefined; apply: (c: EditCommand) => void }) {
  const { clip, apply } = props;
  const set = (patch: { tint?: string | null; time_scale?: number | null; looping?: boolean | null }) =>
    apply({ op: "set_compound_clip", clip_id: clip.id, tint: patch.tint === undefined ? null : patch.tint ?? "", time_scale: patch.time_scale ?? null, looping: patch.looping ?? null });
  const scale = clip.time_scale ?? 1;
  const longer = props.compound && clip.duration_us * scale > props.compound.length_us;
  return (
    <>
      <label className="field check">
        <input type="checkbox" checked={clip.tint !== null} onChange={(e) => set({ tint: e.target.checked ? "#FF4080" : null })} />
        <span>色を差し替える（中の色をこの色に揃える）</span>
      </label>
      {clip.tint !== null && (
        <label className="field">
          <span>差し替える色</span>
          <div className="color-row">
            <input type="color" value={clip.tint} onChange={(e) => set({ tint: e.target.value.toUpperCase() })} />
            <span className="mono">{clip.tint}</span>
          </div>
          <span className="hint">主の色がこの色に、他の色は元の明るさの比で揃う</span>
        </label>
      )}
      <label className="field">
        <span>速さ（倍）</span>
        <input type="number" min={0.25} max={4} step={0.25} value={scale} onChange={(e) => set({ time_scale: Math.min(4, Math.max(0.25, Number(e.target.value))) })} />
        <span className="hint">0.25〜4。拍に合わせる効果は曲の拍のまま</span>
      </label>
      <label className="field check">
        <input type="checkbox" checked={clip.looping ?? false} onChange={(e) => set({ looping: e.target.checked })} />
        <span>素材の長さで繰り返す（切ると素材の終わり以降は何も出ない）</span>
      </label>
      {longer && !clip.looping && <p className="hint">このクリップは素材より長い：繰り返さない場合、後半は何も出ない。</p>}
      <div className="actions">
        <button disabled={scale !== 1} title={scale !== 1 ? "速さが 1 以外は展開できない（効果そのものの速さは縮尺できないため）" : "中のレイヤーを、このレイヤーの上に独立したレイヤーとして展開する"} onClick={() => apply({ op: "detach_compound_clip", clip_id: clip.id })}>
          展開して独立
        </button>
      </div>
    </>
  );
}
