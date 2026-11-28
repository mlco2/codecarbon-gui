# Coding-agent usage (app → `afoot`)

The *Agents* view shows what coding agents (Claude Code, Codex, OpenCode, Pi:
the ones `afoot` reports) use: tokens, and an estimate of the impact of the remote inference. The numbers come from
[`afoot`](https://github.com/mlco2/agentic-footprint) (agentic-footprint), a
separate command-line tool with its own background collector. The app never
reads `afoot`'s files: it runs `afoot` and reads the JSON it prints
(`src-tauri/src/afoot.rs`). The panel does no I/O; it asks the Rust side.

## Where `afoot` is looked for

`PATH`, then the folders user tools are installed to (`~/.local/bin`,
`~/.cargo/bin`, Homebrew's: an app started from the Dock does not get the
shell's `PATH`), then the folder `afoot`'s installers use
(`%LOCALAPPDATA%\Programs\agentic-footprint` on Windows). Not found is a normal state: the view explains it and
offers *Install afoot*.

## Installing afoot

*Install afoot* downloads the `afoot` binary of one release, pinned in
`src-tauri/src/afoot_release.rs` with the SHA-256 of the file for each platform
(macOS arm64 and x86-64, Linux x86-64, Windows x86-64). A download that does
not match its hash is discarded. The binary goes where `afoot`'s own installer
puts a release binary, and the app then runs `afoot self record-install`, as
the installer does, so that `afoot self update` manages it from then on. The
result is the installer's, with two differences for now: the folder is not
added to the user's `PATH` when it is missing from it, and the installer's
next steps (Python runtime, background service, agent configuration) are
not run at once but offered as buttons in the view.

To pin a newer release, change `VERSION` and the four hashes (from the
release's `SHA256SUMS`); the tests check their form.

## Commands the app runs

| Command | When | Changes the computer |
| --- | --- | --- |
| `afoot status` | Agents view opened, after an action, Settings opened | no |
| `afoot summary --period today\|7d\|30d --utc-offset-minutes N` | Agents view opened, then every 30 s while it is on screen | no |
| `afoot service install` | *Install the service* clicked | yes: a per-user background service |
| `afoot setup --yes --global` | *Configure agents* clicked | yes: the agents' own settings files, and the OpenCode plugin and Pi extension files |
| `afoot python setup` | *Set up estimates* clicked | yes: `afoot`'s own Python environment, in its state folder (downloads packages) |
| `afoot self update` | *Update afoot* clicked (Settings → Updates) | yes: replaces `afoot` with its latest release |

Every run has a time limit (10 s for the two reads, 60 s for the first two
actions, 5 min for the two that download) and a limit on how much output is kept.

`afoot` is started with the app's `PATH` plus the usual places user tools are
installed to (`~/.local/bin`, `~/.cargo/bin`, Homebrew's), because an app
started from the Dock does not have them and `afoot` looks for `uv` there.

When `afoot status` says its Python runtime is missing and that nothing on the
computer can build it (no `uv`, no usable Python), the view links to uv's
install instructions: uv brings the right Python with it. The app installs
neither.

## The contract

`afoot status` and `afoot summary` each print one JSON document with
`protocol_version` (currently `1` for both). `afoot summary` is specified by
`schemas/summary/v1.schema.json` in the agentic-footprint repository.

* Unknown fields are ignored, so `afoot` can add fields without breaking the app.
* Another `protocol_version` means "this `afoot` is not supported by this
  version of the app"; the view says so instead of guessing.
* The three actions have no JSON result: the app uses the exit code and shows
  what `afoot` printed.

## What the view shows, and what it does not

* **Tokens are measured** (reported by the agents), and shown split by kind:
  cache reads usually dominate the total.
* **Impacts are modelled**, for the model provider's servers, and come as a
  range (min–max). They are always shown with "N of M calls estimated", never
  as a single mid-point, never extrapolated to calls without an estimate.
* Models the estimator has no figures for are named (`models_not_estimated`):
  their calls and tokens are counted, their impact is in no figure.
* Nothing here is added to what CodeCarbon measures on this computer: local
  energy used by an agent is already part of the machine's energy.

## Each way it can fail

| Outcome (Rust → panel) | Meaning |
| --- | --- |
| `not_found` | no `afoot` on this computer |
| `spawn_failed` | found, but it could not be started |
| `timed_out` | it did not answer in time (it is stopped) |
| `failed` | it exited with an error; its message is shown |
| `invalid_output` | it printed something that is not the expected JSON |
| `unsupported_protocol` | another `protocol_version` |
