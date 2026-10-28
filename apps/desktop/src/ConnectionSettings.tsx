// U02: connection and sync settings (terminal settings, not the show).
// Checking lists ports and sends nothing; the receive check only listens.
import { useCallback, useEffect, useState } from "react";
import * as ipc from "./ipc/commands";
import type { ListenResult, PortRole, RoleReport, SettingsCheck, SettingsView, TerminalSettings } from "./ipc/types";

const ROLES: { role: PortRole; label: string; dir: "input" | "output"; hint: string }[] = [
  { role: "control_in", label: "操作入力", dir: "input", hint: "パッドなどの操作。Note / CC を Live へ転送する" },
  { role: "control_out", label: "Live操作出力", dir: "output", hint: "Live の Control バス（Remote / Track ON）" },
  { role: "clock_in", label: "MIDI Clock入力", dir: "input", hint: "拍源が midi_clock のとき" },
  { role: "led_out", label: "LED出力", dir: "output", hint: "空欄 = 仮想 Launchpad X。実機は M6" },
  { role: "timecode_out", label: "タイムコード出力", dir: "output", hint: "show の mtc_output が有効なとき MTC を送る" },
];

const STATE_LABEL: Record<string, string> = {
  unused: "未使用",
  virtual: "仮想",
  ok: "OK",
  missing: "なし",
  fault: "Fault",
  invalid: "拒否",
};

function badgeClass(state: string): string {
  if (state === "ok" || state === "virtual") return "badge good";
  if (state === "unused") return "badge";
  if (state === "missing") return "badge warn";
  return "badge bad";
}

function listenText(r: ListenResult): string {
  const c = r.counts;
  const parts = [
    ["Clock", c.midi_clock],
    ["Start/Stop/SPP", c.transport],
    ["MTC QF", c.mtc_quarter_frame],
    ["SysEx", c.sysex],
    ["Note", c.note],
    ["CC", c.control_change],
    ["その他", c.other],
  ]
    .filter(([, n]) => (n as number) > 0)
    .map(([k, n]) => `${k} ${n}`);
  return `${r.port}：${(r.duration_ms / 1000).toFixed(0)} 秒で ${c.total} 件${parts.length ? `（${parts.join("、")}）` : ""}${r.dropped ? `、破棄 ${r.dropped}` : ""}`;
}

export default function ConnectionSettings(props: { running: boolean }) {
  const { running } = props;
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<TerminalSettings | null>(null);
  const [check, setCheck] = useState<SettingsCheck | null>(null);
  const [listen, setListen] = useState<Partial<Record<PortRole, string>>>({});
  const [listening, setListening] = useState<PortRole | null>(null);
  const [message, setMessage] = useState<{ error: boolean; text: string } | null>(null);

  useEffect(() => {
    ipc
      .settingsGet()
      .then((v) => {
        setView(v);
        setDraft(v.settings);
      })
      .catch((e) => setMessage({ error: true, text: ipc.errorText(e) }));
  }, []);

  const runCheck = useCallback(async (s: TerminalSettings) => {
    try {
      setCheck(await ipc.settingsCheck(s));
    } catch (e) {
      setMessage({ error: true, text: ipc.errorText(e) });
    }
  }, []);

  // Re-check (enumeration only) shortly after each edit.
  useEffect(() => {
    if (!draft) return;
    const t = window.setTimeout(() => runCheck(draft), 250);
    return () => window.clearTimeout(t);
  }, [draft, runCheck]);

  const setPort = (role: PortRole, value: string) => {
    setDraft((d) => (d ? { ...d, ports: { ...d.ports, [role]: value === "" ? null : value } } : d));
  };

  const save = async () => {
    if (!draft) return;
    try {
      const v = await ipc.settingsSave(draft);
      setView(v);
      setDraft(v.settings);
      setMessage({ error: false, text: running ? "保存した。次の開始から有効" : "保存した" });
    } catch (e) {
      setMessage({ error: true, text: ipc.errorText(e) });
    }
  };

  const doListen = async (role: "control_in" | "clock_in") => {
    if (!draft) return;
    setListening(role);
    try {
      const r = await ipc.settingsListen(draft, role, 3000);
      setListen((l) => ({ ...l, [role]: listenText(r) }));
    } catch (e) {
      setListen((l) => ({ ...l, [role]: ipc.errorText(e) }));
    } finally {
      setListening(null);
    }
  };

  if (!draft) return <section className="panel"><h2>接続・同期設定</h2><p className="hint">読み込み中…</p></section>;
  const report = (role: PortRole): RoleReport | undefined => check?.report.roles.find((r) => r.role === role);

  return (
    <section className="panel">
      <h2>接続・同期設定（この端末）</h2>
      {view?.load_error && <p className="error">設定ファイルを読めなかった（保存するまで既定値）：{view.load_error}</p>}
      <datalist id="inputs">{check?.inputs.map((n) => <option key={n} value={n} />)}</datalist>
      <datalist id="outputs">{check?.outputs.map((n) => <option key={n} value={n} />)}</datalist>
      <table className="roles">
        <thead>
          <tr>
            <th>役割</th>
            <th>ポート（完全一致名 か #番号）</th>
            <th>状態</th>
          </tr>
        </thead>
        <tbody>
          {ROLES.map(({ role, label, dir, hint }) => {
            const r = report(role);
            return (
              <tr key={role}>
                <td>
                  <div>{label}</div>
                  <div className="hint">{dir === "input" ? "入力" : "出力"} · {hint}</div>
                </td>
                <td>
                  <input
                    list={dir === "input" ? "inputs" : "outputs"}
                    value={draft.ports[role] ?? ""}
                    placeholder={role === "led_out" ? "仮想 Launchpad X" : "なし"}
                    onChange={(e) => setPort(role, e.target.value)}
                    spellCheck={false}
                  />
                  {(role === "control_in" || role === "clock_in") && (
                    <button className="small" disabled={running || listening !== null || !draft.ports[role]} onClick={() => doListen(role)}>
                      {listening === role ? "受信中…" : "受信確認 3秒"}
                    </button>
                  )}
                  {listen[role] && <div className="hint">{listen[role]}</div>}
                </td>
                <td>
                  <span className={badgeClass(r?.state ?? "unused")}>{STATE_LABEL[r?.state ?? "unused"]}</span>
                  {r?.resolved && r.resolved !== r.spec && <div className="hint">→ {r.resolved}</div>}
                  {r?.message && <div className={r.state === "invalid" || r.state === "fault" ? "reason bad" : "reason"}>{r.message}</div>}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className="grid settings-misc">
        <label className="field">
          <span>ブリッジ受信（UDP、IP:ポート）</span>
          <input value={draft.bridge_bind} onChange={(e) => setDraft({ ...draft, bridge_bind: e.target.value })} spellCheck={false} />
        </label>
        <label className="field check">
          <input type="checkbox" checked={draft.link} disabled={!view?.link_available} onChange={(e) => setDraft({ ...draft, link: e.target.checked })} />
          <span>拍源が ableton_link の show で Link を使う{view && !view.link_available ? "（このビルドは非対応）" : "（Start Stop Sync は使わない）"}</span>
        </label>
      </div>
      <div className="safety-settings">
        <div className="insp-title">機器への出力（この端末）</div>
        <label className="field check">
          <input
            type="checkbox"
            checked={draft.devices.allow_launchpad_led}
            onChange={(e) => setDraft({ ...draft, devices: { ...draft.devices, allow_launchpad_led: e.target.checked } })}
          />
          <span>Launchpad X へ LED を送る（LED出力に Launchpad を指定できるようにする）</span>
        </label>
        {draft.devices.allow_launchpad_led && (
          <p className="note">
            Launchpad X へ直接 LED を送ります。Live の Control Surface に Launchpad を割り当てないでください。接続ごとに機器を確認（Device Inquiry）してから送ります。操作入力にも同じ Launchpad を指定してください。
          </p>
        )}
      </div>
      <div className="safety-settings">
        <div className="insp-title">安全（この端末）</div>
        <div className="grid">
          <label className="field">
            <span>輝度の上限 {Math.round(draft.safety.max_brightness * 100)}%（各色の天井）</span>
            <input type="range" min={0.05} max={1} step={0.05} value={draft.safety.max_brightness} onChange={(e) => setDraft({ ...draft, safety: { ...draft.safety, max_brightness: Number(e.target.value) } })} />
            <span className="hint">0.05〜1。送信の直前に各チャンネルをこの値で切る（全体を暗くする係数ではない）</span>
          </label>
          <label className="field">
            <span>点滅の上限（Hz）</span>
            <input type="number" min={0.5} max={30} step={0.5} value={draft.safety.max_flash_hz} onChange={(e) => setDraft({ ...draft, safety: { ...draft.safety, max_flash_hz: Number(e.target.value) } })} />
            <span className="hint">0.5〜30。既定 3 Hz（これより速い全面の明滅を高速点滅とみなす）</span>
          </label>
          <label className="field check">
            <input type="checkbox" checked={draft.safety.limit_flash} onChange={(e) => setDraft({ ...draft, safety: { ...draft.safety, limit_flash: e.target.checked } })} />
            <span>上限を超えるレイヤーを消す（不透明度 0 として描画。show は書き換えない）</span>
          </label>
        </div>
      </div>
      {check && check.report.errors.length > 0 && (
        <ul className="issues bad">{check.report.errors.map((e) => <li key={e}>{e}</li>)}</ul>
      )}
      {check && check.report.warnings.length > 0 && (
        <ul className="issues">{check.report.warnings.map((e) => <li key={e}>{e}</li>)}</ul>
      )}
      <div className="actions">
        <button className="primary" disabled={!check?.report.ok} onClick={save}>保存</button>
        <button onClick={() => runCheck(draft)}>ポートを再列挙</button>
        <span className="hint">{check ? `${check.backend} · 入力 ${check.inputs.length} · 出力 ${check.outputs.length}（列挙のみ、何も送らない）` : ""}</span>
      </div>
      {message && <p className={message.error ? "error" : "note ok"}>{message.text}</p>}
      {view?.path && <p className="hint">保存先：{view.path}</p>}
    </section>
  );
}
