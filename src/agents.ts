/**
 * Everything the Agents view decides and words, from what `af` answered
 * (`af status`, `af summary`). Pure, so it is unit-tested; the view
 * (views/agents.ts) only puts the result in the DOM.
 *
 * Two kinds of numbers, never mixed:
 * * tokens are measured (reported by the agents);
 * * impacts are modelled for the remote inference, given as a range over
 *   the estimated calls only. They are never extrapolated to the other
 *   calls, never shown as one mid-point, and never added to what
 *   CodeCarbon measures on this computer.
 */
import type {
  AfAgentStatus,
  AfFigures,
  AfImpact,
  AfNotEstimated,
  AfOutcome,
  AfStatus,
  AfSummary,
  AfTokens,
} from "./api";
import { round3, sig3, withUnit } from "./format";
import { errorText, formatNumber, intlLocale, n, t, type MessageKey } from "./i18n/i18n";

const DASH = "–";

/** Where `af` and its install instructions live. */
export const AF_URL = "https://github.com/mlco2/agentic-footprint";

// --- Tokens ----------------------------------------------------------------------

const COUNT_UNITS: readonly [number, MessageKey][] = [
  [1e3, "agents.count_k"],
  [1e6, "agents.count_m"],
  [1e9, "agents.count_b"],
];

/** A count in at most 3 significant digits: "950", "12.3 k", "4.56 M", "1.2 B". */
export function count(x: number): string {
  if (!Number.isFinite(x) || x < 0) return DASH;
  if (x < 1000) return formatNumber(x, 0);
  let unit = x >= 1e9 ? 2 : x >= 1e6 ? 1 : 0;
  // 999 950 is "1 M", not "1000 k".
  if (unit < 2 && round3(x / (COUNT_UNITS[unit]?.[0] ?? 1)) >= 1000) unit += 1;
  const [size, key] = COUNT_UNITS[unit] ?? [1, "agents.count_k"];
  return t(key, { value: sig3(x / size) });
}

/** The exact count, for tooltips ("1234567"; no grouping, like the rest of the app). */
export function exact(x: number): string {
  return Number.isFinite(x) ? formatNumber(x, 0) : DASH;
}

/** The kinds of tokens, in the order they are shown (and coloured). */
export const TOKEN_KINDS = ["input", "output", "cache_read", "cache_write", "thought"] as const;
export type TokenKind = (typeof TOKEN_KINDS)[number];

export interface TokenPart {
  kind: TokenKind;
  value: number;
  /** 0–1, of the sum of the five kinds. */
  share: number;
}

/** The split of a token total; cache reads usually dominate. */
export function tokenSplit(tokens: AfTokens): TokenPart[] {
  const value = (kind: TokenKind): number => Math.max(0, tokens[kind] || 0);
  const sum = TOKEN_KINDS.reduce((acc, kind) => acc + value(kind), 0);
  return TOKEN_KINDS.map((kind) => ({
    kind,
    value: value(kind),
    share: sum > 0 ? value(kind) / sum : 0,
  }));
}

/** `part` of `whole` as 0–1 (0 when there is no whole). */
export function share(part: number, whole: number): number {
  return whole > 0 && part > 0 ? Math.min(part / whole, 1) : 0;
}

/** A share as text: "0", "<1", "42", "100" (the caller adds "%"). */
export function percent(fraction: number): string {
  if (!(fraction > 0)) return "0";
  if (fraction < 0.01) return "<1";
  // Never "100" for a part that is not the whole.
  return String(fraction < 1 ? Math.min(Math.round(fraction * 100), 99) : 100);
}

// --- Estimated impacts -----------------------------------------------------------

/** The two criteria shown, and the unit `af` gives them in. */
export type Criterion = "energy" | "gwp";
const BASE_UNIT: Record<Criterion, string> = { energy: "kWh", gwp: "kgCO2eq" };
/** Below 1 base unit, shown in the smaller one (×1000). */
const SMALL_UNIT: Record<Criterion, string> = { energy: "Wh", gwp: "g" };
const LARGE_UNIT: Record<Criterion, string> = { energy: "kWh", gwp: "kg" };

function rangeText(low: number, high: number, unit: string): string {
  const [a, b] = [sig3(low), sig3(high)];
  return withUnit(a === b ? a : `${a}–${b}`, unit, intlLocale());
}

/**
 * An impact as a range, both ends in the same unit: "1.2–4.8 Wh",
 * "0.4–2 g" (CO₂e), "1.1–3.2 kWh". Null when `af` gave no figure (no
 * estimated call). A unit we do not expect is shown as it came.
 */
export function impactRange(remote: Record<string, AfImpact>, criterion: Criterion): string | null {
  const impact = remote[criterion];
  if (!impact) return null;
  const { min, max } = impact.total;
  if (!Number.isFinite(min) || !Number.isFinite(max)) return null;
  const [low, high] = [Math.min(min, max), Math.max(min, max)];
  if (impact.unit !== BASE_UNIT[criterion]) return rangeText(low, high, impact.unit);
  return high < 1
    ? rangeText(low * 1000, high * 1000, SMALL_UNIT[criterion])
    : rangeText(low, high, LARGE_UNIT[criterion]);
}

/** "8 of 10 calls estimated": goes with every impact figure. */
export function coverage(figures: AfFigures): string {
  return n("agents.coverage", figures.calls, { estimated: figures.estimated_calls });
}

/** "1.2–4.8 Wh · 0.4–2 g CO₂e", or "no estimate" when there is none. */
export function impactLine(figures: AfFigures): string {
  const energy = impactRange(figures.remote, "energy");
  const gwp = impactRange(figures.remote, "gwp");
  if (figures.estimated_calls === 0 || (!energy && !gwp)) return t("agents.no_estimate");
  return [energy, gwp ? t("agents.co2_value", { value: gwp }) : null].filter(Boolean).join(" · ");
}

/** The calls and tokens of the models the estimator does not know yet. */
export function notEstimatedTotals(models: AfNotEstimated[]): { calls: number; tokens: number } {
  return models.reduce(
    (acc, m) => ({ calls: acc.calls + m.calls, tokens: acc.tokens + m.tokens }),
    { calls: 0, tokens: 0 },
  );
}

/** "model-b: 2 calls, 200 tokens". */
export function notEstimatedLine(model: AfNotEstimated): string {
  return t("agents.not_estimated_row", {
    model: model.model,
    calls: n("agents.calls", model.calls),
    tokens: n("agents.tokens_count", model.tokens, { value: count(model.tokens) }),
  });
}

// --- When `af` did not answer ------------------------------------------------------

export type Problem =
  /** Not installed: explained, with a link to the install instructions. */
  | { kind: "not_found" }
  /** Another protocol version: this `af` is not for this version of the app. */
  | { kind: "unsupported"; text: string }
  /** Anything else, with the reason. */
  | { kind: "failed"; text: string };

/** Why there is no answer; null for an answer. */
export function problem<T>(outcome: AfOutcome<T>): Problem | null {
  switch (outcome.outcome) {
    case "ok":
      return null;
    case "not_found":
      return { kind: "not_found" };
    case "unsupported_protocol":
      return {
        kind: "unsupported",
        text:
          outcome.found === null
            ? t("agents.unsupported_unknown", { supported: outcome.supported })
            : t("agents.unsupported", { found: outcome.found, supported: outcome.supported }),
      };
    case "spawn_failed":
      return { kind: "failed", text: t("agents.problem.spawn_failed", { detail: outcome.detail }) };
    case "timed_out":
      return { kind: "failed", text: t("agents.problem.timed_out", { seconds: outcome.seconds }) };
    case "invalid_output":
      return {
        kind: "failed",
        text: t("agents.problem.invalid_output", { detail: outcome.detail }),
      };
    case "failed": {
      const detail = outcome.detail || t("agents.problem.no_detail");
      return {
        kind: "failed",
        text:
          outcome.exit_code === null
            ? t("agents.problem.interrupted", { detail })
            : t("agents.problem.failed", { code: outcome.exit_code, detail }),
      };
    }
  }
}

/** One line for any problem (action results, Settings). */
export function problemText(p: Problem): string {
  return p.kind === "not_found" ? t("agents.not_found_title") : p.text;
}

/** A command that rejected (it should not) reads like an `af` that did not start. */
export function settled<T>(call: Promise<AfOutcome<T>>): Promise<AfOutcome<T>> {
  return call.catch((e: unknown) => ({ outcome: "spawn_failed", detail: errorText(e) }));
}

// --- Setup: is anything recording? -------------------------------------------------

export type CollectorState =
  | "ok"
  /** No service, and `af` can install one here: offered. */
  | "service_missing"
  /** A service is installed but does not answer. */
  | "stopped"
  /** No service manager `af` can use: the user starts `af watch`. */
  | "manual";

export interface Setup {
  collector: CollectorState;
  /** Why the receiver does not answer, in `af`'s words. */
  receiverNote: string | null;
  /** Offer "Install the service" (`af service install`). */
  offerService: boolean;
  /** The agents found on this computer, with their state. */
  agents: AfAgentStatus[];
  /** Offer "Configure agents" (`af setup`). */
  offerSetup: boolean;
  /** Something keeps usage from being recorded: show the card. */
  needsAttention: boolean;
}

const STATES = [
  "receiving",
  "needs_restart",
  "waiting",
  "misconfigured",
  "not_configured",
  "not_installed",
  "unknown",
] as const;
type AgentState = (typeof STATES)[number];

function agentState(state: string): AgentState {
  return (STATES as readonly string[]).includes(state) ? (state as AgentState) : "unknown";
}

/** "receiving", "restart needed", … ("unknown" for a state added since). */
export function agentStateLabel(state: string): string {
  return t(`agents.state.${agentState(state)}`);
}

/** What stands between the agents and a recorded call, from `af status`. */
export function setup(status: AfStatus): Setup {
  const collecting = status.receiver.reachable && status.collector.running !== false;
  const collector: CollectorState = collecting
    ? "ok"
    : status.service.installed
      ? "stopped"
      : status.service.supported
        ? "service_missing"
        : "manual";
  const agents = status.agents.filter((a) => agentState(a.state) !== "not_installed");
  const states = agents.map((a) => agentState(a.state));
  const offerSetup = states.some((s) => s === "not_configured" || s === "misconfigured");
  return {
    collector,
    receiverNote: collecting ? null : status.receiver.note,
    offerService: collector === "service_missing",
    agents,
    offerSetup,
    needsAttention: collector !== "ok" || offerSetup || states.includes("needs_restart"),
  };
}

// --- The whole view ----------------------------------------------------------------

export type Screen =
  | { kind: "loading" }
  | { kind: "problem"; problem: Problem }
  | {
      kind: "ready";
      setup: Setup;
      summary: AfSummary;
      /** No call in the period: nothing to chart yet. */
      empty: boolean;
    };

/** What the Agents view shows, from the last answers (null: not asked yet). */
export function screen(
  status: AfOutcome<AfStatus> | null,
  summary: AfOutcome<AfSummary> | null,
): Screen {
  if (!status || !summary) return { kind: "loading" };
  const failed = problem(status) ?? problem(summary);
  if (failed) return { kind: "problem", problem: failed };
  if (status.outcome !== "ok" || summary.outcome !== "ok") return { kind: "loading" };
  return {
    kind: "ready",
    setup: setup(status.data),
    summary: summary.data,
    empty: summary.data.totals.calls === 0,
  };
}

/** Minutes east of UTC, as `af summary --utc-offset-minutes` wants them. */
export function utcOffsetMinutes(now: Date = new Date()): number {
  // `getTimezoneOffset` counts the other way (minutes *behind* UTC).
  return -now.getTimezoneOffset() || 0;
}

// --- Settings → Updates: the `af` row ---------------------------------------------

export interface AfUpdateRow {
  /** "af 0.1.0", "af is not installed", or why it could not be read. */
  line: string;
  /** Show the "Update af" button at all (af answered). */
  showButton: boolean;
  /** Enabled only when `af self update` manages this install. */
  canUpdate: boolean;
  /** Under the line: what the button does, or why it is disabled. */
  note: string | null;
}

export function afUpdateRow(status: AfOutcome<AfStatus>): AfUpdateRow {
  const failed = problem(status);
  if (failed || status.outcome !== "ok") {
    const line = failed ? problemText(failed) : "";
    return { line, showButton: false, canUpdate: false, note: null };
  }
  const managed = status.data.self_update.managed;
  return {
    line: t("settings.af_version", { version: status.data.af_version }),
    showButton: true,
    canUpdate: managed,
    note: managed ? t("settings.af_managed") : t("settings.af_unmanaged"),
  };
}
