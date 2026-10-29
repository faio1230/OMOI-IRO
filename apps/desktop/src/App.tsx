// OMOI-IRO main window. Two tabs: 演奏 (U06: show, clock, sync health per
// source, faults and reconnect, Blackout, live diagnostics, LED preview) and
// 接続設定 (U02). The window only displays state; the show clock, rendering
// and MIDI run in the Rust runtime and keep running while this view is busy
// or minimized.
import { useCallback, useEffect, useRef, useState } from "react";
import * as ipc from "./ipc/commands";
import ConnectionSettings from "./ConnectionSettings";
import LedPreview from "./LedPreview";
import TimelineEditor from "./TimelineEditor";
import InputPanel from "./InputPanel";
import UnsavedChoice from "./UnsavedChoice";
import * as preview from "./previewStore";
import type { AppInfo, LiveDiag, StartRequest, StatusDto, TimeSource, TimingSummary } from "./ipc/types";

const STATUS_POLL_MS = 200;
const MAX_EVENTS = 50;

type Tab = "perform" | "timeline" | "connections";
type Tone = "good" | "warn" | "bad" | "idle";

function fmtShowTime(us: number | null): string {
  if (us === null) return "--";
  const s = us / 1e6;
  const m = Math.floor(s / 60);
  return `${m}:${(s - m * 60).toFixed(3).padStart(6, "0")}`;
}

const ms = (us: number) => (us / 1000).toFixed(us < 10_000 ? 2 : 1);

function Tile(props: { title: string; tone: Tone; value: string; detail?: (string | null | undefined)[]; children?: React.ReactNode }) {
  return (
    <div className={`tile ${props.tone}`}>
      <div className="tile-title">{props.title}</div>
      <div className="tile-value">{props.value}</div>
      {props.detail?.filter(Boolean).map((d) => (
        <div className="tile-detail" key={d!}>{d}</div>
      ))}
      {props.children}
    </div>
  );
}

function bindingTone(state: string | null | undefined): Tone {
  if (!state) return "idle";
  if (state === "ready" || state === "virtual") return "good";
  if (state === "fault") return "bad";
  return "warn";
}

const STATE_JA: Record<string, string> = {
  ready: "接続",
  virtual: "仮想",
  recovering: "再接続待ち",
  disconnected: "未接続",
  fault: "Fault",
};
const ja = (s: string | null | undefined) => (s ? STATE_JA[s] ?? s : "--");

function Health({ s, onReset, onReconnect, busy }: { s: StatusDto; onReset: () => void; onReconnect: () => void; busy: boolean }) {
  const timeTone: Tone = !s.running ? "idle" : s.time_health === "locked" ? "good" : s.time_health === "holdover" || s.time_health === "acquiring" ? "warn" : "bad";
  const beatTone: Tone = !s.running || s.beat_health === "unselected" ? "idle" : s.beat_health === "locked" ? "good" : s.beat_health === "holdover" || s.beat_health === "unknown" ? "warn" : "bad";
  const c = s.control;
  const controlTone: Tone = !c ? "idle" : c.fault ? "bad" : c.output === "ready" && c.input === "ready" ? "good" : "warn";
  const t = s.timecode_out;
  const local = s.running && s.time_source === "local" ? s.local : null;
  return (
    <section className="panel">
      <h2>同期と接続の状態</h2>
      <div className="tiles">
        <Tile
          title={local ? "時刻源" : "時刻源（ブリッジ）"}
          tone={timeTone}
          value={local ? `手元の時計（${local.playing ? "再生中" : "停止中"}）` : s.running ? `${s.time_health}${s.stalled ? "（停滞）" : ""}` : "--"}
          detail={[s.running ? `transport ${s.transport} · 出力 ${s.output} ${s.gain.toFixed(2)}` : null, s.dark ? `暗転の理由：${s.dark}` : null]}
        />
        <Tile title="拍源" tone={beatTone} value={local ? `手元（${local.bpm} bpm）` : s.running ? s.beat_health : "--"} detail={[s.beat ? `${s.beat.beat.toFixed(2)} 拍 · ${s.beat.tempo_bpm.toFixed(1)} bpm${s.beat.phase_valid ? "" : "（位相未確定）"}` : null, s.beat_seeded ? "bridge の拍で原点を補完" : null]} />
        <Tile title="LED出力" tone={s.render_panics > 0 ? "bad" : bindingTone(s.led.state)} value={ja(s.led.state)} detail={[s.led.generation !== null ? `接続世代 ${s.led.generation}` : null, s.running ? `点灯 ${s.led.lit}` : null, s.led.error, s.render_panics > 0 ? `描画の異常 ${s.render_panics} フレーム（前のフレームを保持。最後：${s.render_fault ?? ""}）` : null]} />
        <Tile
          title="操作経路（→ Live）"
          tone={controlTone}
          value={!c ? "未使用" : c.fault ? "Fault" : `${ja(c.input)} → ${ja(c.output)}`}
          detail={[c?.fault ? `原因：${c.fault}` : null, c && c.pending_note_offs > 0 ? `保留中の Note Off ${c.pending_note_offs}` : null, c?.input_error, c?.output_error]}
        >
          {c?.fault && (
            <button className="small" onClick={onReset}>fault を解除</button>
          )}
        </Tile>
        <Tile title="タイムコード出力" tone={t ? bindingTone(t.state) : "idle"} value={t ? ja(t.state) : "未使用"} detail={[t?.generation != null ? `接続世代 ${t.generation}` : null, t?.error]} />
        <Tile
          title="安全"
          tone={s.safety.warnings.length ? "warn" : "good"}
          value={`警告 ${s.safety.warnings.length} 件`}
          detail={[`輝度上限 ${Math.round(s.safety.max_brightness * 100)}% · 点滅上限 ${s.safety.max_flash_hz} Hz`, s.safety.limit_flash ? `制限 ON${s.safety.muted_layers.length ? `（消灯中：${s.safety.muted_layers.join(", ")}）` : ""}` : "制限 OFF（警告のみ）"]}
        />
      </div>
      {s.safety.warnings.length > 0 && (
        <ul className="issues">
          {s.safety.warnings.map((w) => (
            <li key={`${w.layer_id}-${w.preset_id}`}>{w.layer_name} / {w.preset_name}：{w.reason}{w.clips.length ? `（クリップ ${w.clips.join(", ")}）` : ""}</li>
          ))}
        </ul>
      )}
      <div className="actions">
        <button disabled={!s.running || busy} onClick={onReconnect}>全ポートを再接続</button>
        <span className="hint">鳴っている note に Note Off を送ってから、LED・操作経路・タイムコード出力を開き直す（新しい接続世代）。抜き差しは自動で再接続する。</span>
      </div>
    </section>
  );
}

function Row({ label, t }: { label: string; t: TimingSummary | undefined }) {
  return (
    <tr>
      <td>{label}</td>
      <td className="num">{t?.n ?? 0}</td>
      <td className="num">{t && t.n ? ms(t.p50_us) : "--"}</td>
      <td className="num">{t && t.n ? ms(t.p95_us) : "--"}</td>
      <td className="num">{t && t.n ? ms(t.p99_us) : "--"}</td>
      <td className="num">{t && t.n ? ms(t.max_us) : "--"}</td>
    </tr>
  );
}

function Diagnostics({ s }: { s: StatusDto }) {
  const d: LiveDiag | null = s.diag;
  return (
    <section className="panel">
      <h2>診断（開始からの累計、約1秒ごとに更新）</h2>
      <table className="diag mono">
        <thead>
          <tr>
            <th>時間（ms）</th>
            <th>件数</th>
            <th>p50</th>
            <th>p95</th>
            <th>p99</th>
            <th>max</th>
          </tr>
        </thead>
        <tbody>
          <Row label="frame の遅れ（jitter）" t={d?.jitter} />
          <Row label="render" t={d?.render} />
          <Row label="LED 送信" t={d?.led_send} />
          <Row label="render → LED 送信完了" t={d?.render_to_sent} />
          <Row label="受信 callback → Live へ送信" t={d?.callback_to_send} />
        </tbody>
      </table>
      <div className="counters mono">
        <span>frame {d?.frames ?? 0}</span>
        <span>飛ばした frame {d?.skipped_frames ?? 0}</span>
        <span>位置の飛び {d?.jumps ?? 0}</span>
        <span>mailbox 上書き {d?.mailbox_overwritten ?? 0}</span>
        <span>LED 送信 {d?.led_frames ?? 0} / 失敗 {d?.led_errors ?? 0}</span>
        <span>Live へ転送 {d?.control_sent ?? 0} / 失敗 {d?.control_errors ?? 0}</span>
        <span>MTC 送信 {d?.mtc_sent ?? 0} / 失敗 {d?.mtc_send_errors ?? 0}</span>
        <span>波紋の発生 {d?.ripple_spawned ?? 0}</span>
        <span>破棄 bridge {s.dropped.bridge} · clock {s.dropped.clock} · 操作 {s.dropped.control}</span>
        <span>Clock 原点の補完 {s.clock_seeded}</span>
        <span>プレビュー間引き {s.preview_skipped}</span>
      </div>
    </section>
  );
}

export default function App() {
  const [tab, setTab] = useState<Tab>("perform");
  // Header "新規…": handed to the timeline editor, which owns the unsaved-changes question.
  const [newRequested, setNewRequested] = useState(false);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [req, setReq] = useState<StartRequest>({ show_path: "" });
  const [status, setStatus] = useState<StatusDto | null>(null);
  const [events, setEvents] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [errorList, setErrorList] = useState<{ pointer: string; message: string }[]>([]);
  const [recent, setRecent] = useState<string[]>([]);
  const [saveNote, setSaveNote] = useState<string | null>(null);
  const [notes, setNotes] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const polling = useRef(false);

  useEffect(() => {
    ipc
      .appInfo()
      .then((i) => {
        setInfo(i);
        const initial = i.initial_show_path;
        if (initial) setReq((r) => ({ ...r, show_path: initial }));
      })
      .catch((e) => setError(ipc.errorText(e)));
  }, []);

  useEffect(() => {
    const timer = window.setInterval(async () => {
      if (polling.current) return;
      polling.current = true;
      try {
        const s = await ipc.runtimeStatus();
        setStatus(s);
        // A21: the transport bar drives the runtime's clock while it runs locally.
        preview.setRuntimeLocal(s.running && s.time_source === "local" ? s.local : null);
        if (s.events.length > 0) setEvents((prev) => [...prev, ...s.events].slice(-MAX_EVENTS));
      } catch (e) {
        setError(ipc.errorText(e));
      } finally {
        polling.current = false;
      }
    }, STATUS_POLL_MS);
    return () => window.clearInterval(timer);
  }, []);

  const running = status?.running ?? false;

  const run = useCallback(async (f: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    setErrorList([]);
    try {
      await f();
    } catch (e) {
      setError(ipc.errorText(e));
      setErrorList(ipc.errorDetails(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const refreshRecent = useCallback(() => {
    ipc.settingsGet().then((v) => setRecent(v.settings.recent_shows)).catch(() => {});
  }, []);
  useEffect(refreshRecent, [refreshRecent]);
  // A21: offer the time source chosen at the last start.
  useEffect(() => {
    ipc.settingsGet().then((v) => setReq((r) => ({ ...r, time_source: v.settings.last_time_source ?? "live" }))).catch(() => {});
  }, []);

  // Open a show for editing (and as the show to start). A file that fails to
  // load leaves the current show as it is; the reasons come with JSON Pointers.
  // Unsaved changes: ask (save and open / open without saving / cancel).
  const [pendingOpen, setPendingOpen] = useState<string | null>(null);
  const opened = (path: string) => {
    setReq((r) => ({ ...r, show_path: path }));
    setSaveNote(`開いた：${path}`);
    refreshRecent();
  };
  const openShow = (path: string) =>
    run(async () => {
      try {
        await ipc.editorOpen(path);
      } catch (e) {
        if (!ipc.isUnsaved(e)) throw e;
        setPendingOpen(path);
        return;
      }
      opened(path);
    });
  const resolvePendingOpen = (choice: "save" | "discard") => {
    const path = pendingOpen;
    if (!path) return;
    run(async () => {
      if (choice === "save") {
        // A new show has no file yet: "save as" first; cancelling the dialog keeps the question.
        if (status?.document?.path) {
          await ipc.editorSave();
        } else {
          const p = await ipc.pickShowSave();
          if (!p) return;
          await ipc.editorSaveAs(p);
        }
      }
      await ipc.editorOpen(path, choice === "discard");
      setPendingOpen(null);
      opened(path);
    });
  };
  const pickAndOpen = () =>
    run(async () => {
      const p = await ipc.pickShow();
      if (p) await openShow(p);
    });
  const saveDoc = () =>
    run(async () => {
      if (!status?.document?.path) {
        await saveDocAs();
        return;
      }
      const v = await ipc.editorSave();
      setSaveNote(`保存した：${v.path}`);
    });
  const saveDocAs = () =>
    run(async () => {
      const p = await ipc.pickShowSave(status?.document?.path);
      if (!p) return;
      const v = await ipc.editorSaveAs(p);
      setReq((r) => ({ ...r, show_path: v.path }));
      setSaveNote(`保存した：${v.path}`);
      refreshRecent();
    });

  // Ctrl+S / ⌘S saves the open document (with Shift: save as).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (status?.document) (e.shiftKey ? saveDocAs : saveDoc)();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // A new show without a file starts with the edited content (no path).
  const newDoc = status?.document?.path === "";
  const showPath = newDoc ? "" : req.show_path;
  const start = () =>
    run(async () => {
      const r = await ipc.runtimeStart({ ...req, show_path: showPath });
      setNotes(r.notes);
      setEvents([]);
    });
  const stop = () => run(() => ipc.runtimeStop());

  const s = status;
  return (
    <div className="app">
      <header>
        <h1>OMOI-IRO</h1>
        <span className="sub mono">LIVE VISUAL CONTROL {info ? ` / v${info.version}` : ""}</span>
        <nav className="tabs">
          <button className={tab === "perform" ? "on" : ""} onClick={() => setTab("perform")}>演奏</button>
          <button className={tab === "timeline" ? "on" : ""} onClick={() => setTab("timeline")}>素材・タイムライン</button>
          <button className={tab === "connections" ? "on" : ""} onClick={() => setTab("connections")}>接続設定</button>
        </nav>
        <span className="doc">
          {status?.document && (
            <span className={status.document.dirty ? "badge warn" : "badge"} title={status.document.path}>
              {status.document.dirty ? "● 未保存" : "保存済み"} · {status.document.path ? status.document.path.split(/[\\/]/).pop() : "新規ショー"}
            </span>
          )}
          <button className="small" disabled={busy} onClick={() => { setTab("timeline"); setNewRequested(true); }} title="素材・タイムラインで新しいショーを作る">新規…</button>
          {status?.document && (
            <>
              <button className="small" disabled={busy} onClick={saveDoc} title="Ctrl+S">保存</button>
              <button className="small" disabled={busy} onClick={saveDocAs} title="Ctrl+Shift+S">名前を付けて保存…</button>
            </>
          )}
        </span>
        <span className={running ? "pill on" : "pill"}>{running ? "RUNTIME 動作中" : "RUNTIME 停止"}</span>
      </header>
      {status?.close_requested && (
        <div className="modal-back">
          <div className="modal">
            <h2>未保存の変更があります</h2>
            <p>{status.document?.path}</p>
            <div className="actions">
              <button className="primary" onClick={() => run(() => ipc.closeDecision("save"))}>保存して終了</button>
              <button className="danger" onClick={() => run(() => ipc.closeDecision("discard"))}>保存せずに終了</button>
              <button onClick={() => run(() => ipc.closeDecision("cancel"))}>キャンセル</button>
            </div>
            {error && <p className="error">{error}</p>}
          </div>
        </div>
      )}

      {pendingOpen && (
        <UnsavedChoice
          action="開く"
          path={status?.document?.path || "新規ショー（未保存）"}
          error={error}
          onSave={() => resolvePendingOpen("save")}
          onDiscard={() => resolvePendingOpen("discard")}
          onCancel={() => setPendingOpen(null)}
        />
      )}

      {tab === "connections" ? (
        <ConnectionSettings running={running} />
      ) : tab === "timeline" ? (
        <div className="columns editor-columns">
          <div className="col">
            <TimelineEditor showPath={req.show_path} playheadUs={running ? s?.show_time_us ?? null : null} running={running} docKey={`${s?.document?.path ?? ""}|${s?.document?.dirty ?? ""}`} newRequested={newRequested} onNewHandled={() => setNewRequested(false)} />
          </div>
          <div className="col">
            <LedPreview showPath={showPath} running={running} onError={setPreviewError} />
            {previewError && <p className="error">{previewError}</p>}
          </div>
        </div>
      ) : (
        <div className="columns">
          <div className="col">
            <section className="panel">
              <h2>ショー</h2>
              <div className="show-row">
                <label className="field grow">
                  <span>ショーファイル（ポートとブリッジは「接続設定」）</span>
                  {newDoc ? (
                    <input value="新規ショー（未保存）— 編集中の内容で開始" readOnly disabled />
                  ) : (
                    <input value={req.show_path} placeholder="ショーファイル（.show.json）を選ぶか、新規ショーをそのまま開始" disabled={running} spellCheck={false} onChange={(e) => setReq((r) => ({ ...r, show_path: e.target.value }))} />
                  )}
                </label>
                <button disabled={busy} onClick={pickAndOpen}>開く…</button>
                <label className="inline">
                  時刻源
                  <select value={req.time_source ?? "live"} disabled={busy || running} onChange={(e) => setReq((r) => ({ ...r, time_source: e.target.value as TimeSource }))}>
                    <option value="live">Live に追従</option>
                    <option value="local">手元の時計（Live なし）</option>
                  </select>
                </label>
                <button className="primary" disabled={busy || running} onClick={start}>開始</button>
                <button disabled={busy || !running} onClick={stop}>停止</button>
              </div>
              {recent.length > 0 && (
                <label className="inline recent">
                  最近使ったショー
                  <select value="" onChange={(e) => e.target.value && openShow(e.target.value)} disabled={busy}>
                    <option value="">選ぶ…</option>
                    {recent.map((r) => <option key={r} value={r}>{r}</option>)}
                  </select>
                </label>
              )}
              {saveNote && <p className="hint">{saveNote}</p>}
              {notes.map((n) => (
                <p className="note" key={n}>{n}</p>
              ))}
              {error && <p className="error">{error}</p>}
              {errorList.length > 0 && (
                <ul className="issues bad mono">
                  {errorList.map((d, i) => <li key={i}><b>{d.pointer}</b> {d.message}</li>)}
                </ul>
              )}
            </section>

            <section className="panel clock-panel">
              <div className="clock">
                <div>
                  <span className="label">タイムコード</span>
                  <span className="huge mono">{s?.timecode ?? "--:--:--:--"}</span>
                </div>
                <div>
                  <span className="label">ショー時刻</span>
                  <span className="big mono">{fmtShowTime(s?.show_time_us ?? null)}</span>
                </div>
                <div>
                  <span className="label">拍</span>
                  <span className="big mono">{s?.beat ? `${s.beat.beat.toFixed(2)}` : "--"}</span>
                </div>
                <div className="blackout-box">
                  <button className={s?.blackout ? "blackout on" : "blackout"} disabled={!running} onClick={() => run(() => ipc.setBlackout(!s?.blackout))}>
                    {s?.blackout ? "BLACKOUT 中（解除）" : "BLACKOUT"}
                  </button>
                  <label className="slider">
                    <span>明るさ {Math.round((s?.brightness ?? 1) * 100)}%（上限 {Math.round((s?.safety.max_brightness ?? 1) * 100)}%）</span>
                    <input type="range" min={0} max={100} value={Math.round((s?.brightness ?? 1) * 100)} disabled={!running} onChange={(e) => run(() => ipc.setBrightness(Number(e.target.value) / 100))} />
                  </label>
                </div>
              </div>
              <p className="hint">Blackout と明るさは LED だけに作用する。Live への操作の転送は止めない。 frame {s?.frame ?? 0} · rev {s?.revision ?? 0} · uptime {((s?.uptime_ms ?? 0) / 1000).toFixed(1)} s</p>
            </section>

            <InputPanel running={running} recordingSinceUs={s?.recording_since_us ?? null} audio={s?.audio ?? null} docKey={`${s?.document?.path ?? ""}|${s?.document?.dirty ?? ""}`} run={run} />
            {s && <Health s={s} busy={busy} onReset={() => run(() => ipc.resetControlFault())} onReconnect={() => run(() => ipc.reconnectPorts())} />}
            {s && <Diagnostics s={s} />}

            <section className="panel">
              <h2>イベント</h2>
              <pre className="events mono">{events.length ? events.join("\n") : "（なし）"}</pre>
            </section>
          </div>
          <div className="col">
            <LedPreview showPath={showPath} running={running} onError={setPreviewError} />
            {previewError && <p className="error">{previewError}</p>}
          </div>
        </div>
      )}
    </div>
  );
}
