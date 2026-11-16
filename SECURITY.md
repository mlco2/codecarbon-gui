# Security

CodeCarbon Desktop is installed on people's personal computers, so we treat
the supply chain with the same care as the code.

## Reporting a vulnerability

Please do **not** open a public issue. Use GitHub's private reporting:
*Security → Report a vulnerability* on this repository, or e-mail the
CodeCarbon maintainers (see <https://codecarbon.io>). We aim to acknowledge
reports within a week.

## What the app does, and does not do

* It runs two processes: the Tauri shell and the `codecarbon-agent` sidecar
  (a frozen Python program). They talk over stdin/stdout only.
* **A third program, `af`, only on demand.** The *Agents* view shows what
  coding agents (Claude Code, Codex) use, read from
  [`af`](https://github.com/mlco2/agentic-footprint) when it is installed on
  the computer. The app starts `af` as a short-lived process, reads the JSON
  it prints, and nothing else (see [docs/AF.md](docs/AF.md)):
  * `af status` and `af summary` while the Agents view is open (and
    `af status` when Settings is opened). Both only read.
  * `af service install`, `af setup --yes --global`, `af python setup` and
    `af self update`, each only when the user clicks its button. These change the computer,
    and it is `af` that makes the changes: a per-user background service
    (launchd / systemd) that receives the agents' usage, the agents' own
    settings files (`~/.claude/settings.json`, Codex's `config.toml`) so that
    they send it, and `af`'s own state folder (including, for
    `af python setup`, a Python environment whose packages `af` downloads
    with uv or pip).
  * That background service belongs to `af`: the app does not start, stop or
    supervise it, and it keeps running when the app is closed.
  * **Installing `af`.** When no `af` is found, the view offers *Install
    af*. Only on that click, the app downloads one file over https, from
    `https://github.com/mlco2/agentic-footprint/releases/download/`: the
    `af` binary of the single release this version of the app pins. The
    file is compared with a SHA-256 written in the app's source
    (`src-tauri/src/af_release.rs`); if it differs, nothing is installed.
    It is placed where `af`'s own installer puts it (`~/.local/bin/af`;
    `%LOCALAPPDATA%\Programs\agentic-footprint\af.exe` on Windows), so
    the app and a terminal use the same file. Later versions come from `af self update`, which verifies the Sigstore
    signature of what it downloads.
* **Network access**: one geolocation request at the start of a measurement
  when no country is set in Settings, and uploads when the user enables the
  CodeCarbon API. In builds that have an updater key, also an update check
  (`https://github.com/mlco2/codecarbon-gui/releases/latest/download/latest.json`)
  shortly after launch and then once a day: on by default, off with
  *Settings → Updates*; the download itself only when the user clicks
  **Install**. There is no telemetry and no remote code other than signed
  updates (below). `af self update`, when the user clicks *Update af*, is
  `af` contacting GitHub for its own release and verifying that release's
  checksum and Sigstore signature itself; the app takes no part in it.
* **Files written**: `settings.json` in the app's config folder,
  `measurements/emissions.csv` in its data folder, and the WebView's cache.
  (What `af` writes when asked to is listed above.)
* **Privileges**: none. The app runs as the user and never asks for
  administrator rights; RAPL / `powermetrics` measurements are used only
  when the system already exposes them.
* The panel's WebView has a strict Content-Security-Policy and only the
  Tauri commands listed in `src-tauri/capabilities/default.json`.

## Automated checks

| What | Where | When |
| --- | --- | --- |
| CodeQL static analysis (TypeScript, Python, Rust, workflows) | `.github/workflows/codeql.yml` | every PR, push to main, weekly |
| Dependency advisories: `cargo audit`, `pip-audit`, `npm audit` | `.github/workflows/security.yml` | every PR, push to main, weekly |
| Dependency updates (Cargo, npm, pip, Actions) | `.github/dependabot.yml` | weekly |
| Build provenance attestations for every release asset | `.github/workflows/release.yml` | each release |
| `SHA256SUMS.txt` attached to every release | `.github/workflows/release.yml` | each release |
| Updater files (`latest.json`, `.sig`) signed with the project's updater key, verified by the app before installing | `.github/workflows/release.yml`, `src-tauri/src/updater.rs` | each release, once the key is set up |
| Linting, unit and end-to-end tests | `.github/workflows/ci.yml` | every PR |

Workflows run with read-only `GITHUB_TOKEN` permissions except the release
workflow, which needs `contents: write` to upload assets and `id-token` /
`attestations: write` to sign provenance.

## Automatic updates

The app uses Tauri's updater plugin. It downloads an update only from the
`endpoints` in `src-tauri/tauri.conf.json` (GitHub Releases, https), and
installs it only if its [minisign](https://jedisct1.github.io/minisign/)
signature verifies against the **public key compiled into the app**
(`plugins.updater.pubkey`). `requireSignedVersion` is on: the version is
part of what is signed, so a tampered `latest.json` cannot make the app
install an older, genuinely signed release. A compromised GitHub account
or release page alone is therefore not enough to push code to users; the
private key (repository secret `TAURI_SIGNING_PRIVATE_KEY`, protected by
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) is also needed. Keep an offline copy
of the key: if it is lost, existing installs cannot be updated any more
and users must download the app again by hand.

Builds whose config still holds the placeholder `REPLACE_WITH_UPDATER_PUBKEY`
do not load the updater at all and never contact the update endpoint. The
panel has no updater permission; only Rust checks and installs. Before
installing, the app stops a running measurement (so it is saved) and the
measurement engine, exactly as Quit does, then restarts.

## Repository settings to keep enabled

These live in *Settings → Code security* and cannot be set from the code:

* Dependabot alerts and security updates
* Secret scanning with push protection
* Code scanning (CodeQL) default alerts on pull requests
* Branch protection on `main`: require the CI and CodeQL checks, no force-push

## Verifying a download

Every release ships `SHA256SUMS.txt` and Sigstore build-provenance
attestations. To check that an installer is the one CI built from the tagged
source:

```bash
sha256sum -c SHA256SUMS.txt --ignore-missing
gh attestation verify CodeCarbon_0.1.0_aarch64.dmg -R mlco2/codecarbon-gui
```

## Known gaps (roadmap)

* The installers are **not code-signed** yet (no Apple / Microsoft
  certificate). macOS and Windows show a warning on first launch; the
  attestations above are the way to verify authenticity in the meantime.
* Automatic updates are switched on only once the maintainers have created
  the updater key (see `docs/DEVELOPMENT.md`). Installs built before that
  cannot update themselves and must download a new version once.
