/**
 * Thin typed wrapper around the Rust side (src-tauri/src/commands.rs).
 *
 * The types mirror the Rust structs serialised by serde (snake_case keys).
 * The panel holds no business logic: it renders `TrackerState` and calls
 * these commands.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { errorText, type AppError, type LanguageSetting } from "./i18n/i18n";

export type { AppError } from "./i18n/i18n";

export type Phase = "idle" | "starting" | "running" | "stopping" | "agent_error";

/** A value split by component (watts or kWh). */
export interface Breakdown {
  cpu: number;
  gpu: number;
  ram: number;
  total: number;
}

export interface ReadyInfo {
  protocol_version: number;
  agent_version: string;
  codecarbon_version: string;
  python_version: string;
  pid: number | null;
  platform: string;
}

export interface Hardware {
  cpu_model: string | null;
  cpu_count: number | null;
  cpu_physical_count: number | null;
  cpu_method: string | null;
  cpu_tdp_w: number | null;
  cpu_tdp_is_generic: boolean | null;
  gpu_model: string | null;
  gpu_count: number | null;
  gpu_method: string | null;
  ram_total_gb: number | null;
  ram_method: string | null;
  descriptions: string[];
}

export interface Location {
  country_name: string | null;
  country_iso_code: string | null;
  region: string | null;
  carbon_intensity_g_per_kwh: number | null;
}

export interface Measurement {
  ts: string;
  duration_s: number;
  power_w: Breakdown;
  energy_kwh: Breakdown;
  emissions_kg: number;
  emissions_rate_kg_s: number;
  cpu_utilization_percent: number | null;
  ram_used_gb: number | null;
  carbon_intensity_g_per_kwh: number | null;
}

export interface Summary {
  run_id: string;
  started_at: string;
  stopped_at: string;
  duration_s: number;
  energy_kwh: Breakdown;
  emissions_kg: number;
  emissions_rate_kg_s: number;
  avg_power_w: Breakdown;
  carbon_intensity_g_per_kwh: number | null;
}

export interface HistoryPoint {
  ts: string;
  duration_s: number;
  total_w: number;
  emissions_rate: number;
}

export interface TrackerState {
  phase: Phase;
  agent_ready: ReadyInfo | null;
  hardware: Hardware | null;
  location: Location | null;
  run_id: string | null;
  started_at: string | null;
  latest: Measurement | null;
  history: HistoryPoint[];
  last_summary: Summary | null;
  notes: string[];
  /** Translated with `errorMessage`. */
  error: AppError | null;
  account: AccountState;
  /** A newer release (Dashboard banner, tray item). */
  update: UpdateInfo | null;
  updater: UpdaterStatus;
}

// --- Updates (updater.rs) ----------------------------------------------------

export interface UpdateInfo {
  version: string;
  /** RFC 3339, from latest.json. */
  date: string | null;
  notes: string | null;
}

export type UpdatePhase = "idle" | "checking" | "downloading" | "installing";

export interface UpdaterStatus {
  /** false: this build has no updater key; nothing is ever checked. */
  configured: boolean;
  phase: UpdatePhase;
  /** A check completed during this run ("You are up to date"). */
  checked: boolean;
  /** Download progress 0–100; null while the size is unknown. */
  percent: number | null;
  /** `update_check_failed` or `update_install_failed`. */
  error: AppError | null;
}

// --- CodeCarbon dashboard account (protocol v2) ------------------------------

export type AccountPhase = "unknown" | "signed_out" | "signing_in" | "signed_in";

export interface User {
  name: string | null;
  email: string | null;
}

export interface Experiment {
  id: string;
  name: string;
}

export interface Project {
  id: string;
  name: string;
  experiments: Experiment[];
}

export interface Organization {
  id: string;
  name: string;
  projects: Project[];
}

export interface Catalog {
  organizations: Organization[];
}

export type ApiKind = "organization" | "project" | "experiment";

export interface Selection {
  organization_id: string;
  organization_name: string;
  project_id: string;
  project_name: string;
  experiment_id: string;
  experiment_name: string;
}

export interface Created {
  kind: string;
  id: string;
  parent_id: string | null;
}

export interface AccountState {
  phase: AccountPhase;
  user: User | null;
  api_endpoint: string | null;
  catalog: Catalog | null;
  catalog_loading: boolean;
  /** Login page of the pending sign-in ("open it again"). */
  auth_url: string | null;
  creating: boolean;
  last_created: Created | null;
  /** Waiting for the API key of this selection. */
  saving: Selection | null;
  last_error: AppError | null;
}

export interface ApiSettings {
  enabled: boolean;
  endpoint: string;
  api_key: string;
  experiment_id: string;
  /** Display only: where the uploads go. */
  organization_name: string;
  project_name: string;
  experiment_name: string;
}

export interface Settings {
  interval_secs: number;
  country_iso_code: string | null;
  region: string | null;
  save_csv: boolean;
  start_tracking_on_launch: boolean;
  launch_at_login: boolean;
  api: ApiSettings;
  pue: number | null;
  force_carbon_intensity_g_per_kwh: number | null;
  /** "system" follows the OS (navigator.languages). */
  language: LanguageSetting;
  /** Native notification when a measurement stops (not on quit). */
  notify_on_stop: boolean;
  /** The Dashboard's first-run card was closed ("Got it"). */
  welcome_dismissed: boolean;
  /** Background update checks (at launch, then daily). */
  check_updates: boolean;
}

/** What the Dashboard's first-run welcome card needs (`get_first_run_info`). */
export interface FirstRunInfo {
  /** `emissions.csv` holds at least one session. */
  has_sessions: boolean;
  /** `std::env::consts::OS`: "linux", "macos" or "windows". */
  platform: string;
}

// --- History (emissions.csv, see history.rs) --------------------------------

/** A past session, one row of `emissions.csv`. */
export interface HistorySession {
  /** Local time, "YYYY-MM-DDTHH:MM:SS" (no offset). */
  started_at: string;
  duration_s: number;
  energy_kwh: number;
  emissions_kg: number;
  avg_power_w: number;
  country_name: string | null;
  project_name: string;
  run_id: string;
}

export interface Totals {
  sessions: number;
  duration_s: number;
  energy_kwh: number;
  emissions_kg: number;
}

export interface DayTotal {
  /** "YYYY-MM-DD", local. */
  date: string;
  energy_kwh: number;
  emissions_kg: number;
  duration_s: number;
}

export interface HistorySummary {
  today: Totals;
  this_week: Totals;
  this_month: Totals;
  all_time: Totals;
  /** The last 30 days, oldest first, today last, zero-filled. */
  daily: DayTotal[];
}

/** `value` `unit` of `key` (raw number, formatted by equivalents.ts). */
export interface Equivalent {
  key: string;
  value: number;
  unit: string;
}

export interface HistoryPayload {
  summary: HistorySummary;
  /** Newest first, at most 500. */
  sessions: HistorySession[];
  equivalents_all_time: Equivalent[];
  equivalents_today: Equivalent[];
  skipped_rows: number;
  path: string;
}

// --- Coding agents, read from the `af` CLI (af.rs, docs/AF.md) ----------------

/**
 * What running `af` gave. Everything but "ok" is explained to the user
 * (agents.ts); "not_found" is a normal state, not an error.
 */
export type AfOutcome<T> =
  | { outcome: "ok"; data: T }
  | { outcome: "not_found" }
  | { outcome: "spawn_failed"; detail: string }
  | { outcome: "timed_out"; seconds: number }
  | { outcome: "failed"; exit_code: number | null; detail: string }
  | { outcome: "invalid_output"; detail: string }
  | { outcome: "unsupported_protocol"; found: number | null; supported: number };

export interface AfAgentStatus {
  /** "claude-code", "codex", … */
  id: string;
  name: string;
  /**
   * "receiving", "needs_restart", "waiting", "misconfigured",
   * "not_configured", "not_installed", "unknown" (or a state added since).
   */
  state: string;
  /** In `af`'s words (English). */
  detail: string | null;
  action: string | null;
}

/** `af status`, the fields the panel uses. */
export interface AfStatus {
  af_version: string;
  /** `af self update` manages this binary (a release install). */
  self_update: { managed: boolean };
  service: { supported: boolean; installed: boolean };
  receiver: { endpoint: string; reachable: boolean; note: string | null };
  /** `running` is null where it cannot be asked. */
  collector: { running: boolean | null };
  /**
   * `af`'s own Python runtime, needed for the impact estimates (null from
   * an `af` that does not report it). `provisioner` null: neither uv nor a
   * usable Python is installed, and uv is the thing to install.
   */
  python?: {
    ready: boolean;
    problem: string | null;
    fix: string | null;
    provisioner: string | null;
    uv_install_url: string | null;
  } | null;
  agents: AfAgentStatus[];
}

/** Measured: as reported by the agents. */
export interface AfTokens {
  input: number;
  output: number;
  thought: number;
  cache_read: number;
  cache_write: number;
  total: number;
}

/** A modelled impact: always a range. */
export interface AfImpact {
  unit: string;
  total: { min: number; max: number };
}

export interface AfFigures {
  calls: number;
  sessions: number;
  tokens: AfTokens;
  /** Calls whose impacts are in `remote`. */
  estimated_calls: number;
  unestimated_calls: number;
  unestimated_by_reason: Record<string, number>;
  /**
   * Modelled impacts of the remote inference, summed over
   * `estimated_calls` only. Keys: energy (kWh), gwp (kgCO2eq), water, adpe, pe.
   */
  remote: Record<string, AfImpact>;
}

export interface AfNotEstimated {
  agent: string;
  model: string;
  /** The estimator's status, e.g. "unknown_model". */
  reason: string;
  calls: number;
  tokens: number;
}

export type AfPeriod = "today" | "7d" | "30d";

/** `af summary` (schemas/summary/v1.schema.json in agentic-footprint). */
export interface AfSummary {
  af_version: string;
  generated_at: string;
  period: { name: string; from: string; to: string; utc_offset_minutes: number };
  collector: { running: boolean | null; last_event_at: string | null };
  totals: AfFigures;
  /** Largest token total first. */
  by_agent: (AfFigures & { agent: string; name: string })[];
  by_model: (AfFigures & { agent: string; model: string })[];
  /** Counted in the tokens, in no `remote` sum. */
  models_not_estimated: AfNotEstimated[];
  /** Every local day of the period ("YYYY-MM-DD"), oldest first. */
  by_day: (AfFigures & { date: string })[];
}

/** What an action printed. */
export interface AfActionOutput {
  output: string;
}

/** Views of the panel; also the payload of the `app://navigate` event. */
export type View = "dashboard" | "history" | "agents" | "settings" | "about";

const VIEWS: readonly string[] = ["dashboard", "history", "agents", "settings", "about"];

/** Narrow an `app://navigate` payload to a view name. */
export function isView(name: unknown): name is View {
  return typeof name === "string" && VIEWS.includes(name);
}

export const getState = (): Promise<TrackerState> => invoke<TrackerState>("get_state");
export const startTracking = (): Promise<void> => invoke("start_tracking");
export const stopTracking = (): Promise<void> => invoke("stop_tracking");
export const restartAgent = (): Promise<void> => invoke("restart_agent");
export const getSettings = (): Promise<Settings> => invoke<Settings>("get_settings");
/** The OS's preferred languages (BCP 47), the ones the tray follows. */
export const getSystemLanguages = (): Promise<string[]> => invoke<string[]>("get_system_languages");
export const saveSettings = (settings: Settings): Promise<Settings> =>
  invoke<Settings>("save_settings", { settings });
export const openDataFolder = (): Promise<void> => invoke("open_data_folder");
/** Reads `emissions.csv` on every call. */
export const getHistory = (): Promise<HistoryPayload> => invoke<HistoryPayload>("get_history");
/** Cheap: stops reading `emissions.csv` at the first session. */
export const getFirstRunInfo = (): Promise<FirstRunInfo> =>
  invoke<FirstRunInfo>("get_first_run_info");
/** Everyday equivalents of one session (the factors live in Rust). */
export const getEquivalents = (emissionsKg: number, energyKwh: number): Promise<Equivalent[]> =>
  invoke<Equivalent[]>("get_equivalents", { emissionsKg, energyKwh });
export const openUrl = (url: string): Promise<void> => invoke("open_url", { url });
export const quitApp = (): Promise<void> => invoke("quit_app");
/** The newer release, or null when up to date (the state follows too). */
export const checkForUpdates = (): Promise<UpdateInfo | null> =>
  invoke<UpdateInfo | null>("check_for_updates");
/** Starts the download; progress and errors arrive in `state.updater`. */
export const installUpdate = (): Promise<void> => invoke("install_update");

export const accountSignIn = (): Promise<void> => invoke("account_sign_in");
export const accountCancelSignIn = (): Promise<void> => invoke("account_cancel_sign_in");
/** Also forgets the API key / experiment and turns uploads off. */
export const accountSignOut = (): Promise<void> => invoke("account_sign_out");
export const accountRefreshCatalog = (): Promise<void> => invoke("account_refresh_catalog");
/** `parentId`: the organisation of a project, the project of an experiment. */
export const accountCreate = (
  kind: ApiKind,
  name: string,
  parentId: string | null,
  description: string | null = null,
): Promise<void> => invoke("account_create", { kind, name, description, parentId });
/** Fetches an API key for the project, then saves the settings (`onSettings`). */
export const accountSelectExperiment = (
  organizationId: string,
  projectId: string,
  experimentId: string,
): Promise<void> =>
  invoke("account_select_experiment", { organizationId, projectId, experimentId });

/** `af status`. Read-only. */
export const afStatus = (): Promise<AfOutcome<AfStatus>> =>
  invoke<AfOutcome<AfStatus>>("af_status");
/** `af summary`; days are cut at `utcOffsetMinutes` east of UTC. Read-only. */
export const afSummary = (
  period: AfPeriod,
  utcOffsetMinutes: number,
): Promise<AfOutcome<AfSummary>> =>
  invoke<AfOutcome<AfSummary>>("af_summary", { period, utcOffsetMinutes });
// The ones below change the user's machine: call them from a click only.
/**
 * Download the `af` release this app pins, check it against its hash, and
 * put it in the app's own folder.
 */
export const afInstall = (): Promise<AfOutcome<AfActionOutput>> =>
  invoke<AfOutcome<AfActionOutput>>("af_install");
/** `af service install`. */
export const afInstallService = (): Promise<AfOutcome<AfActionOutput>> =>
  invoke<AfOutcome<AfActionOutput>>("af_install_service");
/** `af setup --yes --global`: edits the agents' own configuration files. */
export const afSetupAgents = (): Promise<AfOutcome<AfActionOutput>> =>
  invoke<AfOutcome<AfActionOutput>>("af_setup_agents");
/** `af python setup`: `af` builds the Python runtime its estimates need. */
export const afSetupPython = (): Promise<AfOutcome<AfActionOutput>> =>
  invoke<AfOutcome<AfActionOutput>>("af_setup_python");
/** `af self update`: `af` downloads and verifies its own release. */
export const afSelfUpdate = (): Promise<AfOutcome<AfActionOutput>> =>
  invoke<AfOutcome<AfActionOutput>>("af_self_update");

/** Called with the full state after every change on the Rust side. */
export function onState(callback: (state: TrackerState) => void): Promise<UnlistenFn> {
  return listen<TrackerState>("tracker://state", (event) => callback(event.payload));
}

/** Called with the new settings after every save (any view, or the sign-in flow). */
export function onSettings(callback: (settings: Settings) => void): Promise<UnlistenFn> {
  return listen<Settings>("settings://changed", (event) => callback(event.payload));
}

/** The tray asks the panel to show a given view ("Settings…"). */
export function onNavigate(callback: (view: View) => void): Promise<UnlistenFn> {
  return listen<string>("app://navigate", (event) => {
    if (isView(event.payload)) callback(event.payload);
  });
}

/** A session stopped and was (probably) appended to `emissions.csv`. */
export function onHistoryChanged(callback: () => void): Promise<UnlistenFn> {
  return listen("history://changed", () => callback());
}

/**
 * Commands reject with an `AppError` (a code, see src-tauri/src/error.rs):
 * the text for the user, in the current language.
 */
export function errorMessage(error: unknown): string {
  return errorText(error);
}
