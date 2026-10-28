"""MTC timing residual from an `omoiiro-probe monitor` JSONL (M6 C4).

Live's song time advances 1:1 with real time whatever the tempo, so in each
continuous MTC run (between Full Frames) quarter frame k should arrive at
t0 + k * (1 / (4 * fps)). Fits t0 and the slope per run and reports the
residual of every quarter frame. Synthetic-free: reads a recorded file only.

    python tools/m6/mtc_residual.py artifacts/m6/win-m6-c4-mon.jsonl [--fps 30]
"""
import argparse
import json


def runs(path):
    run, out = [], []
    for line in open(path, encoding="utf-8"):
        r = json.loads(line)
        if r.get("source") != "mtc":
            continue
        if r["kind"] == "event" and r["payload"]["event"].startswith("full_frame"):
            if run:
                out.append(run)
            run = []
        elif r["kind"] == "midi" and r["payload"]["bytes"].startswith("F1"):
            run.append(r["t_mono_us"])
    if run:
        out.append(run)
    return out


def fit(ts, step):
    n = len(ts)
    ks = range(n)
    mk = (n - 1) / 2
    mt = sum(ts) / n
    sxx = sum((k - mk) ** 2 for k in ks)
    slope = sum((k - mk) * (t - mt) for k, t in zip(ks, ts)) / sxx
    res = [t - (mt + slope * (k - mk)) for k, t in zip(ks, ts)]
    return slope / step, res


def p(xs, q):
    s = sorted(xs)
    return s[min(len(s) - 1, int(q * len(s)))]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--fps", type=float, default=30.0)
    ap.add_argument("--min-qf", type=int, default=240)
    ap.add_argument("--bin-seconds", type=float, default=0,
                    help="summarize per time bin (by run start) instead of per run, e.g. 300 for a soak")
    a = ap.parse_args()
    step = 1e6 / (4 * a.fps)
    allres = []
    bins = {}
    skipped = 0
    for i, ts in enumerate(runs(a.file)):
        if len(ts) < a.min_qf:
            skipped += 1
            if not a.bin_seconds:
                print(f"run {i}: {len(ts)} QF (skipped, shorter than {a.min_qf})")
            continue
        rate, res = fit(ts, step)
        absr = [abs(x) for x in res]
        allres += absr
        gaps = [b - x for x, b in zip(ts, ts[1:])]
        if a.bin_seconds:
            b = bins.setdefault(int(ts[0] / 1e6 // a.bin_seconds), {"runs": 0, "res": [], "rates": [], "gap": 0})
            b["runs"] += 1
            b["res"] += absr
            b["rates"].append(rate)
            b["gap"] = max(b["gap"], max(gaps))
            continue
        print(f"run {i}: {len(ts)} QF, {len(ts) * step / 1e6:.1f} s, rate {rate:.5f}, "
              f"|res| p50 {p(absr, .5) / 1000:.2f} ms p95 {p(absr, .95) / 1000:.2f} ms max {max(absr) / 1000:.2f} ms, "
              f"max gap {max(gaps) / 1000:.1f} ms")
    for k in sorted(bins):
        b = bins[k]
        print(f"bin {k * a.bin_seconds / 60:6.1f} min: {b['runs']} runs, rate {min(b['rates']):.5f}..{max(b['rates']):.5f}, "
              f"|res| p95 {p(b['res'], .95) / 1000:.2f} ms max {max(b['res']) / 1000:.2f} ms, max gap {b['gap'] / 1000:.1f} ms")
    if skipped and a.bin_seconds:
        print(f"skipped {skipped} runs shorter than {a.min_qf} QF")
    if allres:
        print(f"all: |res| p95 {p(allres, .95) / 1000:.2f} ms, max {max(allres) / 1000:.2f} ms")


if __name__ == "__main__":
    main()
