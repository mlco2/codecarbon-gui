import { afterEach, describe, expect, it } from "vitest";
import {
  afootUpdateRow,
  agentStateLabel,
  count,
  coverage,
  impactLine,
  impactRange,
  notEstimatedLine,
  notEstimatedTotals,
  percent,
  problem,
  screen,
  setup,
  share,
  statusIsDue,
  tokenSplit,
  utcOffsetMinutes,
} from "./agents";
import type { AfootFigures, AfootOutcome, AfootStatus, AfootSummary } from "./api";
import { setLocale } from "./i18n/i18n";

// Hand-written to follow afoot's schema; never copied from a real machine.
const figures = (over: Partial<AfootFigures> = {}): AfootFigures => ({
  calls: 10,
  sessions: 2,
  tokens: { input: 1000, output: 200, thought: 0, cache_read: 8000, cache_write: 800, total: 10000 },
  estimated_calls: 8,
  unestimated_calls: 2,
  unestimated_by_reason: { unknown_model: 2 },
  remote: {
    energy: { unit: "kWh", total: { min: 0.0012, max: 0.0048 } },
    gwp: { unit: "kgCO2eq", total: { min: 0.0004, max: 0.002 } },
  },
  ...over,
});

const status = (over: Partial<AfootStatus> = {}): AfootStatus => ({
  afoot_version: "0.1.0",
  self_update: { managed: false },
  service: { supported: true, installed: true },
  receiver: { endpoint: "http://127.0.0.1:4318/v1/logs", reachable: true, note: null },
  collector: { running: true },
  agents: [
    { id: "claude-code", name: "Claude Code", state: "receiving", detail: null, action: null },
    { id: "codex", name: "Codex", state: "not_installed", detail: null, action: null },
  ],
  ...over,
});

const summary = (over: Partial<AfootSummary> = {}): AfootSummary => ({
  afoot_version: "0.1.0",
  generated_at: "2026-01-02T10:00:00.000Z",
  period: { name: "today", from: "2026-01-02T00:00:00.000Z", to: "2026-01-02T10:00:00.000Z", utc_offset_minutes: 0 },
  collector: { running: true, last_event_at: "2026-01-02T09:59:00.000Z" },
  totals: figures(),
  by_agent: [{ ...figures(), agent: "claude-code", name: "Claude Code" }],
  by_model: [{ ...figures(), agent: "claude-code", model: "model-a" }],
  models_not_estimated: [],
  by_day: [{ ...figures(), date: "2026-01-02" }],
  ...over,
});

const ok = <T>(data: T): AfootOutcome<T> => ({ outcome: "ok", data });

afterEach(() => setLocale("en"));

describe("token counts", () => {
  it("keeps three significant digits and changes unit before 1000", () => {
    expect(count(950)).toBe("950");
    expect(count(12_300)).toBe("12.3 k");
    expect(count(4_560_000)).toBe("4.56 M");
    expect(count(999_950)).toBe("1 M");
    expect(count(-1)).toBe("–");
  });

  it("splits a total by kind, so cache reads are seen for what they are", () => {
    const split = tokenSplit(figures().tokens);
    expect(split.map((p) => p.kind)).toEqual(["input", "output", "cache_read", "cache_write", "thought"]);
    expect(split.find((p) => p.kind === "cache_read")?.share).toBeCloseTo(0.8);
    expect(split.reduce((sum, p) => sum + p.share, 0)).toBeCloseTo(1);
    expect(tokenSplit({ input: 0, output: 0, thought: 0, cache_read: 0, cache_write: 0, total: 0 })[0]?.share).toBe(0);
  });

  it("never rounds a part up to the whole or down to nothing", () => {
    expect(percent(0)).toBe("0");
    expect(percent(0.004)).toBe("<1");
    expect(percent(0.999)).toBe("99");
    expect(percent(1)).toBe("100");
    expect(share(5, 0)).toBe(0);
    expect(share(5, 10)).toBe(0.5);
  });
});

describe("estimated impacts", () => {
  it("are ranges, in a unit that fits both ends", () => {
    expect(impactRange(figures().remote, "energy")).toBe("1.2–4.8 Wh");
    expect(impactRange(figures().remote, "gwp")).toBe("0.4–2 g");
    const big = { energy: { unit: "kWh", total: { min: 1.1, max: 3.2 } } };
    expect(impactRange(big, "energy")).toBe("1.1–3.2 kWh");
  });

  it("are absent, not zero, when nothing was estimated", () => {
    expect(impactRange({}, "energy")).toBeNull();
    expect(impactLine(figures({ estimated_calls: 0, remote: {} }))).toBe("no estimate");
  });

  it("always say how many calls they cover", () => {
    expect(coverage(figures())).toBe("8 of 10 calls estimated");
    expect(coverage(figures({ calls: 1, estimated_calls: 1 }))).toBe("1 of 1 call estimated");
    expect(impactLine(figures())).toBe("1.2–4.8 Wh · 0.4–2 g CO₂e");
  });

  it("name the models left out, with what they amount to", () => {
    const models = [
      { agent: "claude-code", model: "model-b", reason: "unknown_model", calls: 2, tokens: 200 },
      { agent: "codex", model: "model-c", reason: "unknown_model", calls: 1, tokens: 1500 },
    ];
    expect(notEstimatedTotals(models)).toEqual({ calls: 3, tokens: 1700 });
    expect(notEstimatedLine(models[0]!)).toBe("model-b: 2 calls, 200 tokens");
  });
});

describe("what the view shows", () => {
  it("waits for both answers", () => {
    expect(screen(null, null).kind).toBe("loading");
    expect(screen(ok(status()), null).kind).toBe("loading");
  });

  it("explains an afoot that is missing, too old or broken, each differently", () => {
    expect(screen({ outcome: "not_found" }, ok(summary()))).toEqual({
      kind: "problem",
      problem: { kind: "not_found" },
    });
    const unsupported = problem({ outcome: "unsupported_protocol", found: 2, supported: 1 });
    expect(unsupported?.kind).toBe("unsupported");
    expect(problem({ outcome: "timed_out", seconds: 10 })).toEqual({
      kind: "failed",
      text: "afoot did not answer within 10 s.",
    });
    expect(problem({ outcome: "failed", exit_code: 3, detail: "" })).toEqual({
      kind: "failed",
      text: "afoot stopped with code 3: it gave no reason",
    });
    expect(problem(ok(1))).toBeNull();
  });

  it("is ready with data, and says when the period is empty", () => {
    const ready = screen(ok(status()), ok(summary()));
    expect(ready.kind).toBe("ready");
    const empty = screen(ok(status()), ok(summary({ totals: figures({ calls: 0 }) })));
    expect(empty.kind === "ready" && empty.empty).toBe(true);
  });
});

describe("what keeps usage from being recorded", () => {
  it("is nothing when the collector answers and agents are receiving", () => {
    const s = setup(status());
    expect(s.collector).toBe("ok");
    expect(s.needsAttention).toBe(false);
    expect(s.agents.map((a) => a.id)).toEqual(["claude-code"]);
  });

  it("offers the service only where afoot can install one", () => {
    const off = { endpoint: "x", reachable: false, note: "connection refused" };
    const missing = setup(status({ receiver: off, service: { supported: true, installed: false } }));
    expect([missing.collector, missing.offerService]).toEqual(["service_missing", true]);
    expect(missing.receiverNote).toBe("connection refused");
    const stopped = setup(status({ receiver: off }));
    expect([stopped.collector, stopped.offerService]).toEqual(["stopped", false]);
    const manual = setup(status({ receiver: off, service: { supported: false, installed: false } }));
    expect([manual.collector, manual.offerService]).toEqual(["manual", false]);
  });

  it("offers to configure agents that are not, and flags one to restart", () => {
    const agent = (state: string) => ({ id: "codex", name: "Codex", state, detail: null, action: null });
    expect(setup(status({ agents: [agent("not_configured")] })).offerSetup).toBe(true);
    expect(setup(status({ agents: [agent("misconfigured")] })).offerSetup).toBe(true);
    const restart = setup(status({ agents: [agent("needs_restart")] }));
    expect([restart.offerSetup, restart.needsAttention]).toEqual([false, true]);
    expect(agentStateLabel("a-state-added-later")).toBe("unknown");
  });
});

describe("the runtime the estimates need", () => {
  const runtime = (provisioner: string | null) => ({
    ready: false,
    problem: "venv directory missing",
    fix: "run `afoot python setup`",
    provisioner,
    uv_install_url: "https://docs.astral.sh/uv/getting-started/installation/",
  });

  it("is not mentioned when it is ready, or when afoot does not report it", () => {
    expect(setup(status()).estimates).toBe("ok");
    expect(setup(status({ python: null })).estimates).toBe("ok");
  });

  it("is offered when afoot can build it, and uv is recommended when nothing can", () => {
    const can = setup(status({ python: runtime("uv") }));
    expect([can.estimates, can.needsAttention]).toEqual(["setup", true]);
    const cannot = setup(status({ python: runtime(null) }));
    expect(cannot.estimates).toBe("need_uv");
    expect(cannot.uvUrl).toBe("https://docs.astral.sh/uv/getting-started/installation/");
  });

  it("never opens a link that is not https", () => {
    const odd = setup(status({ python: { ...runtime(null), uv_install_url: "file:///etc/passwd" } }));
    expect(odd.uvUrl).toBe("https://docs.astral.sh/uv/getting-started/installation/");
  });
});

describe("when afoot status is read again", () => {
  it("keeps a good answer between refreshes, and retries a failed one", () => {
    expect(statusIsDue(ok(status()), false)).toBe(false);
    expect(statusIsDue(ok(status()), true)).toBe(true);
    expect(statusIsDue(null, false)).toBe(true);
    expect(statusIsDue({ outcome: "timed_out", seconds: 10 }, false)).toBe(true);
    expect(statusIsDue({ outcome: "not_found" }, false)).toBe(true);
  });
});

describe("Settings → Updates: afoot", () => {
  it("enables the update only for an afoot that updates itself", () => {
    expect(afootUpdateRow(ok(status())).canUpdate).toBe(false);
    const managed = afootUpdateRow(ok(status({ self_update: { managed: true } })));
    expect([managed.line, managed.showButton, managed.canUpdate]).toEqual(["afoot 0.1.0", true, true]);
    const missing = afootUpdateRow({ outcome: "not_found" });
    expect([missing.showButton, missing.visible]).toEqual([false, false]);
    expect(managed.visible).toBe(true);
    // Any other failure is shown: there is an afoot, and it did not answer.
    const hung = afootUpdateRow({ outcome: "timed_out", seconds: 10 });
    expect([hung.visible, hung.line]).toEqual([true, "afoot did not answer within 10 s."]);
    const busy = afootUpdateRow({ outcome: "busy" });
    expect(busy.line).toBe("afoot is busy with another action. Try again when it is done.");
  });
});

describe("in French", () => {
  it("words counts, coverage and problems", () => {
    setLocale("fr");
    expect(coverage(figures())).toBe("8 appels estimés sur 10");
    expect(count(1_200_000_000)).toBe("1,2 Md");
    expect(problem({ outcome: "not_found" })).toEqual({ kind: "not_found" });
    expect(agentStateLabel("not_configured")).toBe("non configuré");
  });
});

describe("the UTC offset sent to afoot", () => {
  it("is minutes east of UTC", () => {
    const at = (offset: number) => ({ getTimezoneOffset: () => offset }) as Date;
    expect(utcOffsetMinutes(at(-60))).toBe(60);
    expect(utcOffsetMinutes(at(300))).toBe(-300);
    expect(utcOffsetMinutes(at(0))).toBe(0);
  });
});
