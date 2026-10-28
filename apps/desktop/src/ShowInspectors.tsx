// U05: layer properties, preset library and effect parameters. Every change
// is an EditCommand applied and validated in Rust (same history and runtime
// hand-off as the timeline); ranges shown here follow the show schema.
import { useEffect, useState, type ReactElement } from "react";
import * as ipc from "./ipc/commands";
import type { BeatSource, Blend, EditCommand, EditorView, Effect, EffectType, LayerDto, OnConflict, PresetDto, RegionPattern, PathKind, Mirror, ShapeKind, LayerMask, ParticleMode, TextMode, PadsMode, TriggerOn, TriggerSpec, K, Keyframe, Ease } from "./ipc/types";

const TYPE_JA: Record<EffectType, string> = {
  "builtin.solid": "単色（Solid）",
  "builtin.chase": "流れ（Chase）",
  "builtin.pulse": "拍の明滅（Pulse）",
  "builtin.strobe": "点滅（Strobe）",
  "builtin.noise": "ノイズ（Noise）",
  "builtin.ripple": "波紋（Ripple）",
  "builtin.rain": "雨（Rain）",
  "builtin.wave": "波形（Wave）",
  "builtin.orb": "球（Orb）",
  "builtin.regions": "領域ステップ（Regions）",
  "builtin.tap_glow": "余熱（TapGlow）",
  "builtin.cross": "十字（Cross）",
  "builtin.path": "経路（Path）",
  "builtin.shape": "図形（Shape）",
  "builtin.particles": "粒子（Particles）",
  "builtin.frames": "フレーム列（Frames）",
  "builtin.text": "文字（Text）",
  "builtin.pads": "パッドの状態（Pads）",
  "builtin.cycle": "叩くたびに色（Cycle）",
  "builtin.constellation": "星座（Constellation）",
};

const isKeyed = <T,>(v: K<T>): v is { keyframes: Keyframe<T>[] } => typeof v === "object" && v !== null && "keyframes" in v;
/** The value in force at the start (a constant, or the first keyframe). */
export const first = <T,>(v: K<T>): T => (isKeyed(v) ? v.keyframes[0].v : v);

const EASE_JA: Record<Ease, string> = { linear: "直線", step: "段", ease_in: "ゆっくり始まる", ease_out: "ゆっくり終わる", ease_in_out: "両端ゆっくり" };

/** "時間変化" toggle and the keyframe table (time in seconds, value, ease). */
function Keyed<T>(props: { value: K<T>; onChange: (v: K<T>) => void; editor: (v: T, set: (v: T) => void) => ReactElement; children: ReactElement }) {
  const { value, onChange } = props;
  if (!isKeyed(value)) {
    return (
      <>
        {props.children}
        <label className="check tiny-check">
          <input type="checkbox" checked={false} onChange={() => onChange({ keyframes: [{ t_us: 0, v: value, ease: "linear" }] })} />
          <span>時間変化</span>
        </label>
      </>
    );
  }
  const k = value.keyframes;
  const put = (i: number, patch: Partial<Keyframe<T>>) => onChange({ keyframes: k.map((x, j) => (j === i ? { ...x, ...patch } : x)) });
  const sorted = (rows: Keyframe<T>[]) => [...rows].sort((a, b) => a.t_us - b.t_us);
  return (
    <div className="keyframes">
      <table>
        <thead>
          <tr><th>時刻（秒）</th><th>値</th><th>次への補間</th><th /></tr>
        </thead>
        <tbody>
          {k.map((x, i) => (
            <tr key={i}>
              <td><input type="number" min={0} step={0.1} value={x.t_us / 1e6} onChange={(e) => onChange({ keyframes: sorted(k.map((y, j) => (j === i ? { ...y, t_us: Math.max(0, Math.round(Number(e.target.value) * 1e6)) } : y))) })} /></td>
              <td>{props.editor(x.v, (v) => put(i, { v }))}</td>
              <td>
                <select value={x.ease ?? "linear"} onChange={(e) => put(i, { ease: e.target.value as Ease })}>
                  {Object.entries(EASE_JA).map(([e, l]) => <option key={e} value={e}>{l}</option>)}
                </select>
              </td>
              <td><button className="tiny danger" disabled={k.length === 1} onClick={() => onChange({ keyframes: k.filter((_, j) => j !== i) })}>×</button></td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="actions">
        <button className="small" disabled={k.length >= 64} onClick={() => onChange({ keyframes: sorted([...k, { ...k[k.length - 1], t_us: k[k.length - 1].t_us + 1_000_000 }]) })}>点を追加（1秒後）</button>
        <button className="small" onClick={() => window.confirm("時間変化をやめて、最初の値だけを残しますか？") && onChange(k[0].v)}>時間変化をやめる</button>
        <span className="hint">時刻はクリップの中の時刻（常時レイヤーは曲の時刻、パッドで出る演出は発火からの時間）</span>
      </div>
    </div>
  );
}

/** Numeric input clamped to [min, max]; `scale` shows value / scale (e.g. µs as s); `integer` rounds; `fixed` has no time change (not an effect parameter). */
function NumberField(props: { label: string; value: K<number>; min: number; max: number; step: number; hint?: string; integer?: boolean; scale?: number; fixed?: boolean; dim?: boolean; onChange: (v: K<number>) => void }) {
  const scale = props.scale ?? 1;
  const input = (v: number, set: (v: number) => void) => (
    <input type="number" min={props.min} max={props.max} step={props.step} value={v / scale} onChange={(e) => {
      const x = Math.min(props.max, Math.max(props.min, Number(e.target.value))) * scale;
      set(props.integer ? Math.round(x) : x);
    }} />
  );
  return (
    <label className={props.dim ? "field dim" : "field"}>
      <span>{props.label}</span>
      {props.fixed ? input(first(props.value), props.onChange) : <Keyed value={props.value} onChange={props.onChange} editor={input}>{input(first(props.value), props.onChange)}</Keyed>}
      {props.hint && <span className="hint">{props.hint}</span>}
    </label>
  );
}

/** Slider (0..1 style values); `percent` shows the value as a percentage in the label. */
function SliderField(props: { label: string; value: K<number>; min: number; max: number; step: number; hint?: string; percent?: boolean; onChange: (v: K<number>) => void }) {
  const slider = (v: number, set: (v: number) => void) => <input type="range" min={props.min} max={props.max} step={props.step} value={v} onChange={(e) => set(Number(e.target.value))} />;
  const v0 = first(props.value);
  return (
    <label className="field">
      <span>{props.label}{props.percent && !isKeyed(props.value) ? ` ${Math.round(v0 * 100)}%` : ""}</span>
      <Keyed value={props.value} onChange={props.onChange} editor={(v, set) => <input type="number" min={props.min} max={props.max} step={props.step} value={v} onChange={(e) => set(Math.min(props.max, Math.max(props.min, Number(e.target.value))))} />}>{slider(v0, props.onChange)}</Keyed>
      {props.hint && <span className="hint">{props.hint}</span>}
    </label>
  );
}

function ColorField({ value, onChange, label = "色（#RRGGBB）" }: { value: K<string>; onChange: (v: K<string>) => void; label?: string }) {
  const editor = (v: string, set: (v: string) => void) => (
    <div className="color-row">
      <input type="color" value={/^#[0-9a-fA-F]{6}$/.test(v) ? v : "#000000"} onChange={(e) => set(e.target.value.toUpperCase())} />
      <input value={v} onChange={(e) => set(e.target.value)} spellCheck={false} />
    </div>
  );
  return (
    <label className="field">
      <span>{label}</span>
      <Keyed value={value} onChange={onChange} editor={editor}>{editor(first(value), onChange)}</Keyed>
    </label>
  );
}
/** Text list edited as a string and parsed on blur; an unparsable entry keeps the previous value and shows the error. */
function ListField<T>(props: { label: string; value: T; format: (v: T) => string; parse: (s: string) => T | string; hint?: string; onChange: (v: T) => void }) {
  const [text, setText] = useState(props.format(props.value));
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => setText(props.format(props.value)), [props.value]); // eslint-disable-line react-hooks/exhaustive-deps
  const commit = () => {
    const r = props.parse(text);
    if (typeof r === "string") setErr(r);
    else { setErr(null); props.onChange(r); }
  };
  return (
    <label className="field wide">
      <span>{props.label}</span>
      <input value={text} spellCheck={false} onChange={(e) => setText(e.target.value)} onBlur={commit} onKeyDown={(e) => e.key === "Enter" && commit()} />
      {err ? <span className="hint error">{err}</span> : props.hint && <span className="hint">{props.hint}</span>}
    </label>
  );
}

const nums = (s: string) => s.split(/[\s,]+/).filter(Boolean).map(Number);
const parsePoints = (s: string): [number, number][] | string => {
  const pts = s.split(";").map((x) => nums(x)).filter((x) => x.length > 0);
  if (pts.some((x) => x.length !== 2 || x.some((v) => !(v >= 0 && v <= 7)))) return "各点は「x,y」（0〜7）を ; で区切る";
  if (pts.length < 2 || pts.length > 32) return "点は 2〜32 個";
  return pts as [number, number][];
};
const parseGroups = (s: string): number[][] | string => {
  const gs = s.split(";").map(nums).filter((g) => g.length > 0);
  if (gs.some((g) => g.some((v) => !Number.isInteger(v) || v < 0 || v > 63))) return "セル番号は 0〜63 の整数";
  if (gs.length < 1 || gs.length > 32 || gs.some((g) => g.length > 64)) return "グループは 1〜32 個（各 64 セルまで）";
  return gs;
};
const parseOffsets = (n: number) => (s: string): number[] | undefined | string => {
  const v = nums(s);
  if (v.length === 0) return undefined;
  if (v.length !== n || v.some((x) => !(x >= 0 && x <= 1))) return `ずれは 0〜1 をグループ数（${n}）だけ`;
  return v;
};

/** 8 × 8 editor of the frames (click a cell to switch it between white and black). */
function FramePainter({ frames, onChange }: { frames: string[]; onChange: (f: string[]) => void }) {
  const [k, setK] = useState(0);
  const i = Math.min(k, frames.length - 1);
  const f = frames[i];
  const cell = (c: number) => f.slice(c * 6, c * 6 + 6);
  const toggle = (c: number) => {
    const next = f.slice(0, c * 6) + (cell(c) === "000000" ? "FFFFFF" : "000000") + f.slice(c * 6 + 6);
    onChange(frames.map((x, j) => (j === i ? next : x)));
  };
  return (
    <div className="field wide frame-painter">
      <span>フレーム {i + 1} / {frames.length}（上の行から。クリックで白と黒を切り替え）</span>
      <div className="frame-grid">
        {Array.from({ length: 64 }, (_, c) => (
          <button key={c} className="frame-cell" style={{ background: `#${cell(c)}` }} onClick={() => toggle(c)} aria-label={`行 ${Math.floor(c / 8) + 1} 列 ${(c % 8) + 1}`} />
        ))}
      </div>
      <div className="actions">
        <button className="small" disabled={i === 0} onClick={() => setK(i - 1)}>前</button>
        <button className="small" disabled={i === frames.length - 1} onClick={() => setK(i + 1)}>次</button>
        <button className="small" disabled={frames.length >= 240} onClick={() => { onChange([...frames.slice(0, i + 1), f, ...frames.slice(i + 1)]); setK(i + 1); }}>このフレームを複製</button>
        <button className="small danger" disabled={frames.length === 1} onClick={() => { onChange(frames.filter((_, j) => j !== i)); setK(Math.max(0, i - 1)); }}>削除</button>
      </div>
    </div>
  );
}

/** Parameter form for one effect. Ranges and units follow schemas/show.schema.json. */
export function EffectForm({ effect, onChange }: { effect: Effect; onChange: (e: Effect) => void }) {
  if (effect.type === "builtin.solid") {
    return (
      <div className="insp-grid">
        <ColorField value={effect.params.color} onChange={(color) => onChange({ ...effect, params: { ...effect.params, color } })} />
      </div>
    );
  }
  if (effect.type === "builtin.chase") {
    const p = effect.params;
    return (
      <div className="insp-grid">
        <ColorField value={p.color} onChange={(color) => onChange({ ...effect, params: { ...p, color } })} />
        <NumberField label="周期（秒）" value={p.period_us} min={0.001} max={3600} step={0.1} scale={1e6} integer hint="0.001〜3600 秒（1周する時間）" onChange={(period_us) => onChange({ ...effect, params: { ...p, period_us } })} />
        <NumberField label="帯の幅（LED）" value={p.width} min={1} max={8} step={1} integer hint="1〜8（時間変化は段だけ）" onChange={(width) => onChange({ ...effect, params: { ...p, width } })} />
        <label className="field">
          <span>方向</span>
          <select value={p.axis} onChange={(e) => onChange({ ...effect, params: { ...p, axis: e.target.value as "x" | "y" } })}>
            <option value="x">横（x）</option>
            <option value="y">縦（y）</option>
          </select>
        </label>
        <label className="field check">
          <input type="checkbox" checked={p.reverse ?? false} onChange={(e) => onChange({ ...effect, params: { ...p, reverse: e.target.checked } })} />
          <span>逆向き（横は右→左、縦は上→下）</span>
        </label>
      </div>
    );
  }
  if (effect.type === "builtin.strobe") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <label className="field">
          <span>速さの決め方</span>
          <select value={p.mode} onChange={(e) => set({ mode: e.target.value as "hz" | "beat" })}>
            <option value="beat">拍に合わせる</option>
            <option value="hz">回数（Hz）で決める</option>
          </select>
          <span className="hint">拍に合わせる場合、拍が不明な間は消える</span>
        </label>
        {p.mode === "hz" ? (
          <NumberField label="1秒あたりの回数（Hz）" value={p.rate_hz} min={0.5} max={30} step={0.5} hint="0.5〜30。端末の点滅上限（既定 3 Hz）を超えると警告" onChange={(rate_hz) => set({ rate_hz })} />
        ) : (
          <label className="field">
            <span>1回の拍数</span>
            <select value={String(first(p.beats_per_cycle))} onChange={(e) => set({ beats_per_cycle: Number(e.target.value) })}>
              <option value="0.25">0.25 拍（16分）</option>
              <option value="0.5">0.5 拍（8分）</option>
              <option value="1">1 拍（4分）</option>
              <option value="2">2 拍（2分）</option>
            </select>
          </label>
        )}
        <SliderField label="点灯の割合" value={p.duty} min={0.05} max={0.95} step={0.05} percent hint="0.05〜0.95" onChange={(v) => onChange({ ...effect, params: { ...p, duty: v } } as Effect)} />
        <SliderField label="位相のずらし" value={p.phase_offset} min={0} max={1} step={0.05} percent hint="0〜1 周期" onChange={(v) => onChange({ ...effect, params: { ...p, phase_offset: v } } as Effect)} />
      </div>
    );
  }
  if (effect.type === "builtin.tap_glow") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <ColorField label="押した直後の色" value={p.color_start} onChange={(color_start) => set({ color_start })} />
        <ColorField label="途中の色" value={p.color_mid} onChange={(color_mid) => set({ color_mid })} />
        <ColorField label="消える前の色" value={p.color_end} onChange={(color_end) => set({ color_end })} />
        <NumberField label="消えるまで（ms）" value={p.lifetime_ms} min={100} max={5000} step={100} hint="100〜5000" integer onChange={(lifetime_ms) => set({ lifetime_ms })} />
        <NumberField label="広がり（セル）" value={p.radius} min={0} max={3} step={0.25} hint="0 で押したパッドだけ" onChange={(radius) => set({ radius })} />
        <NumberField label="繰り返し回数" value={p.repeats} min={1} max={8} step={1} hint="1〜8" integer onChange={(repeats) => set({ repeats })} />
        <NumberField label="繰り返しの間隔（ms）" value={p.repeat_interval_ms} min={50} max={2000} step={50} hint="50〜2000" integer onChange={(repeat_interval_ms) => set({ repeat_interval_ms })} />
        <SliderField label="繰り返すたびの明るさ" value={p.repeat_decay} min={0} max={1} step={0.05} percent onChange={(v) => onChange({ ...effect, params: { ...p, repeat_decay: v } } as Effect)} />
        <label className="field check">
          <input type="checkbox" checked={p.charge ?? false} onChange={(e) => set({ charge: e.target.checked || undefined })} />
          <span>長押しチャージ（押している間に広がり、離すと光る。発火レイヤーの「押している間」と使う）</span>
        </label>
        <p className="hint">パッドを押した位置が光って冷めていく。操作入力がないと何も出ない（プレビュー盤のクリックでも確認できる）。{p.charge ? "チャージは入力履歴に依存（巻き戻しでは再現しない）。" : ""}</p>
      </div>
    );
  }
  if (effect.type === "builtin.cross") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <NumberField label="走る速さ（セル/秒）" value={p.speed} min={1} max={32} step={1} hint="1〜32" onChange={(speed) => set({ speed })} />
        <NumberField label="尾の長さ（セル）" value={p.length} min={0.5} max={8} step={0.5} hint="0.5〜8" onChange={(length) => set({ length })} />
        <NumberField label="消えるまで（ms）" value={p.lifetime_ms} min={100} max={3000} step={100} hint="100〜3000" integer onChange={(lifetime_ms) => set({ lifetime_ms })} />
        <p className="hint">押した位置から上下左右へ光が走る。操作入力がないと何も出ない（プレビュー盤のクリックでも確認できる）。</p>
      </div>
    );
  }
  if (effect.type === "builtin.path") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    const cell = (label: string, key: "from_x" | "from_y" | "to_x" | "to_y" | "center_x" | "center_y", dflt: number) => (
      <NumberField label={label} value={p[key] ?? dflt} min={0} max={7} step={0.5} hint="0〜7" onChange={(v) => set({ [key]: v })} />
    );
    const closed = p.path === "perimeter" || p.path === "orbit" || p.path === "figure8";
    const setPath = (path: PathKind) => {
      const patch: Partial<typeof p> = { path };
      if (path === "line") Object.assign(patch, { from_x: p.from_x ?? 0, from_y: p.from_y ?? 0, to_x: p.to_x ?? 7, to_y: p.to_y ?? 7 });
      if (path === "points" && !p.points) patch.points = [[0, 0], [7, 0], [7, 7]] as [number, number][];
      if (path === "orbit" || path === "figure8") Object.assign(patch, { center_x: p.center_x ?? 3.5, center_y: p.center_y ?? 3.5, orbit_radius: p.orbit_radius ?? 3 });
      const isClosed = path !== "line" && path !== "points";
      if (isClosed !== closed) patch.speed = isClosed ? 0.25 : 4;
      set(patch);
    };
    return (
      <div className="insp-grid">
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <label className="field">
          <span>経路</span>
          <select value={p.path} onChange={(e) => setPath(e.target.value as PathKind)}>
            <option value="line">直線（始点→終点）</option>
            <option value="perimeter">外周（左下から時計回り）</option>
            <option value="orbit">円</option>
            <option value="figure8">8の字</option>
            <option value="points">点を順にたどる</option>
          </select>
        </label>
        {p.path === "line" && (<>{cell("始点 x", "from_x", 0)}{cell("始点 y", "from_y", 0)}{cell("終点 x", "to_x", 7)}{cell("終点 y", "to_y", 7)}</>)}
        {(p.path === "orbit" || p.path === "figure8") && (
          <>
            {cell("中心 x", "center_x", 3.5)}
            {cell("中心 y", "center_y", 3.5)}
            <NumberField label={p.path === "orbit" ? "半径（セル）" : "横の半幅（セル）"} value={p.orbit_radius ?? 3} min={0} max={4} step={0.25} hint="0〜4" onChange={(orbit_radius) => set({ orbit_radius })} />
          </>
        )}
        {p.path === "points" && (
          <ListField<[number, number][]> label="通る点" value={p.points ?? [[0, 0], [7, 7]]} format={(ps) => ps.map(([x, y]) => `${x},${y}`).join("; ")} parse={parsePoints} hint="「x,y」を ; で区切る（左下が 0,0）。2〜32 点" onChange={(points) => set({ points })} />
        )}
        <NumberField label={closed ? "速さ（周/秒）" : "速さ（セル/秒）"} value={p.speed} min={closed ? 0.05 : 0.5} max={closed ? 4 : 16} step={closed ? 0.05 : 0.5} hint={closed ? "0.05〜4 周/秒" : "0.5〜16 セル/秒"} onChange={(speed) => set({ speed })} />
        <NumberField label="光の大きさ（セル）" value={p.size} min={0.5} max={3} step={0.25} hint="0.5〜3" onChange={(size) => set({ size })} />
        <NumberField label="尾の長さ（セル）" value={p.tail} min={0} max={8} step={0.5} hint="0〜8" onChange={(tail) => set({ tail })} />
        <SliderField label="開始位置のずらし" value={p.phase} min={0} max={1} step={0.05} percent hint="経路の長さに対する割合" onChange={(v) => onChange({ ...effect, params: { ...p, phase: v } } as Effect)} />
        <label className="field">
          <span>鏡写し</span>
          <select value={p.mirror} onChange={(e) => set({ mirror: e.target.value as Mirror })}>
            <option value="none">なし</option>
            <option value="x">左右に写す</option>
            <option value="y">上下に写す</option>
            <option value="xy">上下左右（4つ）</option>
          </select>
        </label>
        {!closed && (
          <label className="field check">
            <input type="checkbox" checked={p.loop} onChange={(e) => set({ loop: e.target.checked })} />
            <span>終点で折り返して往復する（切ると終点で止まる）</span>
          </label>
        )}
      </div>
    );
  }
  if (effect.type === "builtin.pads") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    const color = (label: string, key: "color_low" | "color_high" | "color", d: string) => (
      <label className="field">
        <span>{label}</span>
        <input type="color" value={p[key] ?? d} onChange={(e) => set({ [key]: e.target.value.toUpperCase() })} />
      </label>
    );
    return (
      <div className="insp-grid">
        <label className="field">
          <span>状態</span>
          <select value={p.mode} onChange={(e) => set({ mode: e.target.value as PadsMode })}>
            <option value="heat">叩くほど溜まる（heat）</option>
            <option value="plant">押した所が育って消える（plant）</option>
            <option value="paint">押した所に色を置く（paint）</option>
          </select>
        </label>
        {p.mode === "heat" && (
          <>
            {color("冷えた色", "color_low", "#200040")}
            {color("熱い色", "color_high", "#FF4020")}
            <NumberField fixed label="1回で溜まる量" value={p.gain ?? 0.25} min={0.05} max={1} step={0.05} hint="0.05〜1（合計は 1 まで）" onChange={(v) => set({ gain: first(v) })} />
            <NumberField fixed label="冷めるまで（ms）" value={p.decay_ms ?? 3000} min={200} max={60000} step={100} integer hint="200〜60000" onChange={(v) => set({ decay_ms: first(v) })} />
          </>
        )}
        {p.mode === "plant" && (
          <>
            {color("色", "color", "#40FF80")}
            <NumberField fixed label="育つ（ms）" value={p.attack_ms ?? 300} min={0} max={60000} step={100} integer onChange={(v) => set({ attack_ms: first(v) })} />
            <NumberField fixed label="保つ（ms）" value={p.sustain_ms ?? 1000} min={0} max={60000} step={100} integer onChange={(v) => set({ sustain_ms: first(v) })} />
            <NumberField fixed label="消える（ms）" value={p.release_ms ?? 1500} min={0} max={60000} step={100} integer onChange={(v) => set({ release_ms: first(v) })} />
            <NumberField fixed label="育った半径（セル）" value={p.size ?? 1.5} min={0.5} max={3} step={0.25} hint="0.5〜3" onChange={(v) => set({ size: first(v) })} />
          </>
        )}
        {p.mode === "paint" && (
          <>
            {color("色", "color", "#FFFFFF")}
            <ListField label="叩くたびの色（任意）" value={p.palette} format={(x) => (x ?? []).join(" ")} parse={(s) => {
              const v = s.split(/[\s,]+/).filter(Boolean).map((c) => c.toUpperCase());
              if (v.length === 0) return undefined;
              return v.length >= 2 && v.length <= 8 && v.every((c) => /^#[0-9A-F]{6}$/.test(c)) ? v : "#RRGGBB を 2〜8 個（空欄で1色）";
            }} hint="同じパッドを叩くたびに次の色。空欄なら上の1色" onChange={(palette) => set({ palette })} />
            <NumberField fixed label="消えるまで（ms）" value={p.decay_ms ?? 0} min={0} max={60000} step={100} integer hint="0 で消えない、200〜60000" onChange={(v) => set({ decay_ms: first(v) || undefined })} />
          </>
        )}
        <p className="hint">1パッド = 1セル。入力履歴（最後の「入力をリセット」以降、クリップの区間内の押下）から毎フレーム計算する。巻き戻しでは再現しない。</p>
      </div>
    );
  }
  if (effect.type === "builtin.cycle") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <ListField label="色の順番（#RRGGBB を 2〜8 個）" value={p.palette} format={(x) => x.join(" ")} parse={(s) => {
          const v = s.split(/[\s,]+/).filter(Boolean).map((c) => c.toUpperCase());
          return v.length >= 2 && v.length <= 8 && v.every((c) => /^#[0-9A-F]{6}$/.test(c)) ? v : "#RRGGBB を 2〜8 個";
        }} onChange={(palette) => set({ palette })} />
        <label className="field">
          <span>数え方</span>
          <select value={p.scope} onChange={(e) => set({ scope: e.target.value as "global" | "per_pad" })}>
            <option value="global">全体で数える</option>
            <option value="per_pad">パッドごとに数える</option>
          </select>
        </label>
        <NumberField fixed label="点灯の長さ（ms）" value={p.lifetime_ms} min={0} max={10000} step={100} integer hint="0 で次の入力まで" onChange={(v) => set({ lifetime_ms: first(v) })} />
        <NumberField fixed label="広がり（セル）" value={p.radius} min={0} max={3} step={0.5} hint="0 で押したパッドだけ" onChange={(v) => set({ radius: first(v) })} />
        <p className="hint">入力履歴に依存（巻き戻しでは再現しない）。</p>
      </div>
    );
  }
  if (effect.type === "builtin.constellation") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <label className="field">
          <span>色</span>
          <input type="color" value={p.color} onChange={(e) => set({ color: e.target.value.toUpperCase() })} />
        </label>
        <NumberField fixed label="線の太さ（セル）" value={p.stroke} min={0.5} max={2} step={0.25} hint="0.5〜2" onChange={(v) => set({ stroke: first(v) })} />
        <NumberField fixed label="消えるまで（ms）" value={p.lifetime_ms} min={200} max={10000} step={100} integer hint="200〜10000" onChange={(v) => set({ lifetime_ms: first(v) })} />
        <label className="field">
          <span>消え方</span>
          <select value={p.fade} onChange={(e) => set({ fade: e.target.value as "linear" | "exp" })}>
            <option value="linear">一定の速さで</option>
            <option value="exp">はじめに速く</option>
          </select>
        </label>
        <NumberField fixed label="残す線の数" value={p.max_segments} min={1} max={16} step={1} integer hint="1〜16（新しい順）" onChange={(v) => set({ max_segments: first(v) })} />
        <label className="field check">
          <input type="checkbox" checked={p.dots} onChange={(e) => set({ dots: e.target.checked })} />
          <span>端点を明るく</span>
        </label>
        <p className="hint">直前に押した位置と今回の位置を線で結ぶ。入力履歴に依存（巻き戻しでは再現しない）。</p>
      </div>
    );
  }
  if (effect.type === "builtin.frames") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <FramePainter frames={p.frames} onChange={(frames) => set({ frames })} />
        <NumberField fixed label="1秒あたりのフレーム数" value={p.fps} min={1} max={30} step={1} hint="1〜30。6 以上は点滅として警告（消灯はしない）" onChange={(v) => set({ fps: first(v) })} />
        <label className="field check">
          <input type="checkbox" checked={p.loop} onChange={(e) => set({ loop: e.target.checked })} />
          <span>繰り返す</span>
        </label>
        {!p.loop && (
          <>
            <label className="field check">
              <input type="checkbox" checked={p.hold_last} onChange={(e) => set({ hold_last: e.target.checked })} />
              <span>最後のフレームを残し続ける</span>
            </label>
            {!p.hold_last && <NumberField fixed label="最後のフレームを残す時間（ms）" value={p.hold_ms ?? 0} min={0} max={60000} step={100} integer hint="スタンプの寿命など。0 ですぐ消える" onChange={(v) => set({ hold_ms: first(v) || undefined })} />}
          </>
        )}
        <label className="field">
          <span>位置</span>
          <select value={p.anchor} onChange={(e) => set({ anchor: e.target.value as "board" | "pad" })}>
            <option value="board">盤全体</option>
            <option value="pad">押したパッドに合わせる（発火レイヤー）</option>
          </select>
          <span className="hint">パッドに合わせると、絵の (3, 3)（下から4段目・左から4列目）が押した位置に来る</span>
        </label>
        <label className="field check">
          <input type="checkbox" checked={p.color !== undefined} onChange={(e) => set({ color: e.target.checked ? "#FFFFFF" : undefined })} />
          <span>点灯セルを1色で塗る</span>
        </label>
        {p.color !== undefined && (
          <label className="field">
            <span>塗る色</span>
            <input type="color" value={p.color} onChange={(e) => set({ color: e.target.value.toUpperCase() })} />
          </label>
        )}
        <label className="field check">
          <input type="checkbox" checked={p.opacity_key ?? true} onChange={(e) => set({ opacity_key: e.target.checked ? undefined : false })} />
          <span>黒を透明にする</span>
        </label>
      </div>
    );
  }
  if (effect.type === "builtin.text") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    const seq = p.mode === "sequential";
    return (
      <div className="insp-grid">
        <label className="field wide">
          <span>文字（ASCII 1〜64 文字）</span>
          <input value={p.text} maxLength={64} spellCheck={false} onChange={(e) => set({ text: e.target.value.replace(/[^ -~]/g, "") || " " })} />
        </label>
        <label className="field">
          <span>フォント</span>
          <select value={p.font} onChange={(e) => set({ font: e.target.value as "5x7" | "3x5" })}>
            <option value="5x7">5×7（小文字あり）</option>
            <option value="3x5">3×5（小文字は大文字で表示）</option>
          </select>
        </label>
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <label className="field">
          <span>出し方</span>
          <select value={p.mode} onChange={(e) => {
            const mode = e.target.value as TextMode;
            set({ mode, speed: mode === "sequential" ? Math.min(8, Math.max(0.2, p.speed)) : Math.max(0.5, p.speed) });
          }}>
            <option value="static">中央に表示（入る分だけ）</option>
            <option value="scroll_left">左へ流す</option>
            <option value="scroll_up">上へ流す</option>
            <option value="sequential">1文字ずつ</option>
          </select>
        </label>
        {p.mode !== "static" && <NumberField fixed label={seq ? "1秒あたりの文字数" : "速さ（セル/秒）"} value={p.speed} min={seq ? 0.2 : 0.5} max={seq ? 8 : 16} step={seq ? 0.1 : 0.5} hint={seq ? "0.2〜8" : "0.5〜16"} onChange={(v) => set({ speed: first(v) })} />}
        <NumberField fixed label="文字の間隔（セル）" value={p.gap} min={0} max={2} step={1} integer onChange={(v) => set({ gap: first(v) })} />
        {seq && (
          <label className="field check">
            <input type="checkbox" checked={p.beat_step ?? false} onChange={(e) => set({ beat_step: e.target.checked || undefined })} />
            <span>拍ごとに進める（拍が不明な間は速さで）</span>
          </label>
        )}
        {p.mode !== "static" && (
          <label className="field check">
            <input type="checkbox" checked={p.loop} onChange={(e) => set({ loop: e.target.checked })} />
            <span>繰り返す</span>
          </label>
        )}
      </div>
    );
  }
  if (effect.type === "builtin.particles") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    const burstLike = p.mode !== "fountain";
    return (
      <div className="insp-grid">
        <label className="field">
          <span>出し方</span>
          <select value={p.mode} onChange={(e) => set({ mode: e.target.value as ParticleMode })}>
            <option value="burst">一度に飛び散る（burst）</option>
            <option value="fountain">出し続ける（fountain）</option>
            <option value="rocket">打ち上げて破裂（rocket）</option>
            <option value="implode">集まってから破裂（implode）</option>
          </select>
          <span className="hint">{p.mode === "fountain" ? "発火レイヤーでは「押している間」に出し続け、他の発火では寿命の間だけ出す" : "タイムラインはクリップ開始で1回、常時レイヤーは繰り返す"}</span>
        </label>
        {burstLike && <NumberField label="数" value={p.count ?? 16} min={1} max={64} step={1} integer hint="1〜64（時間変化は段だけ）" onChange={(count) => set({ count })} />}
        {!burstLike && <NumberField label="1秒あたりの数" value={p.rate ?? 8} min={0.5} max={64} step={0.5} hint="0.5〜64" onChange={(rate) => set({ rate })} />}
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <label className="field check">
          <input type="checkbox" checked={p.color_end !== undefined} onChange={(e) => set({ color_end: e.target.checked ? "#400000" : undefined })} />
          <span>寿命の終わりの色に変える</span>
        </label>
        {p.color_end !== undefined && <ColorField label="終わりの色" value={p.color_end} onChange={(color_end) => set({ color_end })} />}
        <NumberField label="最小の速さ（セル/秒）" value={p.speed_min} min={0} max={16} step={0.5} hint="0〜16" onChange={(speed_min) => set({ speed_min })} />
        <NumberField label="最大の速さ（セル/秒）" value={p.speed_max} min={0} max={16} step={0.5} hint="0〜16" onChange={(speed_max) => set({ speed_max })} />
        <NumberField label="向き（度）" value={p.angle} min={0} max={360} step={15} hint="0 で右、90 で上" onChange={(angle) => set({ angle })} />
        <NumberField label="広がり（度）" value={p.spread} min={0} max={360} step={15} hint="360 で全方向" onChange={(spread) => set({ spread })} />
        <NumberField label="重力（セル/秒²）" value={p.gravity} min={-32} max={32} step={1} hint="負で下へ落ちる" onChange={(gravity) => set({ gravity })} />
        <NumberField label="空気抵抗" value={p.drag} min={0} max={8} step={0.25} hint="0〜8（大きいほど早く止まる）" onChange={(drag) => set({ drag })} />
        <NumberField label="寿命（ms）" value={p.lifetime_ms} min={100} max={6000} step={100} integer hint="100〜6000" onChange={(lifetime_ms) => set({ lifetime_ms })} />
        <NumberField label="粒の大きさ（セル）" value={p.size} min={0.5} max={2} step={0.25} hint="0.5〜2" onChange={(size) => set({ size })} />
        <NumberField label="尾の長さ（セル）" value={p.tail} min={0} max={4} step={0.5} hint="0〜4" onChange={(tail) => set({ tail })} />
        <label className="field">
          <span>盤の端</span>
          <select value={p.edge_mode} onChange={(e) => set({ edge_mode: e.target.value as "vanish" | "bounce" })}>
            <option value="vanish">消える</option>
            <option value="bounce">跳ね返る</option>
          </select>
        </label>
        <NumberField label="発生位置 x" value={p.origin_x ?? 3.5} min={0} max={7} step={0.5} hint="時間で出すとき（パッドで出すときは押した位置）" onChange={(origin_x) => set({ origin_x })} />
        <NumberField label="発生位置 y" value={p.origin_y ?? 3.5} min={0} max={7} step={0.5} hint="0〜7（下が 0）" onChange={(origin_y) => set({ origin_y })} />
        {p.mode === "rocket" && (
          <>
            <NumberField label="上昇の速さ（セル/秒）" value={p.rise_speed ?? 10} min={1} max={32} step={1} hint="1〜32。パッドでは押した列の下端から上がる" onChange={(rise_speed) => set({ rise_speed })} />
            <NumberField label="破裂する高さ（行）" value={p.burst_height ?? 5} min={0} max={7} step={0.5} hint="0〜7" onChange={(burst_height) => set({ burst_height })} />
          </>
        )}
        {p.mode === "implode" && (
          <>
            <NumberField label="集まり始める半径（セル）" value={p.implode_radius ?? 4} min={1} max={6} step={0.5} hint="1〜6。寿命の前半で中心へ集まる" onChange={(implode_radius) => set({ implode_radius })} />
            <NumberField label="中心で待つ時間（ms）" value={p.pause_ms ?? 200} min={0} max={1000} step={50} integer hint="0〜1000" onChange={(pause_ms) => set({ pause_ms })} />
          </>
        )}
        <NumberField label="乱数の種" value={p.seed} min={0} max={2147483647} step={1} integer hint="同じ種なら同じ飛び方（巻き戻しても同じ）" onChange={(seed) => set({ seed })} />
      </div>
    );
  }
  if (effect.type === "builtin.shape") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <label className="field">
          <span>図形</span>
          <select value={p.shape} onChange={(e) => set({ shape: e.target.value as ShapeKind })}>
            <option value="circle">円</option>
            <option value="rect">長方形</option>
            <option value="diamond">ひし形</option>
            <option value="line">線</option>
            <option value="wipe">ワイプ（境界の片側を塗る）</option>
          </select>
        </label>
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        {p.shape !== "line" && p.shape !== "wipe" && (
          <label className="field check">
            <input type="checkbox" checked={p.fill} onChange={(e) => set({ fill: e.target.checked })} />
            <span>塗りつぶす（切ると輪郭だけ）</span>
          </label>
        )}
        {p.shape !== "wipe" && (!p.fill || p.shape === "line") && <NumberField label="線の太さ（セル）" value={p.stroke} min={0.5} max={3} step={0.25} hint="0.5〜3" onChange={(stroke) => set({ stroke })} />}
        <NumberField label="中心 x" value={p.center_x} min={-4} max={11} step={0.5} hint="-4〜11（左端の LED が 0、盤の外にも置ける）" onChange={(center_x) => set({ center_x })} />
        <NumberField label="中心 y" value={p.center_y} min={-4} max={11} step={0.5} hint="-4〜11（下端の LED が 0）" onChange={(center_y) => set({ center_y })} />
        <NumberField label={p.shape === "circle" || p.shape === "diamond" ? "半径（セル）" : "横の半分の長さ（セル）"} value={p.size} min={0} max={6} step={0.25} hint={p.shape === "wipe" ? "ワイプでは使わない" : "0〜6（0 で何も出ない）"} dim={p.shape === "wipe"} onChange={(size) => set({ size })} />
        {p.shape === "rect" && (
          <>
            <label className="field check">
              <input type="checkbox" checked={p.size_y !== undefined} onChange={(e) => set({ size_y: e.target.checked ? first(p.size) : undefined })} />
              <span>縦の長さを別にする（切ると正方形）</span>
            </label>
            {p.size_y !== undefined && <NumberField label="縦の半分の長さ（セル）" value={p.size_y} min={0} max={6} step={0.25} hint="0〜6" onChange={(size_y) => set({ size_y })} />}
          </>
        )}
        <NumberField label="回転（度）" value={p.angle} min={0} max={360} step={15} hint={p.shape === "wipe" ? "塗る側の向き（0 で右側、90 で上側）" : "0〜360（反時計回り）"} onChange={(angle) => set({ angle })} />
        <NumberField label="ふちのぼかし（セル）" value={p.edge} min={0} max={2} step={0.25} hint="0 でくっきり" onChange={(edge) => set({ edge })} />
        <label className="field check">
          <input type="checkbox" checked={first(p.invert ?? false)} onChange={(e) => set({ invert: e.target.checked ? true : undefined })} />
          <span>反転（図形の外側を塗る）</span>
        </label>
        <p className="hint">下のレイヤーを隠す遷移（アイリス・シャッター）は、黒い図形を合成 over で上に重ねて作る。</p>
      </div>
    );
  }
  if (effect.type === "builtin.regions") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <label className="field">
          <span>分け方</span>
          <select value={p.pattern} onChange={(e) => {
            const pattern = e.target.value as RegionPattern;
            // groups / offset belong to custom only.
            set(pattern === "custom" ? { pattern, groups: p.groups ?? [[0, 1, 8, 9], [54, 55, 62, 63]] } : { pattern, groups: undefined, offset: undefined });
          }}>
            <option value="halves_lr">左右の半分</option>
            <option value="halves_tb">上下の半分</option>
            <option value="checker">市松</option>
            <option value="quadrants">4象限（左下→右下→右上→左上）</option>
            <option value="columns">8列</option>
            <option value="rows">8行</option>
            <option value="perimeter">外周（左下から時計回り）</option>
            <option value="fan">中央から外へ</option>
            <option value="custom">自分で決めたグループ</option>
          </select>
        </label>
        {p.pattern === "custom" && (
          <>
            <ListField label="グループ（セル番号）" value={p.groups ?? [[0]]} format={(g) => g.map((x) => x.join(" ")).join("; ")} parse={parseGroups} hint="セル番号 = 行×8 + 列（左下 0、右上 63）。グループを ; で区切り、この順に点く。重なったセルは明るい方" onChange={(groups) => set({ groups, offset: p.offset && p.offset.length === groups.length ? p.offset : undefined })} />
            <ListField label="グループごとのずれ（段）" value={p.offset} format={(o) => (o ?? []).join(" ")} parse={parseOffsets((p.groups ?? [[0]]).length)} hint="空欄でずれなし。0〜1 段だけ遅らせる" onChange={(offset) => set({ offset })} />
          </>
        )}
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <label className="field check">
          <input type="checkbox" checked={p.color_b !== undefined} onChange={(e) => set({ color_b: e.target.checked ? "#000040" : undefined })} />
          <span>点いていない側も色で出す（交互）</span>
        </label>
        {p.color_b !== undefined && <ColorField label="点いていない側の色" value={p.color_b} onChange={(color_b) => set({ color_b })} />}
        <label className="field">
          <span>進み方</span>
          <select value={p.step_mode} onChange={(e) => set({ step_mode: e.target.value as "beat" | "seconds" })}>
            <option value="beat">拍で進む</option>
            <option value="seconds">時間で進む</option>
          </select>
          <span className="hint">拍で進む場合、拍が不明な間は消える</span>
        </label>
        {p.step_mode === "beat" ? (
          <label className="field">
            <span>1段の拍数</span>
            <select value={String(first(p.step_beats))} onChange={(e) => set({ step_beats: Number(e.target.value) })}>
              <option value="0.25">0.25 拍</option>
              <option value="0.5">0.5 拍</option>
              <option value="1">1 拍</option>
              <option value="2">2 拍</option>
              <option value="4">4 拍</option>
            </select>
          </label>
        ) : (
          <NumberField label="1段の長さ（ms）" value={p.step_ms} min={50} max={5000} step={50} hint="50〜5000" integer onChange={(step_ms) => set({ step_ms })} />
        )}
        <label className="field">
          <span>順番</span>
          <select value={p.order} onChange={(e) => set({ order: e.target.value as "forward" | "reverse" | "pingpong" })}>
            <option value="forward">順に</option>
            <option value="reverse">逆順</option>
            <option value="pingpong">往復</option>
          </select>
        </label>
        <SliderField label="なめらかさ" value={p.soft} min={0} max={1} step={0.05} percent hint="50% 以上は点滅として扱わない" onChange={(v) => onChange({ ...effect, params: { ...p, soft: v } } as Effect)} />
        <SliderField label="1段のうち点く割合" value={p.duty} min={0.05} max={1} step={0.05} percent onChange={(v) => onChange({ ...effect, params: { ...p, duty: v } } as Effect)} />
      </div>
    );
  }
  if (effect.type === "builtin.rain") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <ColorField label="先頭の色" value={p.head_color ?? p.color} onChange={(head_color) => set({ head_color })} />
        <label className="field">
          <span>向き</span>
          <select value={p.direction} onChange={(e) => set({ direction: e.target.value as typeof p.direction })}>
            <option value="down">下へ</option>
            <option value="up">上へ</option>
            <option value="left">左へ</option>
            <option value="right">右へ</option>
          </select>
        </label>
        <NumberField label="速さ（セル/秒）" value={p.speed} min={0.5} max={16} step={0.5} hint="0.5〜16" onChange={(speed) => set({ speed })} />
        <SliderField label="粒の多さ" value={p.density} min={0} max={1} step={0.05} percent hint="列ごと・0.5秒ごとに粒が出る確率" onChange={(v) => onChange({ ...effect, params: { ...p, density: v } } as Effect)} />
        <NumberField label="尾の長さ（セル）" value={p.tail} min={0} max={8} step={0.5} hint="0〜8" onChange={(tail) => set({ tail })} />
        <SliderField label="尾のちらつき" value={p.flicker} min={0} max={1} step={0.05} percent hint="0 より大きいと 4 Hz の明滅として安全判定" onChange={(v) => onChange({ ...effect, params: { ...p, flicker: v } } as Effect)} />
        <NumberField label="乱数の種" value={p.seed} min={0} max={2147483647} step={1} hint="同じ種なら同じ降り方" integer onChange={(seed) => set({ seed })} />
      </div>
    );
  }
  if (effect.type === "builtin.wave") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <label className="field">
          <span>流れる方向</span>
          <select value={p.axis} onChange={(e) => set({ axis: e.target.value as "x" | "y" })}>
            <option value="x">横（x）</option>
            <option value="y">縦（y）</option>
          </select>
        </label>
        <NumberField label="波長（セル）" value={p.wavelength} min={2} max={16} step={0.5} hint="2〜16" onChange={(wavelength) => set({ wavelength })} />
        <NumberField label="振幅（セル）" value={p.amplitude} min={0.5} max={3.5} step={0.25} hint="0.5〜3.5" onChange={(amplitude) => set({ amplitude })} />
        <NumberField label="中心（セル）" value={p.center} min={0} max={7} step={0.5} hint="0〜7" onChange={(center) => set({ center })} />
        <NumberField label="速さ（周期/秒）" value={p.speed} min={-4} max={4} step={0.25} hint="−4〜4（負で逆向き）" onChange={(speed) => set({ speed })} />
        <NumberField label="線の太さ（セル）" value={p.thickness} min={0.5} max={3} step={0.25} hint="0.5〜3" onChange={(thickness) => set({ thickness })} />
        <label className="field check">
          <input type="checkbox" checked={p.beat_amplitude} onChange={(e) => set({ beat_amplitude: e.target.checked })} />
          <span>拍で振幅を揺らす（0.3〜1 倍）</span>
        </label>
        <NumberField label="位相のずらし（周期）" value={p.phase ?? 0} min={0} max={1} step={0.05} hint="0〜1" onChange={(phase) => set({ phase })} />
      </div>
    );
  }
  if (effect.type === "builtin.orb") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    const speedRange = p.motion === "orbit" ? { min: 0.05, max: 2, step: 0.05, label: "速さ（周/秒）", hint: "0.05〜2" } : { min: 0.5, max: 12, step: 0.5, label: "速さ（セル/秒）", hint: "0.5〜12" };
    return (
      <div className="insp-grid">
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <NumberField label="中心 x（セル）" value={p.center_x} min={0} max={7} step={0.5} onChange={(center_x) => set({ center_x })} />
        <NumberField label="中心 y（セル）" value={p.center_y} min={0} max={7} step={0.5} onChange={(center_y) => set({ center_y })} />
        <NumberField label="半径（セル）" value={p.radius} min={0.5} max={6} step={0.25} hint="0.5〜6" onChange={(radius) => set({ radius })} />
        <NumberField label="輪郭のぼかし（セル）" value={p.edge} min={0} max={2} step={0.1} hint="0〜2" onChange={(edge) => set({ edge })} />
        <NumberField label="光の向き（度）" value={p.light_angle} min={0} max={360} step={15} hint="90 = 上から" onChange={(light_angle) => set({ light_angle })} />
        <SliderField label="陰影" value={p.shading} min={0} max={1} step={0.05} percent onChange={(v) => onChange({ ...effect, params: { ...p, shading: v } } as Effect)} />
        <SliderField label="粒状の揺らぎ" value={p.dither} min={0} max={1} step={0.05} percent onChange={(v) => onChange({ ...effect, params: { ...p, dither: v } } as Effect)} />
        <NumberField label="揺らぎの更新（回/秒）" value={p.dither_rate_hz} min={0.5} max={15} step={0.5} hint="0.5〜15。揺らぎがあるときこの半分を明滅として安全判定" onChange={(dither_rate_hz) => set({ dither_rate_hz })} />
        <NumberField label="乱数の種" value={p.seed} min={0} max={2147483647} step={1} integer onChange={(seed) => set({ seed })} />
        <label className="field">
          <span>動き</span>
          <select value={p.motion} onChange={(e) => {
            const motion = e.target.value as typeof p.motion;
            // Keep the speed inside the range of the new motion.
            const s0 = first(p.speed);
            const speed = motion === "orbit" ? Math.min(2, Math.max(0.05, s0)) : motion === "bounce" ? Math.min(12, Math.max(0.5, s0)) : p.speed;
            set({ motion, speed });
          }}>
            <option value="none">止まる</option>
            <option value="orbit">回る</option>
            <option value="bounce">跳ね返る</option>
          </select>
        </label>
        {p.motion !== "none" && <NumberField label={speedRange.label} value={p.speed} min={speedRange.min} max={speedRange.max} step={speedRange.step} hint={speedRange.hint} onChange={(speed) => set({ speed })} />}
        {p.motion === "orbit" && <NumberField label="回る半径（セル）" value={p.orbit_radius} min={0} max={4} step={0.25} hint="0〜4" onChange={(orbit_radius) => set({ orbit_radius })} />}
        {p.motion === "orbit" && <NumberField label="回り始めの角度（周）" value={p.orbit_phase ?? 0} min={0} max={1} step={0.05} hint="0〜1" onChange={(orbit_phase) => set({ orbit_phase })} />}
        {p.motion === "bounce" && <NumberField label="跳ねる向き（度）" value={p.bounce_angle} min={0} max={360} step={5} onChange={(bounce_angle) => set({ bounce_angle })} />}
      </div>
    );
  }
  if (effect.type === "builtin.ripple") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <ColorField value={p.color} onChange={(color) => set({ color })} />
        <NumberField label="広がる速さ（セル/秒）" value={p.speed} min={0.5} max={16} step={0.5} hint="0.5〜16" onChange={(speed) => set({ speed })} />
        <NumberField label="輪の太さ（セル）" value={p.ring_width} min={0.5} max={4} step={0.25} hint="0.5〜4" onChange={(ring_width) => set({ ring_width })} />
        <NumberField label="消えるまで（ms）" value={p.lifetime_ms} min={200} max={4000} step={100} hint="200〜4000" integer onChange={(lifetime_ms) => set({ lifetime_ms })} />
        <label className="field">
          <span>消え方</span>
          <select value={p.fade} onChange={(e) => set({ fade: e.target.value as "linear" | "exp" })}>
            <option value="linear">一定の速さで</option>
            <option value="exp">はじめに速く</option>
          </select>
        </label>
        <NumberField label="同時に出す数" value={p.max_concurrent} min={1} max={16} step={1} hint="1〜16（超えたら古い輪から消す）" integer onChange={(max_concurrent) => set({ max_concurrent })} />
        <label className="field">
          <span>輪の形</span>
          <select value={p.shape ?? "circle"} onChange={(e) => set({ shape: e.target.value as "circle" | "diamond" | "square" })}>
            <option value="circle">円</option>
            <option value="diamond">ひし形</option>
            <option value="square">四角</option>
          </select>
        </label>
        <label className="field">
          <span>出るきっかけ</span>
          <select value={p.trigger} onChange={(e) => {
            const trigger = e.target.value as "any_pad" | "interval";
            // Interval needs its origin and interval (defaults: centre, 1 s).
            set(trigger === "interval" ? { trigger, origin_x: p.origin_x ?? 3.5, origin_y: p.origin_y ?? 3.5, interval_ms: p.interval_ms ?? 1000 } : { trigger });
          }}>
            <option value="any_pad">パッドを押したとき</option>
            <option value="interval">一定の間隔（入力なし）</option>
          </select>
        </label>
        {p.trigger === "interval" && (
          <>
            <NumberField label="中心 x（セル）" value={p.origin_x ?? 3.5} min={0} max={7} step={0.5} onChange={(origin_x) => set({ origin_x })} />
            <NumberField label="中心 y（セル）" value={p.origin_y ?? 3.5} min={0} max={7} step={0.5} onChange={(origin_y) => set({ origin_y })} />
            <NumberField label="間隔（ms）" value={p.interval_ms ?? 1000} min={100} max={10000} step={100} hint="100〜10000（クリップの時刻で決まる）" integer onChange={(interval_ms) => set({ interval_ms })} />
          </>
        )}
        <p className="hint">「パッドを押したとき」は押した位置から輪が広がる（操作入力がないと何も出ない。プレビュー盤のクリックでも確認できる）。「一定の間隔」は入力なしで指定の位置から出る。</p>
      </div>
    );
  }
  if (effect.type === "builtin.noise") {
    const p = effect.params;
    const set = (patch: Partial<typeof p>) => onChange({ ...effect, params: { ...p, ...patch } });
    return (
      <div className="insp-grid">
        <ColorField label="地の色" value={p.base_color} onChange={(base_color) => set({ base_color })} />
        <ColorField label="粒の色" value={p.highlight_color} onChange={(highlight_color) => set({ highlight_color })} />
        <SliderField label="粒の割合" value={p.density} min={0} max={1} step={0.05} percent hint="0〜1（点灯するセルの割合）" onChange={(v) => onChange({ ...effect, params: { ...p, density: v } } as Effect)} />
        <NumberField label="模様の更新（回/秒）" value={p.rate_hz} min={0.5} max={30} step={0.5} hint="0.5〜30。セルの明滅はこの半分として安全判定" onChange={(rate_hz) => set({ rate_hz })} />
        <SliderField label="粒の明るさの揺らぎ" value={p.depth} min={0} max={1} step={0.05} percent hint="0 で全部同じ明るさ" onChange={(v) => onChange({ ...effect, params: { ...p, depth: v } } as Effect)} />
        <NumberField label="乱数の種" value={p.seed} min={0} max={2147483647} step={1} hint="同じ種なら同じ模様" integer onChange={(seed) => set({ seed })} />
        <label className="field check">
          <input type="checkbox" checked={p.smooth} onChange={(e) => set({ smooth: e.target.checked })} />
          <span>次の模様へなめらかに移る</span>
        </label>
      </div>
    );
  }
  const p = effect.params;
  return (
    <div className="insp-grid">
      <ColorField value={p.color} onChange={(color) => onChange({ ...effect, params: { ...p, color } })} />
      <label className="field">
        <span>1周期の拍数</span>
        <select value={String(first(p.beats_per_cycle))} onChange={(e) => onChange({ ...effect, params: { ...p, beats_per_cycle: Number(e.target.value) } })}>
          <option value="0.25">0.25 拍（16分）</option>
          <option value="0.5">0.5 拍（8分）</option>
          <option value="1">1 拍（4分）</option>
        </select>
      </label>
      <SliderField label="深さ" value={p.depth} min={0} max={1} step={0.05} percent hint="0〜1（1 で消灯まで落ちる）" onChange={(v) => onChange({ ...effect, params: { ...p, depth: v } } as Effect)} />
      <label className="field">
        <span>波形</span>
        <select value={p.waveform} onChange={(e) => onChange({ ...effect, params: { ...p, waveform: e.target.value as "sine" | "triangle" } })}>
          <option value="sine">サイン</option>
          <option value="triangle">三角</option>
        </select>
      </label>
    </div>
  );
}

export function LayerInspector({ view, layer, apply }: { view: EditorView; layer: LayerDto; apply: (c: EditCommand) => void }) {
  const [name, setName] = useState(layer.name);
  useEffect(() => setName(layer.name), [layer.id, layer.name]);
  const set = (patch: Partial<{ name: string; enabled: boolean; opacity: number; blend: Blend; target_id: string }>) =>
    apply({ op: "set_layer", layer_id: layer.id, name: patch.name ?? null, enabled: patch.enabled ?? null, opacity: patch.opacity ?? null, blend: patch.blend ?? null, target_id: patch.target_id ?? null });
  return (
    <div className="inspector">
      <div className="insp-title">レイヤー <span className="mono">{layer.id}</span>（{layer.kind === "timeline" ? "タイムライン" : layer.kind === "triggered" ? "発火" : "常時"}）</div>
      <div className="insp-grid">
        <label className="field">
          <span>名前</span>
          <input value={name} onChange={(e) => setName(e.target.value)} onBlur={() => name !== layer.name && set({ name })} />
        </label>
        <label className="field check">
          <input type="checkbox" checked={layer.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
          <span>有効</span>
        </label>
        <label className="field">
          <span>不透明度 {Math.round(layer.opacity * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} defaultValue={layer.opacity} key={`${layer.id}-${layer.opacity}`} onPointerUp={(e) => set({ opacity: Number((e.target as HTMLInputElement).value) })} onKeyUp={(e) => set({ opacity: Number((e.target as HTMLInputElement).value) })} />
          <span className="hint">0〜1</span>
        </label>
        <label className="field">
          <span>合成</span>
          <select value={layer.blend} onChange={(e) => set({ blend: e.target.value as Blend })}>
            <option value="over">over（重ねる）</option>
            <option value="add">add（加算）</option>
            <option value="max">max（明るい方）</option>
          </select>
        </label>
        {layer.target_id !== "" && (
          <label className="field">
            <span>出力先</span>
            <select value={layer.target_id} onChange={(e) => set({ target_id: e.target.value })}>
              {view.targets.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
            </select>
          </label>
        )}
        {layer.kind === "triggered" && layer.trigger && <TriggerFields layer={layer} trigger={layer.trigger} apply={apply} />}
        <MaskFields layer={layer} apply={apply} />
        {layer.kind === "continuous" && (
          <label className="field">
            <span>プリセット</span>
            <select value={layer.preset_id ?? ""} onChange={(e) => apply({ op: "set_layer_preset", layer_id: layer.id, preset_id: e.target.value })}>
              {view.presets.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
            </select>
          </label>
        )}
      </div>
    </div>
  );
}

const DEFAULT_MASK: LayerMask = { shape: "circle", center_x: 3.5, center_y: 3.5, size: 3, edge: 0.5, invert: false };

/** Optional mask: cuts this layer's own drawing (not the layers below). */
function MaskFields({ layer, apply }: { layer: LayerDto; apply: (c: EditCommand) => void }) {
  const m = layer.mask;
  const put = (mask: LayerMask | null) => apply({ op: "set_layer_mask", layer_id: layer.id, mask });
  const toggle = (
    <label className="field check wide">
      <input type="checkbox" checked={m !== null} onChange={(e) => put(e.target.checked ? DEFAULT_MASK : null)} />
      <span>マスク（このレイヤーの描画だけを図形で切り抜く）</span>
    </label>
  );
  if (!m) return toggle;
  const set = (patch: Partial<LayerMask>) => put({ ...m, ...patch });
  const shape = first(m.shape);
  const time = layer.kind === "continuous" ? "曲の時刻" : "クリップの中の時刻";
  return (
    <>
      {toggle}
      <label className="field">
        <span>マスクの形</span>
        <select value={shape} onChange={(e) => {
          const s = e.target.value as LayerMask["shape"] & string;
          set(s === "rect" ? { shape: s } : { shape: s, size_y: undefined });
        }}>
          <option value="circle">円</option>
          <option value="rect">長方形</option>
          <option value="diamond">ひし形</option>
          <option value="wipe">ワイプ（境界の片側）</option>
        </select>
        <span className="hint">時間変化は{time}で進む</span>
      </label>
      <NumberField label="中心 x" value={m.center_x} min={-4} max={11} step={0.5} hint="-4〜11" onChange={(center_x) => set({ center_x })} />
      <NumberField label="中心 y" value={m.center_y} min={-4} max={11} step={0.5} hint="-4〜11" onChange={(center_y) => set({ center_y })} />
      <NumberField label={shape === "rect" ? "横の半分の長さ（セル）" : "半径（セル）"} value={m.size} min={0} max={8} step={0.25} hint={shape === "wipe" ? "ワイプでは使わない" : "0〜8"} dim={shape === "wipe"} onChange={(size) => set({ size })} />
      {shape === "rect" && (
        <>
          <label className="field check">
            <input type="checkbox" checked={m.size_y !== undefined} onChange={(e) => set({ size_y: e.target.checked ? first(m.size) : undefined })} />
            <span>縦の長さを別にする</span>
          </label>
          {m.size_y !== undefined && <NumberField label="縦の半分の長さ（セル）" value={m.size_y} min={0} max={8} step={0.25} hint="0〜8" onChange={(size_y) => set({ size_y })} />}
        </>
      )}
      <NumberField label="回転（度）" value={m.angle ?? 0} min={0} max={360} step={15} hint={shape === "wipe" ? "残す側の向き（0 で右側）" : "0〜360"} onChange={(angle) => set({ angle })} />
      <NumberField label="ふちのぼかし（セル）" value={m.edge} min={0} max={2} step={0.25} hint="0 でくっきり" onChange={(edge) => set({ edge })} />
      <label className="field check">
        <input type="checkbox" checked={first(m.invert)} onChange={(e) => set({ invert: e.target.checked })} />
        <span>反転（図形の外側を残す）</span>
      </label>
    </>
  );
}

/** Japanese names of the triggers. */
export const TRIGGER_JA: Record<TriggerOn, string> = {
  press: "押したとき",
  release: "離したとき",
  hold: "押している間",
  clip_start: "クリップの開始",
  interval: "一定の間隔",
};

/** Trigger of a triggered layer: clips are its active windows. */
function TriggerFields({ layer, trigger, apply }: { layer: LayerDto; trigger: TriggerSpec; apply: (c: EditCommand) => void }) {
  const set = (patch: Partial<TriggerSpec>) => apply({ op: "set_layer_trigger", layer_id: layer.id, trigger: { ...trigger, ...patch } });
  const positioned = trigger.on === "clip_start" || trigger.on === "interval";
  return (
    <>
      <label className="field">
        <span>発火</span>
        <select value={trigger.on} onChange={(e) => {
          const on = e.target.value as TriggerOn;
          set(on === "interval" ? { on, interval_ms: trigger.interval_ms ?? 1000 } : { on });
        }}>
          {Object.entries(TRIGGER_JA).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
        </select>
        <span className="hint">{positioned ? "曲の時刻で決まる（巻き戻しても同じ）" : "パッドの入力で出る（巻き戻しでは再現しない）。クリップの区間内の入力だけ"}</span>
      </label>
      {trigger.on === "interval" && <NumberField fixed label="間隔（ms）" value={trigger.interval_ms ?? 1000} min={100} max={10000} step={100} hint="100〜10000" integer onChange={(v) => set({ interval_ms: first(v) })} />}
      {positioned && <NumberField fixed label="位置 x（セル）" value={trigger.origin_x ?? 3.5} min={0} max={7} step={0.5} onChange={(v) => set({ origin_x: first(v) })} />}
      {positioned && <NumberField fixed label="位置 y（セル）" value={trigger.origin_y ?? 3.5} min={0} max={7} step={0.5} onChange={(v) => set({ origin_y: first(v) })} />}
    </>
  );
}

/** Show sync settings (beat source, MTC output). Ports come from the terminal settings. */
export function ShowSyncPanel({ view, apply }: { view: EditorView; apply: (c: EditCommand) => void }) {
  return (
    <section className="panel">
      <h2>ショーの同期</h2>
      <div className="insp-grid">
        <label className="field">
          <span>拍源（Pulse の拍）</span>
          <select value={view.beat_source} onChange={(e) => apply({ op: "set_sync", beat_source: e.target.value as BeatSource, mtc_output_enabled: null })}>
            <option value="midi_clock">MIDI Clock</option>
            <option value="ableton_link">Ableton Link</option>
          </select>
        </label>
        <label className="field check">
          <input type="checkbox" checked={view.mtc_output_enabled} onChange={(e) => apply({ op: "set_sync", beat_source: null, mtc_output_enabled: e.target.checked })} />
          <span>MTC を出力する（接続設定の「タイムコード出力」へ）</span>
        </label>
      </div>
      <p className="hint">拍が来ないと Pulse は透明になる。MIDI Clock は接続設定の「MIDI Clock入力」、Link は「Link を使う」が要る。</p>
    </section>
  );
}

/** Which edit the preset editor performs. */
export type PresetEditMode = { kind: "shared"; presetId: string } | { kind: "clip"; clipId: string; presetId: string };

export function PresetPanel(props: {
  view: EditorView;
  mode: PresetEditMode | null;
  setMode: (m: PresetEditMode | null) => void;
  apply: (c: EditCommand) => void;
  applyView: (f: () => Promise<EditorView>) => void;
}) {
  const { view, mode, setMode, apply, applyView } = props;
  const preset: PresetDto | undefined = mode ? view.presets.find((p) => p.id === mode.presetId) : undefined;
  const [draft, setDraft] = useState<Effect | null>(null);
  const [name, setName] = useState("");
  const [newType, setNewType] = useState<EffectType>("builtin.solid");
  const [importPath, setImportPath] = useState("");
  const [conflict, setConflict] = useState<OnConflict>("rename");
  const [exportPath, setExportPath] = useState("");
  const [overwrite, setOverwrite] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    setDraft(preset ? (JSON.parse(JSON.stringify(preset.effect)) as Effect) : null);
    setName(preset?.name ?? "");
  }, [preset?.id, JSON.stringify(preset?.effect), mode?.kind]);

  const users = preset ? preset.used_by_clips.length + preset.used_by_layers.length : 0;
  const applyDraft = () => {
    if (!mode || !draft) return;
    if (mode.kind === "clip") apply({ op: "edit_clip_effect", clip_id: mode.clipId, effect: draft });
    else apply({ op: "set_preset", preset_id: mode.presetId, name: name !== preset?.name ? name : null, effect: draft });
  };

  return (
    <section className="panel">
      <h2>プリセット</h2>
      <div className="preset-list">
        {view.presets.map((p) => (
          <button key={p.id} className={mode?.presetId === p.id ? "preset on" : "preset"} onClick={() => setMode({ kind: "shared", presetId: p.id })}>
            <span className="swatch" style={{ background: p.color }} />
            <span className="pname">{p.name}</span>
            <span className="hint">{TYPE_JA[p.effect_type]} · 使用 {p.used_by_clips.length + p.used_by_layers.length}</span>
          </button>
        ))}
      </div>
      <div className="actions">
        <select value={newType} onChange={(e) => setNewType(e.target.value as EffectType)}>
          {Object.entries(TYPE_JA).map(([t, l]) => <option key={t} value={t}>{l}</option>)}
        </select>
        <button onClick={() => apply({ op: "add_preset", name: "", effect_type: newType })}>新規プリセット</button>
      </div>

      {preset && draft && mode && (
        <div className="inspector">
          <div className="insp-title">
            {mode.kind === "clip" ? <>クリップ <span className="mono">{mode.clipId}</span> だけの効果</> : <>プリセット <span className="mono">{preset.id}</span>（共有の編集）</>} · {TYPE_JA[preset.effect_type]}
          </div>
          {mode.kind === "shared" && (
            <label className="field">
              <span>名前</span>
              <input value={name} onChange={(e) => setName(e.target.value)} />
            </label>
          )}
          <EffectForm effect={draft} onChange={setDraft} />
          <p className={mode.kind === "shared" && users > 1 ? "note" : "hint"}>
            {mode.kind === "shared"
              ? users > 1
                ? `共有中：使用中の ${users} 箇所すべてが変わる（${[...preset.used_by_clips, ...preset.used_by_layers].join(", ")}）`
                : `使用 ${users} 箇所`
              : users > 1
                ? `このプリセットは ${users} 箇所で共有中。適用するとこのクリップ用に複製し、このクリップだけを差し替える`
                : "このクリップだけが使っているので、そのまま変更する"}
          </p>
          <div className="actions">
            <button className="primary" onClick={applyDraft}>{mode.kind === "clip" ? "このクリップに適用" : "適用（共有）"}</button>
            {mode.kind === "shared" && <button className="danger" onClick={() => apply({ op: "delete_preset", preset_id: preset.id })}>削除</button>}
            <button onClick={() => setMode(null)}>閉じる</button>
          </div>
          {mode.kind === "shared" && (
            <div className="io-row">
              <input placeholder="書き出し先（…\name.preset.json）" value={exportPath} onChange={(e) => setExportPath(e.target.value)} spellCheck={false} />
              <button onClick={() => ipc.pickPresetSave(`${preset.id}.preset.json`).then((p) => p && setExportPath(p))}>選ぶ…</button>
              <label className="inline"><input type="checkbox" checked={overwrite} onChange={(e) => setOverwrite(e.target.checked)} />上書き</label>
              <button
                disabled={!exportPath}
                onClick={() =>
                  ipc
                    .presetExport(preset.id, exportPath, overwrite)
                    .then((p) => setNote(`書き出した：${p}`))
                    .catch((e) => setNote(ipc.errorText(e)))
                }
              >
                書き出し
              </button>
            </div>
          )}
        </div>
      )}

      <div className="io-row">
        <input placeholder="読み込むプリセットファイル（kind omoiiro.preset、0.1.1）" value={importPath} onChange={(e) => setImportPath(e.target.value)} spellCheck={false} />
        <button onClick={() => ipc.pickPreset().then((p) => p && setImportPath(p))}>選ぶ…</button>
        <select value={conflict} onChange={(e) => setConflict(e.target.value as OnConflict)}>
          <option value="rename">ID が衝突したら別名</option>
          <option value="error">ID が衝突したらエラー</option>
        </select>
        <button disabled={!importPath} onClick={() => applyView(() => ipc.presetImport(importPath, conflict))}>読み込み</button>
      </div>
      <div className="io-row">
        <button onClick={() => ipc.pickFramesAsset().then((p) => p && applyView(() => ipc.framesImport(p, conflict)))} title="omoiiro.frames（8×8 のフレーム列）か omoiiro.bitmap（8×8 の点灯図）を、フレーム列のプリセットとして読み込む">フレーム列を読み込み…</button>
        <span className="hint">ID の衝突は左の設定に従う。ビットマップは白い1枚として入る（色はプリセットで変えられる）。</span>
      </div>
      {note && <p className="hint">{note}</p>}
    </section>
  );
}
