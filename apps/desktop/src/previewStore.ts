// LED preview state shared by the LED preview panels and the timeline
// transport bar: one Channel subscription, one PreviewInfo. Components read it
// with `usePreview` (only they re-render per frame). The local clock never
// changes Live's or a device's position.
import { useSyncExternalStore } from "react";
import * as ipc from "./ipc/commands";
import type { PreviewFrame, PreviewInfo, PreviewSource } from "./ipc/types";

export interface PreviewState {
  frame: PreviewFrame | null;
  info: PreviewInfo | null;
  error: string | null;
}

let state: PreviewState = { frame: null, info: null, error: null };
const listeners = new Set<() => void>();
let subscribed = false;
let pending: PreviewFrame | null = null;
let raf = 0;

function set(patch: Partial<PreviewState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

// Subscribe once; draw the newest frame at the next animation frame, then ack.
function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  ipc
    .previewSubscribe((f) => {
      pending = f;
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        const p = pending;
        if (!p) return;
        set({ frame: p });
        ipc.previewAck(p.seq).catch(() => {});
      });
    })
    .catch((e) => {
      subscribed = false;
      set({ error: ipc.errorText(e) });
    });
}

function subscribe(l: () => void) {
  ensureSubscribed();
  listeners.add(l);
  return () => listeners.delete(l);
}

export function usePreview(): PreviewState {
  return useSyncExternalStore(subscribe, () => state);
}

export const previewState = () => state;

async function call(f: () => Promise<PreviewInfo>): Promise<PreviewInfo | null> {
  try {
    const i = await f();
    set({ info: i, error: null });
    return i;
  } catch (e) {
    set({ error: ipc.errorText(e) });
    return null;
  }
}

/** Load a show into the local clock ("" = the edited new show). `local`: also show it. */
export async function open(showPath: string, local = false) {
  const i = await call(() => ipc.previewOpen(showPath));
  if (i && local && i.mode !== "local") await call(() => ipc.previewMode("local"));
  return i;
}
export const setMode = (m: PreviewSource) => call(() => ipc.previewMode(m));
/** Play / seek use the local clock: while the runtime is followed this is the independent preview. */
async function toLocal() {
  if (state.info?.mode !== "local") await setMode("local");
}
export async function play(playing: boolean) {
  await toLocal();
  return call(() => ipc.previewPlay(playing));
}
export async function seek(us: number) {
  await toLocal();
  return call(() => ipc.previewSeek(Math.max(0, Math.round(us))));
}
/** Pause and go back to the start. */
export async function stop() {
  await toLocal();
  await call(() => ipc.previewPlay(false));
  return call(() => ipc.previewSeek(0));
}
export const setBpm = (bpm: number) => call(() => ipc.previewSetBpm(bpm));

/** Position shown: the newest frame's show time, else the local clock. */
export function positionUs(s: PreviewState): number | null {
  return s.frame?.show_time_us ?? s.info?.position_us ?? null;
}

/** `m:ss.ff` with 30 fps frames. */
export function fmtFrames(us: number | null): string {
  if (us === null) return "-:--.--";
  const total = Math.max(0, Math.floor(us));
  const sec = Math.floor(total / 1e6);
  const ff = Math.floor(((total % 1e6) * 30) / 1e6);
  return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, "0")}.${String(ff).padStart(2, "0")}`;
}
