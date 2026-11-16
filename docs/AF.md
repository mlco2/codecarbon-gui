# Coding-agent usage (app → `af`)

The *Agents* view shows what coding agents (Claude Code, Codex) use: tokens,
and an estimate of the impact of the remote inference. The numbers come from
[`af`](https://github.com/mlco2/agentic-footprint) (agentic-footprint), a
separate command-line tool with its own background collector. The app never
reads `af`'s files: it runs `af` and reads the JSON it prints
(`src-tauri/src/af.rs`). The panel does no I/O; it asks the Rust side.

## Where `af` is looked for

`PATH`, then `~/.local/bin/af` (its installer's default). Not found is a
normal state: the view explains it and links to the install instructions.
This version of the app does not download `af`.

## Commands the app runs

| Command | When | Changes the computer |
| --- | --- | --- |
| `af status` | Agents view opened, after an action, Settings opened | no |
| `af summary --period today\|7d\|30d --utc-offset-minutes N` | Agents view opened, then every 30 s while it is on screen | no |
| `af service install` | *Install the service* clicked | yes: a per-user background service |
| `af setup --yes --global` | *Configure agents* clicked | yes: the agents' own settings files |
| `af self update` | *Update af* clicked (Settings → Updates) | yes: replaces `af` with its latest release |

Every run has a time limit (10 s for the two reads, 60 s for the first two
actions, 5 min for an update) and a limit on how much output is kept.

## The contract

`af status` and `af summary` each print one JSON document with
`protocol_version` (currently `1` for both). `af summary` is specified by
`schemas/summary/v1.schema.json` in the agentic-footprint repository.

* Unknown fields are ignored, so `af` can add fields without breaking the app.
* Another `protocol_version` means "this `af` is not supported by this
  version of the app"; the view says so instead of guessing.
* The three actions have no JSON result: the app uses the exit code and shows
  what `af` printed.

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
| `not_found` | no `af` on this computer |
| `spawn_failed` | found, but it could not be started |
| `timed_out` | it did not answer in time (it is stopped) |
| `failed` | it exited with an error; its message is shown |
| `invalid_output` | it printed something that is not the expected JSON |
| `unsupported_protocol` | another `protocol_version` |
