import { useEffect, useMemo, useRef, useState } from "react";
import * as ipc from "./ipc/commands";
import type { EditCommand, EditorView, LibraryView, MaterialKind, MaterialMetadata } from "./ipc/types";

const CATEGORIES = ["タップ", "背景", "アクセント", "トランジション", "照明", "グラフィック"];
const PAGE_SIZE = 12;
/** `source`: the library file it comes from (absent = the edited show). */
type Material = { id: string; kind: MaterialKind; name: string; metadata: MaterialMetadata; label: string; used: number; signature: string; durationMs: number; source?: string; sourceName?: string };
const keyOf = (m: Material) => `${m.source ?? ""}|${m.kind}:${m.id}`;
const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;
type LibraryEntry = { path: string; view?: LibraryView; error?: string };

/** Draw only Rust-rendered pixels; y=0 is the bottom row on the device. */
function MiniPreview({ frames, animate, name, durationMs = 2000 }: { frames?: number[][]; animate: boolean; name: string; durationMs?: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const ctx = canvas.current?.getContext("2d");
    if (!ctx) return;
    let index = frames ? Math.min(4, frames.length - 1) : 0;
    const paint = () => {
      ctx.fillStyle = "#050505"; ctx.fillRect(0, 0, 96, 96);
      const f = frames?.[index];
      for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) {
        const k = (y * 8 + x) * 3;
        ctx.fillStyle = f ? `rgb(${f[k]},${f[k + 1]},${f[k + 2]})` : "#161616";
        ctx.fillRect(x * 12 + 1, (7 - y) * 12 + 1, 10, 10);
      }
      index = (index + 1) % (frames?.length || 1);
    };
    paint();
    if (!animate || !frames) return;
    const timer = window.setInterval(() => { if (!document.hidden) paint(); }, Math.max(16, durationMs / frames.length));
    return () => clearInterval(timer);
  }, [frames, animate, durationMs]);
  return <canvas ref={canvas} width={96} height={96} role="img" aria-label={`${name}のLEDプレビュー`} />;
}

function Classification({ material, categories, apply }: { material: Material; categories: string[]; apply: (c: EditCommand) => void }) {
  const [category, setCategory] = useState(material.metadata.category);
  const [tags, setTags] = useState(material.metadata.tags.join(", "));
  useEffect(() => { setCategory(material.metadata.category); setTags(material.metadata.tags.join(", ")); }, [material.id, material.kind, JSON.stringify(material.metadata)]);
  const parsed = [...new Map(tags.split(/[,、]/).map(t => t.trim()).filter(Boolean).map(t => [t.toLocaleLowerCase(), t])).values()];
  const valid = [...category.trim()].length <= 48 && parsed.length <= 24 && parsed.every(t => [...t].length <= 32);
  const changed = category.trim() !== material.metadata.category || JSON.stringify(parsed) !== JSON.stringify(material.metadata.tags);
  return <div className="classification">
    <label className="field"><span>カテゴリ</span><input list="material-categories" value={category} onChange={e => setCategory(e.target.value)} placeholder="未分類" maxLength={48} /></label>
    <datalist id="material-categories">{categories.map(c => <option key={c} value={c} />)}</datalist>
    <label className="field tag-entry"><span>タグ · カンマ区切り</span><input value={tags} onChange={e => setTags(e.target.value)} placeholder="暖色, ゆっくり, サビ" /></label>
    <button disabled={!valid || !changed} onClick={() => apply({ op: "set_material_metadata", material_kind: material.kind, material_id: material.id, metadata: { ...material.metadata, category: category.trim(), tags: parsed } })}>分類を保存</button>
    {!valid && <p className="error">カテゴリは48文字、タグは各32文字・24個までです。</p>}
  </div>;
}

export default function MaterialBrowser({ view, apply, onEdit, onPick, targetName, canPlace, onPlace, onImport }: {
  view: EditorView; apply: (c: EditCommand) => void;
  onEdit: (kind: MaterialKind, id: string) => void;
  onPick: (kind: MaterialKind, id: string) => void;
  targetName: string | null; canPlace: (kind: MaterialKind) => boolean;
  onPlace: (kind: MaterialKind, id: string) => void;
  /** Copy a library material into the edited show; the new view (null: failed, shown by the editor). */
  onImport: (path: string, kind: MaterialKind, id: string) => Promise<EditorView | null>;
}) {
  const [origin, setOrigin] = useState<"show" | "library">("show");
  const [library, setLibrary] = useState<LibraryEntry[]>([]);
  const [libraryError, setLibraryError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const loadLibrary = async (paths: string[]) => {
    // One failing file does not hide the others.
    setLibrary(await Promise.all(paths.map(async path => {
      try { return { path, view: await ipc.libraryList(path) }; } catch (e) { return { path, error: ipc.errorText(e) }; }
    })));
  };
  useEffect(() => { if (origin === "library") ipc.settingsGet().then(v => loadLibrary(v.settings.library_shows ?? [])).catch(e => setLibraryError(ipc.errorText(e))); }, [origin]);
  const addLibrary = async () => {
    setLibraryError(null);
    try {
      const path = await ipc.pickShow();
      if (path) await loadLibrary(await ipc.libraryAdd(path));
    } catch (e) { setLibraryError(ipc.errorText(e)); }
  };
  const removeLibrary = async (path: string) => {
    setLibraryError(null);
    try { await loadLibrary(await ipc.libraryRemove(path)); } catch (e) { setLibraryError(ipc.errorText(e)); }
  };
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [kind, setKind] = useState<"all" | MaterialKind>("all");
  const [favorite, setFavorite] = useState(false);
  const [tags, setTags] = useState<string[]>([]);
  const [page, setPage] = useState(0);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [moving, setMoving] = useState(false);
  const [previews, setPreviews] = useState<Record<string, number[][]>>({});
  const [previewError, setPreviewError] = useState<string | null>(null);
  const cache = useRef(new Map<string, number[][]>());
  // Metadata never invalidates pixels; nested preset edits invalidate compound previews.
  const showMaterials: Material[] = useMemo(() => [
    ...view.presets.map(p => ({ ...p, kind: "preset" as const, durationMs: 2000, label: p.effect_type.replace("builtin.", "").toUpperCase(), used: p.used_by_clips.length + p.used_by_layers.length, signature: JSON.stringify(p.effect) })),
    ...view.compounds.map(c => ({ ...c, kind: "compound" as const, durationMs: Math.max(1, Math.min(30000, c.length_us / 1000)), label: `COMPOUND · ${(c.length_us / 1e6).toFixed(1)}s`, used: c.used_by_clips.length, signature: JSON.stringify([c.layers, c.length_us, view.presets.map(p => [p.id, p.effect])]) })),
  ], [view]);
  // Library files are never written: the signature is the file stamp.
  const libraryMaterials: Material[] = useMemo(() => library.flatMap(({ path, view: lv }) => lv ? [
    ...lv.presets.map(p => ({ ...p, kind: "preset" as const, durationMs: 2000, label: p.effect_type.replace("builtin.", "").toUpperCase(), used: 0, signature: lv.stamp, source: path, sourceName: fileName(path) })),
    ...lv.compounds.map(c => ({ ...c, kind: "compound" as const, durationMs: Math.max(1, Math.min(30000, c.length_us / 1000)), label: `COMPOUND · ${(c.length_us / 1e6).toFixed(1)}s`, used: 0, signature: lv.stamp, source: path, sourceName: fileName(path) })),
  ] : []), [library]);
  const materials = origin === "show" ? showMaterials : libraryMaterials;
  const categories = [...new Set([...CATEGORIES, ...materials.map(m => m.metadata.category).filter(Boolean)])];
  const allTags = [...new Set(materials.flatMap(m => m.metadata.tags))].sort((a, b) => a.localeCompare(b, "ja"));
  const terms = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
  const filtered = materials.filter(m => {
    const text = [m.name, m.id, m.label, m.metadata.category, ...m.metadata.tags].join(" ").toLocaleLowerCase();
    return (kind === "all" || kind === m.kind) && (category === null || category === m.metadata.category) && (!favorite || m.metadata.favorite) && tags.every(t => m.metadata.tags.includes(t)) && terms.every(t => text.includes(t));
  });
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const safePage = Math.min(page, pages - 1);
  const visible = filtered.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE);
  const selected = materials.find(m => keyOf(m) === selectedKey);
  const previewKey = (m: Material) => JSON.stringify([m.source ?? view.path, view.preview_bpm, m.kind, m.id, m.signature]);
  const previewTargets = selected && !visible.some(m => keyOf(m) === selectedKey) ? [...visible, selected] : visible;
  const requestKey = JSON.stringify(previewTargets.map(previewKey));
  useEffect(() => { setPage(0); }, [query, category, kind, favorite, JSON.stringify(tags), origin]);
  // Library: copy into the show, then (optionally) place the copy.
  const importSelected = async (place: boolean) => {
    if (!selected?.source || importing) return;
    setImporting(true);
    try {
      const v = await onImport(selected.source, selected.kind, selected.id);
      const id = selected.kind === "preset" ? v?.outcome?.preset_id : v?.outcome?.compound_id;
      if (place && id) onPlace(selected.kind, id);
    } finally { setImporting(false); }
  };
  useEffect(() => {
    let cancelled = false;
    setPreviewError(null);
    setPreviews(Object.fromEntries(previewTargets.flatMap(m => {
      const frames = cache.current.get(previewKey(m));
      return frames ? [[keyOf(m), frames]] : [];
    })));
    // One request at a time. Only the current page is sampled, cache stays bounded.
    (async () => {
      for (const m of previewTargets) {
        if (cancelled) return;
        const k = previewKey(m);
        try {
          let f = cache.current.get(k);
          if (!f) {
            f = await (m.source ? ipc.libraryPreview(m.source, m.kind, m.id) : ipc.materialPreview(m.kind, m.id));
            if (cancelled) return;
            if (cache.current.size >= 96) cache.current.delete(cache.current.keys().next().value!);
            cache.current.set(k, f);
          }
          setPreviews(prev => ({ ...prev, [keyOf(m)]: f! }));
        } catch (e) { if (!cancelled) setPreviewError(ipc.errorText(e)); }
      }
    })();
    return () => { cancelled = true; };
  }, [requestKey]);
  const reset = () => { setQuery(""); setCategory(null); setKind("all"); setFavorite(false); setTags([]); setPage(0); };
  return <section className="panel material-bank" aria-label="素材バンク">
    <div className="bank-heading"><div><span className="eyebrow">MATERIAL BANK</span><h2>素材ライブラリー</h2></div><span className="bank-count mono">{String(filtered.length).padStart(2, "0")} <small>/ {materials.length}</small></span></div>
    <div className="bank-tools" role="group" aria-label="素材の出所">
      <button className={origin === "show" ? "on" : ""} aria-pressed={origin === "show"} onClick={() => { setOrigin("show"); setSelectedKey(null); }}>このショー</button>
      <button className={origin === "library" ? "on" : ""} aria-pressed={origin === "library"} onClick={() => { setOrigin("library"); setSelectedKey(null); }}>ライブラリー</button>
      {origin === "library" && <button onClick={addLibrary}>ライブラリーを追加…</button>}
    </div>
    {origin === "library" && <div className="bank-library">
      {library.length === 0 && <p className="hint">ライブラリーは未登録です。雛形 shows/omoi-iro-live-template.show.json を登録すると 31 素材が使えます。</p>}
      {library.map(e => <div key={e.path} className="bank-library-file"><span className="mono" title={e.path}>{fileName(e.path)}</span>{e.view && <small> {e.view.presets.length + e.view.compounds.length} 素材</small>} <button onClick={() => removeLibrary(e.path)}>外す</button>
        {e.error && <p role="alert" className="error">{fileName(e.path)}：{e.error}</p>}</div>)}
      {libraryError && <p role="alert" className="error">ライブラリー：{libraryError}</p>}
    </div>}
    <div className="bank-tools">
      <input className="material-search" aria-label="素材を検索" type="search" value={query} onChange={e => setQuery(e.target.value)} placeholder="名前・タグ・エフェクトを検索" />
      <select aria-label="素材の種類" value={kind} onChange={e => setKind(e.target.value as typeof kind)}><option value="all">すべての素材</option><option value="preset">プリセット</option><option value="compound">複合素材</option></select>
      <button className={favorite ? "on" : ""} aria-pressed={favorite} onClick={() => setFavorite(!favorite)}>お気に入り</button>
    </div>
    <div className="bank-body">
      <nav className="bank-categories" aria-label="素材カテゴリ">
        <span className="eyebrow">CATEGORY</span>
        {[null, ...categories, ""].map(c => <button key={c ?? "all"} className={category === c ? "on" : ""} aria-pressed={category === c} onClick={() => setCategory(c)}><span>{c === null ? "すべて" : c || "未分類"}</span><small className="mono">{materials.filter(m => c === null || m.metadata.category === c).length}</small></button>)}
      </nav>
      <div className="bank-results">
        {allTags.length > 0 && <div className="bank-tags" aria-label="タグ絞り込み"><span>TAG</span>{allTags.map(t => <button key={t} className={tags.includes(t) ? "on" : ""} aria-pressed={tags.includes(t)} onClick={() => setTags(prev => prev.includes(t) ? prev.filter(x => x !== t) : [...prev, t])}>{t}</button>)}</div>}
        <div className="material-grid">
          {visible.map((m, i) => <div key={keyOf(m)} className={`material-card ${selectedKey === keyOf(m) ? "selected" : ""}`}>
            <button className="material-select" aria-pressed={selectedKey === keyOf(m)} onClick={() => { setSelectedKey(keyOf(m)); setMoving(false); onPick(m.kind, m.id); }}>
              <span className="material-number mono">{String(safePage * PAGE_SIZE + i + 1).padStart(2, "0")} <span>{m.kind === "compound" ? "CP" : "FX"}</span></span>
              <MiniPreview frames={previews[keyOf(m)]} name={m.name} animate={false} />
              <span className="material-name" title={m.name}>{m.name}</span><span className="material-type mono">{m.label}</span>{m.sourceName && <small className="material-source mono" title={m.source}>{m.sourceName}</small>}
            </button>
            <div className="material-footer"><span>{m.metadata.category || "未分類"}</span><button disabled={!!m.source} className={m.metadata.favorite ? "favorite on" : "favorite"} aria-label={`${m.name}のお気に入り`} aria-pressed={m.metadata.favorite} onClick={() => apply({ op: "set_material_metadata", material_kind: m.kind, material_id: m.id, metadata: { ...m.metadata, favorite: !m.metadata.favorite } })}>FAV</button></div>
          </div>)}
        </div>
        {filtered.length === 0 && <div className="bank-empty"><p>条件に合う素材がありません</p><button onClick={reset}>絞り込みを解除</button></div>}
        <div className="bank-pagination"><span role="status">{filtered.length} 件 · タグはすべて一致</span><button disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>前へ</button><span className="mono">{safePage + 1} / {pages}</span><button disabled={safePage + 1 === pages} onClick={() => setPage(safePage + 1)}>次へ</button><button onClick={reset}>クリア</button></div>
      </div>
    </div>
    {selected ? <div className="material-detail">
      <div className="material-detail-title"><MiniPreview frames={previews[keyOf(selected)]} name={selected.name} animate={moving} durationMs={selected.durationMs} /><div className="selected-material-name"><span className="eyebrow">SELECTED MATERIAL</span><h3>{selected.name}</h3><span className="hint">{selected.label} · {selected.source ? `ライブラリー：${selected.sourceName}` : `使用 ${selected.used} 箇所`}</span></div><div className="actions">
        <button aria-pressed={moving} className={moving ? "on" : ""} onClick={() => setMoving(!moving)}>{moving ? "サムネイル停止" : "サムネイルを動かす"}</button>
        {selected.source ? <>
          <button disabled={importing} onClick={() => importSelected(false)}>このショーに取り込む</button>
          <button className="primary" disabled={importing || !canPlace(selected.kind)} onClick={() => importSelected(true)}>レイヤーに配置</button>
        </> : <>
          <button onClick={() => onEdit(selected.kind, selected.id)}>効果を編集</button>
          <button className="primary" disabled={!canPlace(selected.kind)} onClick={() => onPlace(selected.kind, selected.id)}>レイヤーに配置</button>
        </>}
      </div></div>
      <p className="hint">24コマのコマ送り（サンプル入力）。実際の再生は「レイヤーに配置」→ LEDプレビューの「手元で再生」で確認</p>
      {selected.source ? <p className="hint">ライブラリーの素材はこのショーへ複製して使います（同じ内容が既にあれば増やしません）。分類と効果の編集は取り込んだあと「このショー」で行います。ライブラリーのファイルは書き換えません。</p>
        : <Classification material={selected} categories={categories} apply={apply} />}
      <p className="hint">{targetName ? `配置先：${targetName}。再生位置、停止中は末尾に追加。` : "配置先は下のタイムラインでレイヤーを選択。"} 複合素材はショーのタイムラインに配置できます。</p>
    </div> : <p className="bank-select-hint">素材を選ぶと、分類の編集・動きの確認・レイヤーへの配置ができます。</p>}
    <p className="bank-footnote">プレビューはサンプル入力・ショーのBPMによる参考表示。音量連動素材は音声入力が必要です。</p>
    {previewError && <p role="alert" className="error">素材プレビュー：{previewError}</p>}
  </section>;
}
