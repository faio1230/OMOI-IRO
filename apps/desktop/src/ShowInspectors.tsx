// U05: layer properties, preset library and effect parameters. Every change
// is an EditCommand applied and validated in Rust (same history and runtime
// hand-off as the timeline); ranges shown here follow the show schema.
import { useEffect, useState } from "react";
import * as ipc from "./ipc/commands";
import type { BeatSource, Blend, EditCommand, EditorView, Effect, EffectType, LayerDto, OnConflict, PresetDto, RegionPattern } from "./ipc/types";

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
};

/** Numeric input clamped to [min, max] on change. */
function NumberField(props: { label: string; value: number; min: number; max: number; step: number; hint?: string; onChange: (v: number) => void }) {
  return (
    <label className="field">
      <span>{props.label}</span>
      <input type="number" min={props.min} max={props.max} step={props.step} value={props.value} onChange={(e) => props.onChange(Math.min(props.max, Math.max(props.min, Number(e.target.value))))} />
      {props.hint && <span className="hint">{props.hint}</span>}
    </label>
  );
}

function ColorField({ value, onChange, label = "色（#RRGGBB）" }: { value: string; onChange: (v: string) => void; label?: string }) {
  return (
    <label className="field">
      <span>{label}</span>
      <div className="color-row">
        <input type="color" value={/^#[0-9a-fA-F]{6}$/.test(value) ? value : "#000000"} onChange={(e) => onChange(e.target.value.toUpperCase())} />
        <input value={value} onChange={(e) => onChange(e.target.value)} spellCheck={false} />
      </div>
    </label>
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
        <label className="field">
          <span>周期（秒）</span>
          <input type="number" min={0.001} max={3600} step={0.1} value={p.period_us / 1e6} onChange={(e) => onChange({ ...effect, params: { ...p, period_us: Math.round(Number(e.target.value) * 1e6) } })} />
          <span className="hint">0.001〜3600 秒（1周する時間）</span>
        </label>
        <label className="field">
          <span>帯の幅（LED）</span>
          <input type="number" min={1} max={8} step={1} value={p.width} onChange={(e) => onChange({ ...effect, params: { ...p, width: Math.round(Number(e.target.value)) } })} />
          <span className="hint">1〜8</span>
        </label>
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
            <select value={String(p.beats_per_cycle)} onChange={(e) => set({ beats_per_cycle: Number(e.target.value) })}>
              <option value="0.25">0.25 拍（16分）</option>
              <option value="0.5">0.5 拍（8分）</option>
              <option value="1">1 拍（4分）</option>
              <option value="2">2 拍（2分）</option>
            </select>
          </label>
        )}
        <label className="field">
          <span>点灯の割合 {Math.round(p.duty * 100)}%</span>
          <input type="range" min={0.05} max={0.95} step={0.05} value={p.duty} onChange={(e) => set({ duty: Number(e.target.value) })} />
          <span className="hint">0.05〜0.95</span>
        </label>
        <label className="field">
          <span>位相のずらし {Math.round(p.phase_offset * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.phase_offset} onChange={(e) => set({ phase_offset: Number(e.target.value) })} />
          <span className="hint">0〜1 周期</span>
        </label>
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
        <NumberField label="消えるまで（ms）" value={p.lifetime_ms} min={100} max={5000} step={100} hint="100〜5000" onChange={(v) => set({ lifetime_ms: Math.round(v) })} />
        <NumberField label="広がり（セル）" value={p.radius} min={0} max={3} step={0.25} hint="0 で押したパッドだけ" onChange={(radius) => set({ radius })} />
        <NumberField label="繰り返し回数" value={p.repeats} min={1} max={8} step={1} hint="1〜8" onChange={(v) => set({ repeats: Math.round(v) })} />
        <NumberField label="繰り返しの間隔（ms）" value={p.repeat_interval_ms} min={50} max={2000} step={50} hint="50〜2000" onChange={(v) => set({ repeat_interval_ms: Math.round(v) })} />
        <label className="field">
          <span>繰り返すたびの明るさ {Math.round(p.repeat_decay * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.repeat_decay} onChange={(e) => set({ repeat_decay: Number(e.target.value) })} />
        </label>
        <p className="hint">パッドを押した位置が光って冷めていく。操作入力がないと何も出ない（プレビュー盤のクリックでも確認できる）。</p>
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
        <NumberField label="消えるまで（ms）" value={p.lifetime_ms} min={100} max={3000} step={100} hint="100〜3000" onChange={(v) => set({ lifetime_ms: Math.round(v) })} />
        <p className="hint">押した位置から上下左右へ光が走る。操作入力がないと何も出ない（プレビュー盤のクリックでも確認できる）。</p>
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
          <select value={p.pattern} onChange={(e) => set({ pattern: e.target.value as RegionPattern })}>
            <option value="halves_lr">左右の半分</option>
            <option value="halves_tb">上下の半分</option>
            <option value="checker">市松</option>
            <option value="quadrants">4象限（左下→右下→右上→左上）</option>
            <option value="columns">8列</option>
            <option value="rows">8行</option>
            <option value="perimeter">外周（左下から時計回り）</option>
            <option value="fan">中央から外へ</option>
          </select>
        </label>
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
            <select value={String(p.step_beats)} onChange={(e) => set({ step_beats: Number(e.target.value) })}>
              <option value="0.25">0.25 拍</option>
              <option value="0.5">0.5 拍</option>
              <option value="1">1 拍</option>
              <option value="2">2 拍</option>
              <option value="4">4 拍</option>
            </select>
          </label>
        ) : (
          <NumberField label="1段の長さ（ms）" value={p.step_ms} min={50} max={5000} step={50} hint="50〜5000" onChange={(v) => set({ step_ms: Math.round(v) })} />
        )}
        <label className="field">
          <span>順番</span>
          <select value={p.order} onChange={(e) => set({ order: e.target.value as "forward" | "reverse" | "pingpong" })}>
            <option value="forward">順に</option>
            <option value="reverse">逆順</option>
            <option value="pingpong">往復</option>
          </select>
        </label>
        <label className="field">
          <span>なめらかさ {Math.round(p.soft * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.soft} onChange={(e) => set({ soft: Number(e.target.value) })} />
          <span className="hint">50% 以上は点滅として扱わない</span>
        </label>
        <label className="field">
          <span>1段のうち点く割合 {Math.round(p.duty * 100)}%</span>
          <input type="range" min={0.05} max={1} step={0.05} value={p.duty} onChange={(e) => set({ duty: Number(e.target.value) })} />
        </label>
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
        <label className="field">
          <span>粒の多さ {Math.round(p.density * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.density} onChange={(e) => set({ density: Number(e.target.value) })} />
          <span className="hint">列ごと・0.5秒ごとに粒が出る確率</span>
        </label>
        <NumberField label="尾の長さ（セル）" value={p.tail} min={0} max={8} step={0.5} hint="0〜8" onChange={(tail) => set({ tail })} />
        <label className="field">
          <span>尾のちらつき {Math.round(p.flicker * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.flicker} onChange={(e) => set({ flicker: Number(e.target.value) })} />
          <span className="hint">0 より大きいと 4 Hz の明滅として安全判定</span>
        </label>
        <NumberField label="乱数の種" value={p.seed} min={0} max={2147483647} step={1} hint="同じ種なら同じ降り方" onChange={(seed) => set({ seed: Math.round(seed) })} />
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
        <label className="field">
          <span>陰影 {Math.round(p.shading * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.shading} onChange={(e) => set({ shading: Number(e.target.value) })} />
        </label>
        <label className="field">
          <span>粒状の揺らぎ {Math.round(p.dither * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.dither} onChange={(e) => set({ dither: Number(e.target.value) })} />
        </label>
        <NumberField label="揺らぎの更新（回/秒）" value={p.dither_rate_hz} min={0.5} max={15} step={0.5} hint="0.5〜15。揺らぎがあるときこの半分を明滅として安全判定" onChange={(dither_rate_hz) => set({ dither_rate_hz })} />
        <NumberField label="乱数の種" value={p.seed} min={0} max={2147483647} step={1} onChange={(seed) => set({ seed: Math.round(seed) })} />
        <label className="field">
          <span>動き</span>
          <select value={p.motion} onChange={(e) => {
            const motion = e.target.value as typeof p.motion;
            // Keep the speed inside the range of the new motion.
            const speed = motion === "orbit" ? Math.min(2, Math.max(0.05, p.speed)) : motion === "bounce" ? Math.min(12, Math.max(0.5, p.speed)) : p.speed;
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
        <NumberField label="消えるまで（ms）" value={p.lifetime_ms} min={200} max={4000} step={100} hint="200〜4000" onChange={(v) => set({ lifetime_ms: Math.round(v) })} />
        <label className="field">
          <span>消え方</span>
          <select value={p.fade} onChange={(e) => set({ fade: e.target.value as "linear" | "exp" })}>
            <option value="linear">一定の速さで</option>
            <option value="exp">はじめに速く</option>
          </select>
        </label>
        <NumberField label="同時に出す数" value={p.max_concurrent} min={1} max={16} step={1} hint="1〜16（超えたら古い輪から消す）" onChange={(v) => set({ max_concurrent: Math.round(v) })} />
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
            <NumberField label="間隔（ms）" value={p.interval_ms ?? 1000} min={100} max={10000} step={100} hint="100〜10000（クリップの時刻で決まる）" onChange={(v) => set({ interval_ms: Math.round(v) })} />
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
        <label className="field">
          <span>粒の割合 {Math.round(p.density * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.density} onChange={(e) => set({ density: Number(e.target.value) })} />
          <span className="hint">0〜1（点灯するセルの割合）</span>
        </label>
        <NumberField label="模様の更新（回/秒）" value={p.rate_hz} min={0.5} max={30} step={0.5} hint="0.5〜30。セルの明滅はこの半分として安全判定" onChange={(rate_hz) => set({ rate_hz })} />
        <label className="field">
          <span>粒の明るさの揺らぎ {Math.round(p.depth * 100)}%</span>
          <input type="range" min={0} max={1} step={0.05} value={p.depth} onChange={(e) => set({ depth: Number(e.target.value) })} />
          <span className="hint">0 で全部同じ明るさ</span>
        </label>
        <NumberField label="乱数の種" value={p.seed} min={0} max={2147483647} step={1} hint="同じ種なら同じ模様" onChange={(seed) => set({ seed: Math.round(seed) })} />
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
        <select value={String(p.beats_per_cycle)} onChange={(e) => onChange({ ...effect, params: { ...p, beats_per_cycle: Number(e.target.value) } })}>
          <option value="0.25">0.25 拍（16分）</option>
          <option value="0.5">0.5 拍（8分）</option>
          <option value="1">1 拍（4分）</option>
        </select>
      </label>
      <label className="field">
        <span>深さ {Math.round(p.depth * 100)}%</span>
        <input type="range" min={0} max={1} step={0.05} value={p.depth} onChange={(e) => onChange({ ...effect, params: { ...p, depth: Number(e.target.value) } })} />
        <span className="hint">0〜1（1 で消灯まで落ちる）</span>
      </label>
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
      <div className="insp-title">レイヤー <span className="mono">{layer.id}</span>（{layer.kind === "timeline" ? "タイムライン" : "常時"}）</div>
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
        <label className="field">
          <span>出力先</span>
          <select value={layer.target_id} onChange={(e) => set({ target_id: e.target.value })}>
            {view.targets.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
          </select>
        </label>
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
      {note && <p className="hint">{note}</p>}
    </section>
  );
}
