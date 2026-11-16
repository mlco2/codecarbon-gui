//! Coding-agent usage, read from `af` (agentic-footprint), a separate
//! command-line tool the user installs (docs/AF.md).
//!
//! * `af` is looked up in `PATH`, then in `~/.local/bin` (its installer's
//!   default; an app started from the Dock or a launcher does not get the
//!   shell's `PATH`). Not finding it is a normal state, not an error.
//! * Each call starts one short-lived `af` process with a hard time-out
//!   and reads its stdout: one JSON document carrying `protocol_version`.
//!   Every way this can go wrong is its own [`Outcome`], which the panel
//!   explains (the texts are in `src/i18n`).
//! * `af status` and `af summary` only read. The three actions
//!   ([`Action`]) change the user's machine: they run from a button click
//!   only, never by themselves.
//! * `af` has its own background service (the collector). This app never
//!   starts, stops or supervises it.
//!
//! This version does not download `af`: see [`locate`].

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::{async_runtime, AppHandle, Manager};

/// The `protocol_version` of `af status` and `af summary` this app reads.
pub const PROTOCOL_VERSION: u64 = 1;

/// The executable's file name.
const BINARY: &str = if cfg!(windows) { "af.exe" } else { "af" };

/// `af status` / `af summary` answer in well under a second.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// `af service install` and `af setup` write a few files and start a service.
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
/// `af self update` downloads a release.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How often a running `af` is checked for exit.
const POLL: Duration = Duration::from_millis(10);
/// After `af` exited: how long its output may still take to arrive (a
/// process it started can keep the pipe open; we do not wait for it).
const DRAIN: Duration = Duration::from_secs(1);
/// At most this much of stdout / stderr is kept (the rest is read and dropped).
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// At most this many characters of `af`'s own text go to the panel (the end).
const MAX_TEXT: usize = 4000;

/// Only one action runs at a time.
static ACTION: Mutex<()> = Mutex::new(());

/// What running `af` gave, as sent to the panel (`{"outcome": "ok", …}`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome<T> {
    Ok {
        data: T,
    },
    /// No `af` in `PATH` or `~/.local/bin`.
    NotFound,
    /// The file is there but could not be started (the OS's reason).
    SpawnFailed {
        detail: String,
    },
    /// Still running after `seconds`: killed.
    TimedOut {
        seconds: u64,
    },
    /// Non-zero exit (`exit_code` is `None` when a signal ended it);
    /// `detail` is the end of what `af` printed.
    Failed {
        exit_code: Option<i32>,
        detail: String,
    },
    /// Exit 0, but stdout is not the JSON document we expect.
    InvalidOutput {
        detail: String,
    },
    /// A JSON document of another `protocol_version` (`None`: it has none):
    /// this `af` is not supported by this version of the app.
    UnsupportedProtocol {
        found: Option<u64>,
        supported: u64,
    },
}

// --- `af status` ---------------------------------------------------------------

/// The fields of `af status` the panel uses. Paths and process ids, which
/// the document also carries, are not passed on.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Status {
    pub af_version: String,
    pub self_update: SelfUpdate,
    pub service: Service,
    pub receiver: Receiver,
    pub collector: StatusCollector,
    pub agents: Vec<AgentStatus>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SelfUpdate {
    /// `af self update` manages this binary (a release install).
    pub managed: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Service {
    /// This system has a per-user service manager `af` can use.
    pub supported: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Receiver {
    pub endpoint: String,
    pub reachable: bool,
    /// Why it does not answer, in `af`'s words.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StatusCollector {
    /// `None` where it cannot be asked (not Unix).
    pub running: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AgentStatus {
    /// `claude-code`, `codex`, …
    pub id: String,
    pub name: String,
    /// `receiving`, `needs_restart`, `waiting`, `misconfigured`,
    /// `not_configured`, `not_installed` or `unknown` (a string: a newer
    /// `af` may add states).
    pub state: String,
    pub detail: Option<String>,
    pub action: Option<String>,
}

// --- `af summary` --------------------------------------------------------------

/// `af summary` (the contract is `schemas/summary/v1.schema.json` in the
/// agentic-footprint repository). Fields we do not name are ignored.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Summary {
    pub af_version: String,
    pub generated_at: String,
    pub period: PeriodInfo,
    pub collector: SummaryCollector,
    pub totals: Figures,
    /// Largest token total first.
    pub by_agent: Vec<AgentFigures>,
    pub by_model: Vec<ModelFigures>,
    /// Models the estimator does not know: counted in tokens, in no `remote` sum.
    pub models_not_estimated: Vec<NotEstimated>,
    /// Every local day of the period, oldest first.
    pub by_day: Vec<DayFigures>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PeriodInfo {
    pub name: String,
    pub from: String,
    pub to: String,
    pub utc_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SummaryCollector {
    pub running: Option<bool>,
    pub last_event_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Figures {
    pub calls: u64,
    pub sessions: u64,
    /// Measured: as reported by the agents.
    pub tokens: Tokens,
    /// Calls whose impacts are in `remote`.
    pub estimated_calls: u64,
    pub unestimated_calls: u64,
    pub unestimated_by_reason: BTreeMap<String, u64>,
    /// Modelled impacts of the remote inference, over `estimated_calls`
    /// only. Keys: `energy` (kWh), `gwp` (kgCO2eq), `water`, `adpe`, `pe`.
    pub remote: BTreeMap<String, Impact>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub thought: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Impact {
    pub unit: String,
    pub total: Range,
}

/// An estimate is a range, never one number.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AgentFigures {
    pub agent: String,
    pub name: String,
    #[serde(flatten)]
    pub figures: Figures,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ModelFigures {
    pub agent: String,
    pub model: String,
    #[serde(flatten)]
    pub figures: Figures,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DayFigures {
    /// "YYYY-MM-DD", local.
    pub date: String,
    #[serde(flatten)]
    pub figures: Figures,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct NotEstimated {
    pub agent: String,
    pub model: String,
    /// The estimator's status, e.g. `unknown_model`.
    pub reason: String,
    pub calls: u64,
    pub tokens: u64,
}

/// The periods `af summary --period` takes (local calendar days, ending today).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Period {
    #[serde(rename = "today")]
    Today,
    #[serde(rename = "7d")]
    Days7,
    #[serde(rename = "30d")]
    Days30,
}

impl Period {
    fn arg(self) -> &'static str {
        match self {
            Period::Today => "today",
            Period::Days7 => "7d",
            Period::Days30 => "30d",
        }
    }
}

// --- Actions -------------------------------------------------------------------

/// The three things the user can ask `af` to do from the panel. Each one
/// changes the user's machine: see SECURITY.md for what they write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `af service install`: the per-user service that receives the
    /// agents' events.
    InstallService,
    /// `af setup --yes --global`: writes the agents' user configuration
    /// (`~/.claude/settings.json`, Codex's `config.toml`).
    SetupAgents,
    /// `af self update`: `af` downloads its own latest release from GitHub
    /// and verifies its Sigstore signature itself.
    SelfUpdate,
}

impl Action {
    fn args(self) -> &'static [&'static str] {
        match self {
            Action::InstallService => &["service", "install"],
            Action::SetupAgents => &["setup", "--yes", "--global"],
            Action::SelfUpdate => &["self", "update"],
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Action::SelfUpdate => UPDATE_TIMEOUT,
            Action::InstallService | Action::SetupAgents => ACTION_TIMEOUT,
        }
    }
}

/// What an action printed (stdout, then stderr; the end when it is long).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionOutput {
    pub output: String,
}

// --- Finding and running `af` --------------------------------------------------

/// Where `af` is looked up, and where it runs.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// The `PATH` variable.
    pub path: Option<OsString>,
    /// The user's home directory.
    pub home: Option<PathBuf>,
}

impl Env {
    fn of(app: &AppHandle) -> Self {
        Self {
            path: std::env::var_os("PATH"),
            home: app.path().home_dir().ok(),
        }
    }
}

/// The `af` to run: the first one in `path`, else `~/.local/bin/af`.
///
/// `None` is a normal answer: the panel then links to the install
/// instructions. A later version can download a pinned, verified release
/// of `af` into the app's data folder and look there first: that goes
/// HERE (there is no release to pin yet, so nothing is downloaded).
pub fn locate(env: &Env) -> Option<PathBuf> {
    let from_path = env
        .path
        .iter()
        .flat_map(|path| std::env::split_paths(path))
        // An empty or relative entry means "the current directory".
        .filter(|dir| dir.is_absolute());
    let installer_default = env.home.iter().map(|home| home.join(".local").join("bin"));
    from_path
        .chain(installer_default)
        .map(|dir| dir.join(BINARY))
        .find(|file| is_executable(file))
}

fn is_executable(file: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(file) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// A finished `af` process.
struct Finished {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

enum RunError {
    Spawn(String),
    TimedOut,
}

/// Run `program` to its end, or kill it after `timeout`. No stdin; stdout
/// and stderr are captured. It runs in `cwd` (the home directory), so that
/// nothing depends on where the app was started from: `af setup` also
/// looks at the agent settings of the directory it runs in.
fn run(
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<Finished, RunError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd.filter(|dir| dir.is_dir()) {
        command.current_dir(cwd);
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: no console window flashes on screen.
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| RunError::Spawn(e.to_string()))?;
    let stdout = collect(child.stdout.take());
    let stderr = collect(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::TimedOut);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Spawn(e.to_string()));
            }
        }
    };
    Ok(Finished {
        status,
        stdout: stdout.recv_timeout(DRAIN).unwrap_or_default(),
        stderr: stderr.recv_timeout(DRAIN).unwrap_or_default(),
    })
}

/// Read a pipe to its end on a thread (so a chatty process never blocks on
/// a full pipe) and send the text, at most [`MAX_OUTPUT`] bytes of it.
fn collect<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut chunk = [0u8; 8192];
        if let Some(mut pipe) = pipe {
            while let Ok(read) = pipe.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                let room = MAX_OUTPUT.saturating_sub(kept.len()).min(read);
                kept.extend_from_slice(&chunk[..room]);
            }
        }
        let _ = sender.send(String::from_utf8_lossy(&kept).into_owned());
    });
    receiver
}

/// The last [`MAX_TEXT`] characters, trimmed.
fn tail(text: &str) -> String {
    let text = text.trim();
    let skip = text.chars().count().saturating_sub(MAX_TEXT);
    text.chars().skip(skip).collect()
}

/// Run `af` and sort out how it ended; `ok` gets the stdout and stderr of
/// a run that exited with 0.
fn finish<T>(
    env: &Env,
    args: &[&str],
    timeout: Duration,
    ok: impl FnOnce(&str, &str) -> Outcome<T>,
) -> Outcome<T> {
    let Some(af) = locate(env) else {
        return Outcome::NotFound;
    };
    match run(&af, args, env.home.as_deref(), timeout) {
        Err(RunError::Spawn(detail)) => Outcome::SpawnFailed { detail },
        Err(RunError::TimedOut) => Outcome::TimedOut {
            seconds: timeout.as_secs(),
        },
        Ok(done) if !done.status.success() => Outcome::Failed {
            exit_code: done.status.code(),
            detail: tail(if done.stderr.trim().is_empty() {
                &done.stdout
            } else {
                &done.stderr
            }),
        },
        Ok(done) => ok(&done.stdout, &done.stderr),
    }
}

/// Read the JSON document `af` printed. The version is checked before the
/// shape, so a newer protocol is reported as such and not as bad output.
pub fn parse<T: DeserializeOwned>(stdout: &str) -> Outcome<T> {
    let document: serde_json::Value = match serde_json::from_str(stdout.trim()) {
        Ok(document) => document,
        Err(e) => {
            return Outcome::InvalidOutput {
                detail: e.to_string(),
            }
        }
    };
    let found = document
        .get("protocol_version")
        .and_then(serde_json::Value::as_u64);
    if found != Some(PROTOCOL_VERSION) {
        return Outcome::UnsupportedProtocol {
            found,
            supported: PROTOCOL_VERSION,
        };
    }
    match serde_json::from_value(document) {
        Ok(data) => Outcome::Ok { data },
        Err(e) => Outcome::InvalidOutput {
            detail: e.to_string(),
        },
    }
}

fn read_with<T: DeserializeOwned>(env: &Env, args: &[&str], timeout: Duration) -> Outcome<T> {
    finish(env, args, timeout, |stdout, _| parse(stdout))
}

fn act_with(env: &Env, args: &[&str], timeout: Duration) -> Outcome<ActionOutput> {
    finish(env, args, timeout, |stdout, stderr| Outcome::Ok {
        data: ActionOutput {
            output: tail(&format!("{}\n{}", stdout.trim(), stderr.trim())),
        },
    })
}

/// `af summary --period <period> --utc-offset-minutes=<minutes>`.
fn summary_args(period: Period, utc_offset_minutes: i32) -> [String; 4] {
    [
        "summary".into(),
        "--period".into(),
        period.arg().into(),
        // One argument, so that a negative offset is not read as an option.
        format!("--utc-offset-minutes={utc_offset_minutes}"),
    ]
}

/// A blocking call on the blocking pool (never on the main thread).
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Outcome<T> + Send + 'static,
) -> Outcome<T> {
    async_runtime::spawn_blocking(work)
        .await
        .unwrap_or_else(|e| Outcome::SpawnFailed {
            detail: e.to_string(),
        })
}

/// `af status`: what is installed, configured and running. Read-only.
pub async fn status(app: &AppHandle) -> Outcome<Status> {
    let env = Env::of(app);
    blocking(move || read_with(&env, &["status"], READ_TIMEOUT)).await
}

/// `af summary` for `period`, days cut at `utc_offset_minutes` east of UTC
/// (the panel's own offset, so "today" is the user's today). Read-only.
pub async fn summary(app: &AppHandle, period: Period, utc_offset_minutes: i32) -> Outcome<Summary> {
    let env = Env::of(app);
    blocking(move || {
        let args = summary_args(period, utc_offset_minutes);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        read_with(&env, &args, READ_TIMEOUT)
    })
    .await
}

/// Run one [`Action`] and return what `af` printed. Called from a button
/// click only.
pub async fn act(app: &AppHandle, action: Action) -> Outcome<ActionOutput> {
    let env = Env::of(app);
    log::info!("af: running `af {}`", action.args().join(" "));
    let outcome = blocking(move || {
        let _one_at_a_time = ACTION.lock().unwrap_or_else(|e| e.into_inner());
        act_with(&env, action.args(), action.timeout())
    })
    .await;
    match &outcome {
        Outcome::Ok { .. } => log::info!("af: `af {}` succeeded", action.args().join(" ")),
        _ => log::warn!("af: `af {}` did not succeed", action.args().join(" ")),
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::ffi::OsStr;

    fn env_with(path: &OsStr, home: Option<&Path>) -> Env {
        Env {
            path: Some(path.to_os_string()),
            home: home.map(Path::to_path_buf),
        }
    }

    /// A synthetic `af status` document (no real machine's output).
    fn status_json() -> serde_json::Value {
        json!({
            "protocol_version": 1,
            "af_version": "0.1.0",
            "binary": "/home/someone/.local/bin/af",
            "state_dir": "/home/someone/.local/state/af",
            "self_update": {"managed": true},
            "service": {"supported": true, "installed": false},
            "receiver": {
                "endpoint": "http://127.0.0.1:4318/v1/logs",
                "reachable": false,
                "note": "connection refused"
            },
            "collector": {"running": false},
            "agents": [
                {
                    "id": "claude-code",
                    "name": "Claude Code",
                    "state": "not_configured",
                    "detail": "no OTLP endpoint in the user settings",
                    "action": "run af setup",
                    "sessions": 0,
                    "processes": [{"pid": 1, "cwd": "/home/someone/project"}]
                },
                {"id": "codex", "name": "Codex", "state": "not_installed",
                 "detail": null, "action": null}
            ],
            "a_field_from_the_future": [1, 2, 3]
        })
    }

    fn figures(calls: u64, estimated: u64, tokens: u64) -> serde_json::Value {
        json!({
            "calls": calls,
            "sessions": 1,
            "tokens": {
                "input": tokens / 10, "output": tokens / 10, "thought": 0,
                "cache_read": tokens * 7 / 10, "cache_write": tokens / 10,
                "total": tokens
            },
            "estimated_calls": estimated,
            "unestimated_calls": calls - estimated,
            "unestimated_by_reason": {"unknown_model": calls - estimated},
            "remote": {
                "energy": {"unit": "kWh", "total": {"min": 0.001, "max": 0.004}},
                "gwp": {"unit": "kgCO2eq", "total": {"min": 0.0004, "max": 0.002}}
            }
        })
    }

    /// A synthetic `af summary` document following the v1 schema.
    fn summary_json() -> serde_json::Value {
        let with = |extra: serde_json::Value, figures: serde_json::Value| {
            let mut object = figures;
            for (key, value) in extra.as_object().unwrap() {
                object[key] = value.clone();
            }
            object
        };
        json!({
            "protocol_version": 1,
            "af_version": "0.1.0",
            "generated_at": "2026-03-02T10:00:00.000Z",
            "period": {
                "name": "7d",
                "from": "2026-02-24T05:00:00.000Z",
                "to": "2026-03-02T10:00:00.000Z",
                "utc_offset_minutes": -300
            },
            "collector": {"running": true, "last_event_at": "2026-03-02T09:59:00.000Z"},
            "totals": figures(10, 8, 1000),
            "by_agent": [with(
                json!({"agent": "claude-code", "name": "Claude Code"}),
                figures(10, 8, 1000)
            )],
            "by_model": [
                with(json!({"agent": "claude-code", "model": "model-a"}), figures(8, 8, 800)),
                with(json!({"agent": "claude-code", "model": "model-b"}), figures(2, 0, 200))
            ],
            "models_not_estimated": [{
                "agent": "claude-code", "model": "model-b",
                "reason": "unknown_model", "calls": 2, "tokens": 200
            }],
            "by_day": [with(json!({"date": "2026-03-02"}), figures(10, 8, 1000))]
        })
    }

    #[test]
    fn status_keeps_what_the_panel_uses_and_ignores_the_rest() {
        let Outcome::Ok { data } = parse::<Status>(&status_json().to_string()) else {
            panic!("not parsed");
        };
        assert_eq!(data.af_version, "0.1.0");
        assert!(data.self_update.managed);
        assert!(data.service.supported && !data.service.installed);
        assert_eq!(data.receiver.note.as_deref(), Some("connection refused"));
        assert_eq!(data.collector.running, Some(false));
        assert_eq!(data.agents.len(), 2);
        assert_eq!(data.agents[0].state, "not_configured");
        assert_eq!(data.agents[1].detail, None);
        // Paths and process ids stay on the Rust side.
        let sent = serde_json::to_string(&Outcome::Ok { data }).unwrap();
        assert!(sent.starts_with(r#"{"outcome":"ok","data":{"#), "{sent}");
        assert!(
            !sent.contains("/home/someone") && !sent.contains("processes"),
            "{sent}"
        );
    }

    #[test]
    fn collector_running_may_be_null() {
        let mut status = status_json();
        status["collector"]["running"] = json!(null);
        let Outcome::Ok { data } = parse::<Status>(&status.to_string()) else {
            panic!("not parsed");
        };
        assert_eq!(data.collector.running, None);
    }

    #[test]
    fn summary_is_read_and_sent_flat() {
        let Outcome::Ok { data } = parse::<Summary>(&summary_json().to_string()) else {
            panic!("not parsed");
        };
        assert_eq!(data.totals.calls, 10);
        assert_eq!(data.totals.estimated_calls, 8);
        assert_eq!(data.totals.tokens.cache_read, 700);
        assert_eq!(data.totals.remote["energy"].unit, "kWh");
        assert_eq!(data.totals.remote["gwp"].total.max, 0.002);
        assert_eq!(data.by_agent[0].name, "Claude Code");
        assert_eq!(data.by_model[1].figures.unestimated_calls, 2);
        assert_eq!(data.models_not_estimated[0].tokens, 200);
        assert_eq!(data.by_day[0].date, "2026-03-02");
        assert_eq!(data.period.utc_offset_minutes, -300);
        // The panel reads the same flat shape `af` printed.
        let sent = serde_json::to_value(&data).unwrap();
        assert_eq!(sent["by_agent"][0]["tokens"]["total"], 1000);
        assert_eq!(sent["by_day"][0]["calls"], 10);
    }

    #[test]
    fn an_empty_period_is_a_valid_summary() {
        let mut summary = summary_json();
        for list in ["by_agent", "by_model", "models_not_estimated"] {
            summary[list] = json!([]);
        }
        summary["totals"]["remote"] = json!({});
        summary["collector"] = json!({"running": null, "last_event_at": null});
        let Outcome::Ok { data } = parse::<Summary>(&summary.to_string()) else {
            panic!("not parsed");
        };
        assert!(data.totals.remote.is_empty());
        assert_eq!(data.collector.running, None);
    }

    #[test]
    fn output_that_is_not_the_document_is_invalid() {
        for text in ["", "af 0.1.0", "<html>", "{\"protocol_version\": 1"] {
            assert!(
                matches!(parse::<Status>(text), Outcome::InvalidOutput { .. }),
                "{text:?}"
            );
        }
        // The right version, but not the shape of `af status`.
        let wrong_shape = json!({"protocol_version": 1, "af_version": 7});
        assert!(matches!(
            parse::<Status>(&wrong_shape.to_string()),
            Outcome::InvalidOutput { .. }
        ));
    }

    #[test]
    fn another_protocol_version_is_unsupported_whatever_its_shape() {
        let mut status = status_json();
        status["protocol_version"] = json!(2);
        assert_eq!(
            parse::<Status>(&status.to_string()),
            Outcome::UnsupportedProtocol {
                found: Some(2),
                supported: 1
            }
        );
        assert_eq!(
            parse::<Summary>(r#"{"protocol_version": 2, "everything": "else"}"#),
            Outcome::UnsupportedProtocol {
                found: Some(2),
                supported: 1
            }
        );
        assert_eq!(
            parse::<Summary>(r#"{"hello": "world"}"#),
            Outcome::UnsupportedProtocol {
                found: None,
                supported: 1
            }
        );
    }

    #[test]
    fn outcomes_are_tagged_for_the_panel() {
        let tag = |outcome: Outcome<()>| serde_json::to_value(outcome).unwrap();
        assert_eq!(tag(Outcome::NotFound), json!({"outcome": "not_found"}));
        assert_eq!(
            tag(Outcome::TimedOut { seconds: 10 }),
            json!({"outcome": "timed_out", "seconds": 10})
        );
        assert_eq!(
            tag(Outcome::Failed {
                exit_code: None,
                detail: "killed".into()
            }),
            json!({"outcome": "failed", "exit_code": null, "detail": "killed"})
        );
        assert_eq!(
            tag(Outcome::SpawnFailed { detail: "x".into() })["outcome"],
            "spawn_failed"
        );
        assert_eq!(
            tag(Outcome::InvalidOutput { detail: "x".into() })["outcome"],
            "invalid_output"
        );
        assert_eq!(
            tag(Outcome::UnsupportedProtocol {
                found: Some(2),
                supported: 1
            }),
            json!({"outcome": "unsupported_protocol", "found": 2, "supported": 1})
        );
    }

    #[test]
    fn periods_and_arguments() {
        let period = |text: &str| serde_json::from_value::<Period>(json!(text));
        assert_eq!(period("today").unwrap(), Period::Today);
        assert_eq!(period("7d").unwrap(), Period::Days7);
        assert_eq!(period("30d").unwrap(), Period::Days30);
        assert!(period("1y").is_err());
        assert!(period("7d --state-dir /").is_err());
        assert_eq!(
            summary_args(Period::Days30, -300),
            ["summary", "--period", "30d", "--utc-offset-minutes=-300"]
        );
        assert_eq!(Action::InstallService.args(), ["service", "install"]);
        assert_eq!(Action::SetupAgents.args(), ["setup", "--yes", "--global"]);
        assert_eq!(Action::SelfUpdate.args(), ["self", "update"]);
    }

    #[test]
    fn long_text_keeps_its_end() {
        let text = format!("{}the end\n", "x".repeat(2 * MAX_TEXT));
        let kept = tail(&text);
        assert_eq!(kept.chars().count(), MAX_TEXT);
        assert!(kept.ends_with("the end"));
        assert_eq!(tail("  short \n"), "short");
    }

    #[test]
    fn nothing_to_find_without_path_and_home() {
        assert_eq!(locate(&Env::default()), None);
    }

    /// For developers: `cargo test -- --ignored real_af` reads the `af`
    /// installed on this machine (both commands are read-only) and prints
    /// nothing of what it says.
    #[test]
    #[ignore = "needs af on this machine"]
    fn real_af_answers_in_the_shape_we_read() {
        let env = Env {
            path: std::env::var_os("PATH"),
            home: std::env::var_os("HOME").map(PathBuf::from),
        };
        assert!(locate(&env).is_some(), "af is not installed here");
        let status = read_with::<Status>(&env, &["status"], READ_TIMEOUT);
        assert!(matches!(status, Outcome::Ok { .. }), "af status");
        for period in [Period::Today, Period::Days7, Period::Days30] {
            for offset in [-300, 0, 60] {
                let args = summary_args(period, offset);
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let summary = read_with::<Summary>(&env, &args, READ_TIMEOUT);
                assert!(matches!(summary, Outcome::Ok { .. }), "af summary {args:?}");
            }
        }
    }

    /// Fake `af` scripts in a temporary directory.
    #[cfg(unix)]
    mod with_a_fake_af {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU32, Ordering};

        /// One test at a time writes and starts scripts: a script still
        /// open for writing in another thread's fork cannot be executed
        /// ("Text file busy").
        static SCRIPTS: Mutex<()> = Mutex::new(());
        static NEXT: AtomicU32 = AtomicU32::new(0);

        fn temp_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "cc-af-{}-{}-{name}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn write(file: &Path, text: &str, mode: u32) {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(mode)).unwrap();
        }

        /// A directory holding an `af` that runs `body` (sh), as an [`Env`]
        /// whose `PATH` is that directory alone.
        fn fake(name: &str, body: &str) -> (PathBuf, Env) {
            let dir = temp_dir(name);
            write(&dir.join("af"), &format!("#!/bin/sh\n{body}\n"), 0o755);
            let env = env_with(dir.as_os_str(), None);
            (dir, env)
        }

        const SHORT: Duration = Duration::from_secs(20);

        #[test]
        fn path_comes_first_then_the_installers_default() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let dir = temp_dir("locate");
            let (first, second, home) = (dir.join("first"), dir.join("second"), dir.join("home"));
            let in_home = home.join(".local/bin/af");
            write(&in_home, "#!/bin/sh\n", 0o755);
            std::fs::create_dir_all(&first).unwrap();
            write(&second.join("af"), "#!/bin/sh\n", 0o755);

            let path = std::env::join_paths([&first, &second]).unwrap();
            assert_eq!(
                locate(&env_with(&path, Some(&home))),
                Some(second.join("af")),
                "PATH order, before ~/.local/bin"
            );
            assert_eq!(
                locate(&env_with(first.as_os_str(), Some(&home))),
                Some(in_home.clone()),
                "not in PATH: the installer's default"
            );
            assert_eq!(
                locate(&Env {
                    path: None,
                    home: Some(home.clone())
                }),
                Some(in_home)
            );
            assert_eq!(locate(&env_with(first.as_os_str(), None)), None);
            assert_eq!(locate(&env_with(first.as_os_str(), Some(&dir))), None);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn only_an_executable_file_in_an_absolute_directory_counts() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let dir = temp_dir("exec");
            let (plain, folder) = (dir.join("plain"), dir.join("folder"));
            write(&plain.join("af"), "#!/bin/sh\n", 0o644);
            std::fs::create_dir_all(folder.join("af")).unwrap();
            let path = std::env::join_paths([&plain, &folder]).unwrap();
            assert_eq!(
                locate(&env_with(&path, None)),
                None,
                "not executable; a directory"
            );
            // "" and "." in PATH mean the current directory: never searched.
            assert_eq!(locate(&env_with(OsStr::new(":.:bin"), None)), None);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn reads_the_document_a_working_af_prints() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            // Answers `summary --period 7d --utc-offset-minutes=-300` only.
            let body = format!(
                "[ \"$*\" = 'summary --period 7d --utc-offset-minutes=-300' ] || exit 64\n\
                 echo 'a warning' >&2\n\
                 cat <<'EOF'\n{}\nEOF",
                summary_json()
            );
            let (dir, env) = fake("summary", &body);
            let args = summary_args(Period::Days7, -300);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let Outcome::Ok { data } = read_with::<Summary>(&env, &args, SHORT) else {
                panic!("summary not read");
            };
            assert_eq!(data.totals.tokens.total, 1000);
            // Other arguments: the script's exit code 64.
            assert!(matches!(
                read_with::<Summary>(&env, &["summary"], SHORT),
                Outcome::Failed {
                    exit_code: Some(64),
                    ..
                }
            ));
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_missing_af_is_not_found() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let dir = temp_dir("missing");
            let env = env_with(dir.as_os_str(), Some(&dir));
            assert_eq!(
                read_with::<Status>(&env, &["status"], SHORT),
                Outcome::NotFound
            );
            assert_eq!(
                act_with(&env, Action::SelfUpdate.args(), SHORT),
                Outcome::NotFound
            );
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_non_zero_exit_gives_the_code_and_what_af_said() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let (dir, env) = fake(
                "exit",
                "echo '{\"protocol_version\": 1}'\necho 'error: no state directory' >&2\nexit 3",
            );
            assert_eq!(
                read_with::<Status>(&env, &["status"], SHORT),
                Outcome::Failed {
                    exit_code: Some(3),
                    detail: "error: no state directory".into()
                }
            );
            let _ = std::fs::remove_dir_all(dir);
            // Nothing on stderr: the end of stdout is the reason.
            let (dir, env) = fake("exit-stdout", "echo 'cannot do that'\nexit 1");
            assert_eq!(
                read_with::<Status>(&env, &["status"], SHORT),
                Outcome::Failed {
                    exit_code: Some(1),
                    detail: "cannot do that".into()
                }
            );
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn output_that_is_not_json_is_invalid() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let (dir, env) = fake("text", "echo 'Usage: af <COMMAND>'");
            assert!(matches!(
                read_with::<Status>(&env, &["status"], SHORT),
                Outcome::InvalidOutput { .. }
            ));
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn another_protocol_version_is_unsupported() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let (dir, env) = fake(
                "protocol",
                "echo '{\"protocol_version\": 2, \"af_version\": \"9.0.0\"}'",
            );
            assert_eq!(
                read_with::<Status>(&env, &["status"], SHORT),
                Outcome::UnsupportedProtocol {
                    found: Some(2),
                    supported: 1
                }
            );
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_hung_af_is_killed_at_the_time_out() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let (dir, env) = fake("hang", "exec sleep 30");
            let started = Instant::now();
            assert_eq!(
                read_with::<Status>(&env, &["status"], Duration::from_millis(300)),
                Outcome::TimedOut { seconds: 0 }
            );
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "not waited for"
            );
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_file_that_cannot_be_started_is_a_spawn_failure() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let dir = temp_dir("spawn");
            write(&dir.join("af"), "#!/nonexistent/interpreter\n", 0o755);
            let env = env_with(dir.as_os_str(), None);
            assert!(matches!(
                read_with::<Status>(&env, &["status"], SHORT),
                Outcome::SpawnFailed { .. }
            ));
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn an_action_returns_what_af_printed() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let (dir, env) = fake(
                "action",
                "[ \"$*\" = 'setup --yes --global' ] || { echo \"unexpected: $*\" >&2; exit 2; }\n\
                 echo 'configured 2 agents'\necho 'note: restart your agents' >&2",
            );
            assert_eq!(
                act_with(&env, Action::SetupAgents.args(), SHORT),
                Outcome::Ok {
                    data: ActionOutput {
                        output: "configured 2 agents\nnote: restart your agents".into()
                    }
                }
            );
            assert_eq!(
                act_with(&env, Action::InstallService.args(), SHORT),
                Outcome::Failed {
                    exit_code: Some(2),
                    detail: "unexpected: service install".into()
                }
            );
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn af_runs_in_the_home_directory() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            let (dir, mut env) = fake("cwd", "pwd");
            let home = dir.join("home");
            std::fs::create_dir_all(&home).unwrap();
            env.home = Some(home.clone());
            let Outcome::Ok { data } = act_with(&env, &[], SHORT) else {
                panic!("not run");
            };
            assert_eq!(
                std::fs::canonicalize(data.output).unwrap(),
                std::fs::canonicalize(home).unwrap()
            );
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn output_larger_than_a_pipe_does_not_block() {
            let _guard = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
            // 300 kB on each stream: more than a pipe buffer holds.
            let (dir, env) = fake(
                "large",
                "i=0\nwhile [ $i -lt 3000 ]; do\n\
                 printf '%0100d\\n' 0\nprintf '%0100d\\n' 0 >&2\ni=$((i+1))\ndone\necho done",
            );
            let Outcome::Ok { data } = act_with(&env, &[], SHORT) else {
                panic!("not run");
            };
            assert_eq!(data.output.chars().count(), MAX_TEXT);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
