/**
 * Agents: the tokens your coding agents used and an estimate of the impact
 * of the remote inference, read from the `af` CLI by Rust (`af_status`,
 * `af_summary`; af.rs). What to show and how to word it is decided in
 * agents.ts; this file builds the DOM.
 *
 * `af` runs only while this view is on screen: once when it opens, then
 * the summary every 30 s. The three buttons that change the machine
 * (install the service, configure the agents, update af) run on a click.
 */
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  afInstallService,
  afSetupAgents,
  afSetupPython,
  afStatus,
  afSummary,
  errorMessage,
  openUrl,
  type AfActionOutput,
  type AfFigures,
  type AfOutcome,
  type AfPeriod,
  type AfStatus,
  type AfSummary,
} from "../api";
import {
  AF_URL,
  agentStateLabel,
  count,
  coverage,
  exact,
  impactLine,
  impactRange,
  notEstimatedLine,
  notEstimatedTotals,
  percent,
  problem,
  problemText,
  screen,
  settled,
  share,
  tokenSplit,
  utcOffsetMinutes,
  type Problem,
  type Setup,
} from "../agents";
import { barChart } from "../barchart";
import { h, setText, show, spinner, svg } from "../dom";
import * as fmt from "../format";
import { live, liveAttr, n, t, type MessageKey } from "../i18n/i18n";

/** The summary is read again this often while the view is on screen. */
const REFRESH_MS = 30_000;

const PERIODS: [AfPeriod, MessageKey][] = [
  ["today", "agents.period.today"],
  ["7d", "agents.period.7d"],
  ["30d", "agents.period.30d"],
];

const CHART_W = 300;
const BARS_H = 72;
const TICKS_H = 16;

type ActionName = "service" | "setup" | "python";
const ACTIONS: Record<ActionName, () => Promise<AfOutcome<AfActionOutput>>> = {
  service: afInstallService,
  setup: afSetupAgents,
  python: afSetupPython,
};

/** The panel window is on screen (it is hidden, not closed, when dismissed). */
async function panelVisible(): Promise<boolean> {
  if (document.visibilityState === "hidden") return false;
  try {
    return await getCurrentWindow().isVisible();
  } catch {
    return true;
  }
}

/** A row of a "By agent" / "By model" list. */
function figuresRow(name: string, sub: string | null, f: AfFigures, total: number): HTMLElement {
  const part = share(f.tokens.total, total);
  const fill = h("span", { class: "share-fill" });
  fill.style.width = `${Math.max(part * 100, part > 0 ? 1 : 0)}%`;
  return h(
    "li",
    {},
    h(
      "div",
      { class: "agent-row-head" },
      h(
        "span",
        { class: "agent-name" },
        name,
        sub ? h("span", { class: "muted small" }, ` ${sub}`) : null,
      ),
      h(
        "span",
        { class: "agent-tokens", title: exact(f.tokens.total) },
        n("agents.tokens_count", f.tokens.total, { value: count(f.tokens.total) }),
      ),
    ),
    h(
      "div",
      { class: "share", title: t("agents.share", { percent: percent(part) }) },
      fill,
    ),
    h("div", { class: "muted small" }, `${impactLine(f)} · ${coverage(f)}`),
  );
}

export class AgentsView {
  readonly root: HTMLElement;

  private period: AfPeriod = "today";
  private status: AfOutcome<AfStatus> | null = null;
  private summary: AfOutcome<AfSummary> | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  /** Ignore the answer of a load that a newer one overtook. */
  private loadSeq = 0;
  /** The action in progress, and the result of the last one. */
  private running: ActionName | null = null;
  private lastAction: AfOutcome<AfActionOutput> | null = null;

  private readonly periodButtons = new Map<AfPeriod, HTMLButtonElement>();
  private readonly body = h("div", { class: "view" });

  constructor() {
    const periods = liveAttr(
      h("div", { class: "segmented", role: "group" }),
      "aria-label",
      "agents.period_label",
    );
    for (const [period, label] of PERIODS) {
      const button = h("button", { type: "button" }, live(label));
      button.addEventListener("click", () => this.setPeriod(period));
      this.periodButtons.set(period, button);
      periods.append(button);
    }
    this.root = h("section", { class: "view agents" }, periods, this.body);
    this.render();
  }

  /** The Agents tab was opened (true) or left (false). */
  setActive(active: boolean): void {
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
    if (!active) return;
    this.lastAction = null;
    void this.load(true);
    this.timer = setInterval(() => {
      void panelVisible().then((visible) => {
        if (visible && this.timer !== null) void this.load(false);
      });
    }, REFRESH_MS);
  }

  /** The language changed: render the last answers again. */
  relocalize(): void {
    this.render();
  }

  private setPeriod(period: AfPeriod): void {
    if (period === this.period) return;
    this.period = period;
    // The previous period stays on screen until the new one answers.
    this.render();
    void this.load(false);
  }

  /** Read the summary and, on opening or after an action, the status. */
  private async load(withStatus: boolean): Promise<void> {
    const seq = ++this.loadSeq;
    const [status, summary] = await Promise.all([
      withStatus || !this.status ? settled(afStatus()) : this.status,
      settled(afSummary(this.period, utcOffsetMinutes())),
    ]);
    if (seq !== this.loadSeq) return;
    this.status = status;
    this.summary = summary;
    this.render();
  }

  /** A button was clicked: run `af`, show what it said, read everything again. */
  private async act(action: ActionName): Promise<void> {
    if (this.running) return;
    this.running = action;
    this.lastAction = null;
    this.render();
    this.lastAction = await settled(ACTIONS[action]());
    this.running = null;
    this.render();
    await this.load(true);
  }

  private render(): void {
    for (const [period, button] of this.periodButtons) {
      button.setAttribute("aria-pressed", String(period === this.period));
    }
    const state = screen(this.status, this.summary);
    switch (state.kind) {
      case "loading":
        this.body.replaceChildren(
          h("p", { class: "hint" }, spinner(), t("agents.loading")),
        );
        return;
      case "problem":
        this.body.replaceChildren(this.problemCard(state.problem));
        return;
      case "ready": {
        const { summary } = state;
        this.body.replaceChildren(
          ...[
            state.setup.needsAttention || this.running || this.lastAction
              ? this.setupCard(state.setup)
              : null,
            state.empty
              ? h("div", { class: "card empty-state" }, h("p", {}, t("agents.empty")))
              : null,
            ...(state.empty ? [] : this.dataCards(summary)),
          ].filter((node): node is HTMLElement => node !== null),
        );
      }
    }
  }

  // --- `af` did not answer -------------------------------------------------------

  private problemCard(p: Problem): HTMLElement {
    const again = h("button", { class: "btn btn-small", type: "button" }, t("agents.check_again"));
    again.addEventListener("click", () => {
      this.status = null;
      this.render();
      void this.load(true);
    });
    const message = h("p", { class: "form-message error", role: "status" });
    const link = h("a", { href: AF_URL, class: "link" }, t("agents.install_link"));
    link.addEventListener("click", (e) => {
      e.preventDefault();
      openUrl(AF_URL).catch((err: unknown) => setText(message, errorMessage(err)));
    });
    const title =
      p.kind === "not_found"
        ? t("agents.not_found_title")
        : p.kind === "unsupported"
          ? t("agents.unsupported_title")
          : t("agents.failed_title");
    return h(
      "section",
      { class: "card agents-problem" },
      h("div", { class: "card-head" }, h("span", {}, title)),
      p.kind === "not_found"
        ? h("p", {}, t("agents.not_found_body"))
        : h("p", { class: p.kind === "failed" ? "af-reason" : "" }, p.text),
      p.kind === "not_found" ? h("p", { class: "muted small" }, t("agents.not_found_how")) : null,
      h("div", { class: "agents-actions" }, p.kind === "failed" ? null : link, again),
      message,
    );
  }

  // --- Something is not recording yet --------------------------------------------

  private actionButton(action: ActionName, label: MessageKey, hint: MessageKey): HTMLElement {
    const button = h(
      "button",
      { class: "btn btn-small", type: "button", disabled: this.running !== null },
      this.running === action ? spinner() : null,
      t(label),
    );
    button.addEventListener("click", () => void this.act(action));
    return h("div", { class: "agents-action" }, button, h("p", { class: "muted small" }, t(hint)));
  }

  private setupCard(setup: Setup): HTMLElement {
    const collector =
      setup.collector === "ok"
        ? null
        : h(
            "p",
            {},
            t(`agents.collector.${setup.collector}`),
            setup.receiverNote ? h("span", { class: "muted small" }, ` ${setup.receiverNote}`) : null,
          );
    const agents = setup.agents.map((agent) =>
      h(
        "li",
        {},
        h(
          "div",
          { class: "agent-row-head" },
          h("span", { class: "agent-name" }, agent.name),
          h("span", { class: "muted" }, agentStateLabel(agent.state)),
        ),
        agent.detail ? h("div", { class: "muted small" }, agent.detail) : null,
        agent.action ? h("div", { class: "muted small" }, agent.action) : null,
      ),
    );
    return h(
      "section",
      { class: "card agents-setup" },
      h("div", { class: "card-head" }, h("span", {}, t("agents.setup_title"))),
      collector,
      setup.offerService
        ? this.actionButton("service", "agents.install_service", "agents.install_service_hint")
        : null,
      agents.length > 0 ? h("ul", { class: "agent-rows" }, ...agents) : null,
      setup.offerSetup
        ? this.actionButton("setup", "agents.setup_agents", "agents.setup_agents_hint")
        : null,
      setup.estimates === "setup"
        ? h(
            "div",
            {},
            h("p", {}, t("agents.estimates_missing")),
            this.actionButton("python", "agents.setup_python", "agents.setup_python_hint"),
          )
        : null,
      setup.estimates === "need_uv" ? this.needUv(setup.uvUrl) : null,
      this.actionResult(),
    );
  }

  /** No uv and no Python: where to get uv, which brings Python with it. */
  private needUv(url: string): HTMLElement {
    const message = h("p", { class: "form-message error", role: "status" });
    const link = h("a", { href: url, class: "link" }, t("agents.install_uv"));
    link.addEventListener("click", (e) => {
      e.preventDefault();
      openUrl(url).catch((err: unknown) => setText(message, errorMessage(err)));
    });
    const again = h("button", { class: "btn btn-small", type: "button" }, t("agents.check_again"));
    again.addEventListener("click", () => void this.load(true));
    return h(
      "div",
      {},
      h("p", {}, t("agents.estimates_missing")),
      h("p", { class: "muted small" }, t("agents.need_uv")),
      h("div", { class: "agents-actions" }, link, again),
      message,
    );
  }

  /** "Done." or why not, and what `af` printed. */
  private actionResult(): HTMLElement | null {
    if (this.running) {
      return h("p", { class: "muted small", role: "status" }, t("agents.action_running"));
    }
    const last = this.lastAction;
    if (!last) return null;
    const failed = problem(last);
    const output = last.outcome === "ok" ? last.data.output : "";
    return h(
      "div",
      { role: "status" },
      h(
        "p",
        { class: failed ? "form-message error" : "form-message" },
        failed ? problemText(failed) : t("agents.action_done"),
      ),
      output
        ? h(
            "details",
            { class: "small" },
            h("summary", {}, t("agents.action_output")),
            h("pre", { class: "af-output" }, output),
          )
        : null,
    );
  }

  // --- The numbers ---------------------------------------------------------------

  private dataCards(summary: AfSummary): HTMLElement[] {
    const { totals } = summary;
    const split = tokenSplit(totals.tokens);

    const bar = h("div", {
      class: "token-bar",
      role: "img",
      "aria-label": t("agents.token_bar_label"),
    });
    const legend = h("ul", { class: "token-legend" });
    for (const part of split) {
      const label = t(`agents.token.${part.kind}`);
      const pct = t("agents.percent", { percent: percent(part.share) });
      if (part.value > 0) {
        const segment = h("span", {
          class: `token-seg token-${part.kind}`,
          title: `${label}: ${exact(part.value)} (${pct})`,
        });
        segment.style.flexGrow = String(part.value);
        bar.append(segment);
      }
      legend.append(
        h(
          "li",
          { title: exact(part.value) },
          h("span", { class: `token-key token-${part.kind}`, "aria-hidden": "true" }),
          h("span", { class: "token-label" }, label),
          h("span", { class: "token-value" }, count(part.value)),
          h("span", { class: "token-share muted" }, pct),
        ),
      );
    }
    const tokens = h(
      "section",
      { class: "card" },
      h(
        "div",
        { class: "card-head" },
        h("span", {}, t("agents.tokens")),
        h("span", { class: "muted" }, t("agents.tokens_measured")),
      ),
      h("div", { class: "tile-value", title: exact(totals.tokens.total) }, count(totals.tokens.total)),
      h(
        "div",
        { class: "tile-sub" },
        `${n("agents.calls", totals.calls)} · ${n("agents.sessions", totals.sessions)}`,
      ),
      bar,
      legend,
    );

    const impactTile = (label: MessageKey, value: string | null): HTMLElement =>
      h(
        "div",
        { class: "impact" },
        h("div", { class: "tile-label" }, t(label)),
        h("div", { class: "impact-value" }, value ?? "–"),
      );
    const impacts = h(
      "section",
      { class: "card" },
      h(
        "div",
        { class: "card-head" },
        h("span", {}, t("agents.impact")),
        h("span", { class: "muted" }, t("agents.impact_scope")),
      ),
      h(
        "div",
        { class: "impacts" },
        impactTile("dashboard.energy", impactRange(totals.remote, "energy")),
        impactTile("dashboard.co2", impactRange(totals.remote, "gwp")),
      ),
      h("p", { class: "impact-coverage" }, coverage(totals)),
      h("p", { class: "muted small" }, t("agents.impact_note")),
    );

    const cards = [tokens, impacts];

    if (summary.models_not_estimated.length > 0) {
      const missing = notEstimatedTotals(summary.models_not_estimated);
      cards.push(
        h(
          "section",
          { class: "card agents-notice" },
          h("div", { class: "card-head" }, h("span", {}, t("agents.not_estimated_title"))),
          h(
            "p",
            {},
            t("agents.not_estimated_body", {
              calls: n("agents.calls", missing.calls),
              tokens: n("agents.tokens_count", missing.tokens, { value: count(missing.tokens) }),
            }),
          ),
          h(
            "ul",
            { class: "muted small plain-list" },
            ...summary.models_not_estimated.map((m) => h("li", {}, notEstimatedLine(m))),
          ),
        ),
      );
    }

    const list = (title: MessageKey, rows: HTMLElement[]): HTMLElement =>
      h(
        "section",
        { class: "card" },
        h("div", { class: "card-head" }, h("span", {}, t(title))),
        h("ul", { class: "agent-rows" }, ...rows),
      );
    if (summary.by_agent.length > 0) {
      cards.push(
        list(
          "agents.by_agent",
          summary.by_agent.map((a) => figuresRow(a.name, null, a, totals.tokens.total)),
        ),
      );
    }
    if (summary.by_model.length > 0) {
      const agentName = new Map(summary.by_agent.map((a) => [a.agent, a.name]));
      cards.push(
        list(
          "agents.by_model",
          summary.by_model.map((m) =>
            figuresRow(
              m.model,
              summary.by_agent.length > 1 ? (agentName.get(m.agent) ?? m.agent) : null,
              m,
              totals.tokens.total,
            ),
          ),
        ),
      );
    }
    // One day ("Today") is the total above, not a chart.
    if (summary.by_day.length > 1) cards.push(this.dayChart(summary));
    return cards;
  }

  private dayChart(summary: AfSummary): HTMLElement {
    const days = summary.by_day;
    const { bars, max } = barChart(
      days.map((d) => d.tokens.total),
      CHART_W,
      BARS_H,
    );
    const chart = svg("svg", {
      class: "bars",
      viewBox: `0 0 ${CHART_W} ${BARS_H + TICKS_H}`,
      role: "img",
      "aria-label": t("agents.chart_label"),
    });
    const nodes: SVGElement[] = [
      svg("line", { class: "bars-axis", x1: 0, x2: CHART_W, y1: BARS_H + 0.5, y2: BARS_H + 0.5 }),
    ];
    const last = days.length - 1;
    // A week: every day is named; a month: every Monday, like History.
    const everyDay = days.length <= 7;
    days.forEach((day, i) => {
      const bar = bars[i];
      if (!bar) return;
      const g = svg("g", { class: i === last ? "bar today" : "bar" });
      const title = svg("title");
      const dayText = fmt.formatDay(day.date);
      title.textContent =
        t("agents.day_title", {
          day: i === last ? t("history.day_today", { day: dayText }) : dayText,
          tokens: n("agents.tokens_count", day.tokens.total, { value: count(day.tokens.total) }),
        }) + (day.calls > 0 ? ` · ${n("agents.calls", day.calls)}` : "");
      g.append(
        title,
        svg("rect", { class: "bar-hit", x: bar.x - 1, y: 0, width: bar.width + 2, height: BARS_H }),
        svg("rect", {
          class: "bar-fill",
          x: bar.x,
          y: bar.y,
          width: bar.width,
          height: bar.height,
          rx: 1,
        }),
      );
      nodes.push(g);
      if (everyDay || fmt.parseLocal(day.date)?.getDay() === 1) {
        const cx = bar.x + bar.width / 2;
        const anchor = cx < 20 ? "start" : cx > CHART_W - 20 ? "end" : "middle";
        nodes.push(svg("line", { class: "bars-tick", x1: cx, x2: cx, y1: BARS_H, y2: BARS_H + 3 }));
        const label = svg("text", {
          class: "bars-label",
          x: anchor === "start" ? bar.x : anchor === "end" ? bar.x + bar.width : cx,
          y: BARS_H + TICKS_H - 3,
          "text-anchor": anchor,
        });
        label.textContent = fmt.formatTick(day.date);
        nodes.push(label);
      }
    });
    chart.append(...nodes);
    const empty = h("p", { class: "muted small chart-empty" }, t("agents.empty"));
    show(empty, max === 0);
    return h(
      "section",
      { class: "card" },
      h(
        "div",
        { class: "card-head" },
        h("span", {}, t("agents.per_day")),
        h("span", { class: "muted" }, max > 0 ? t("agents.chart_max", { value: count(max) }) : ""),
      ),
      chart,
      empty,
    );
  }
}
