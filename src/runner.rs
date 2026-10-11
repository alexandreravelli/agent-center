//! Runs catalog scripts: as the user, or as root through sudo or pkexec.
//!
//! Every script runs with `sh` after the [`PRELUDE`], which defines helpers such as `download`,
//! `apt_install` or `npm_global`:
//!
//! ```
//! use agent_center::runner::user_command;
//!
//! let out = user_command("have sh && log ready").output()?;
//! assert_eq!(String::from_utf8_lossy(&out.stdout), "==> ready\n");
//! # Ok::<(), std::io::Error>(())
//! ```

use crate::catalog::{Action, App, Catalog, Scope};
use std::collections::HashSet;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

/// Shell helpers available to every catalog script (`sh`, which is dash on Debian).
pub const PRELUDE: &str = r#"set -eu
export DEBIAN_FRONTEND=noninteractive
PATH="$HOME/.local/bin:/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin${PATH:+:$PATH}"
export PATH
ARCH=$(dpkg --print-architecture 2>/dev/null || echo amd64)
AC_STATE=/var/lib/agent-center
AC_TMP=$(mktemp -d)
trap 'rm -rf "$AC_TMP"' EXIT

# log MESSAGE: prints a step of the script.
log() { printf '==> %s\n' "$*"; }
# die MESSAGE: stops the script with an error.
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
# have COMMAND: true when COMMAND is in PATH.
have() { command -v "$1" >/dev/null 2>&1; }
# need COMMAND HINT: stops when COMMAND is missing.
need() { have "$1" || die "$1 is not installed. $2"; }
# download URL [NAME]: downloads into a temporary directory and prints the file's path.
download() {
    _f="$AC_TMP/${2:-$(basename "${1%%\?*}")}"
    curl -fL --retry 3 --proto '=https' -sS -o "$_f" "$1" || die "cannot download $1"
    printf '%s\n' "$_f"
}
# run_installer URL [ARGS...]: downloads a vendor's install script, then runs it with bash.
run_installer() {
    _url=$1; shift
    log "Running $_url"
    _s=$(download "$_url" install.sh)
    bash "$_s" "$@"
}
# gh_asset OWNER/REPO REGEX: prints the download URL of the latest release's asset matching REGEX.
gh_asset() {
    curl -fsSL -H 'Accept: application/vnd.github+json' "https://api.github.com/repos/$1/releases/latest" \
        | grep -o '"browser_download_url": *"[^"]*"' | sed 's/^.*"\(https:[^"]*\)"$/\1/' \
        | grep -E -- "$2" | head -n 1
}
# repo_version URL PACKAGE: the newest version of PACKAGE in an APT repository's package index
# (URL is the .../binary-amd64 directory).
repo_version() {
    { curl -fsSL "$1/Packages" 2>/dev/null || curl -fsSL "$1/Packages.gz" | gzip -dc; } \
        | awk -v p="$2" '/^Package: / { ok = ($2 == p) } ok && /^Version: / { print $2 }' | sort -V | tail -n 1
}

# deb_version PACKAGE: prints the installed version of a Debian package (nothing if absent).
deb_version() {
    [ -n "$1" ] || return 0
    dpkg-query -W -f='${db:Status-Abbrev} ${Version}\n' "$1" 2>/dev/null | awk '$1 == "ii" { print $2 }'
}
# apt_key URL FILE: stores a repository signing key at FILE, the path the vendor documents.
# A .gpg FILE gets a binary key (armored keys are converted with gpg --dearmor, as vendors do).
apt_key() {
    log "Adding the signing key $2"
    install -d -m 0755 "$(dirname "$2")"
    _k=$(download "$1" key)
    if head -c 40 "$_k" | grep -q -- '-----BEGIN PGP' && [ "${2%.gpg}" != "$2" ]; then
        have gpg || apt_install gpg
        gpg --batch --yes --dearmor -o "$AC_TMP/key.gpg" "$_k"
        _k="$AC_TMP/key.gpg"
    fi
    install -m 0644 "$_k" "$2"
}
# apt_install PACKAGE...: refreshes the package lists and installs packages (or local .deb files).
apt_install() {
    log "apt-get install $*"
    apt-get -q update
    apt-get -y -q -o DPkg::Lock::Timeout=300 install "$@"
}
# apt_upgrade PACKAGE...: refreshes the package lists and upgrades installed packages.
apt_upgrade() {
    log "apt-get install --only-upgrade $*"
    apt-get -q update
    apt-get -y -q -o DPkg::Lock::Timeout=300 install --only-upgrade "$@"
}
# apt_purge PACKAGE...: removes packages and their system-wide configuration.
apt_purge() {
    log "apt-get purge $*"
    apt-get -y -q -o DPkg::Lock::Timeout=300 purge "$@"
}
# drop FILE...: deletes repository files, keys and pins, then refreshes the package lists.
drop() {
    for _f in "$@"; do
        if [ -e "$_f" ]; then log "Removing $_f"; rm -f "$_f"; fi
    done
    apt-get -q update || true
}
# deb_from_url URL [ID]: downloads a .deb and installs it with its dependencies. With ID, the
# package name is remembered for deb_of (for vendors that do not document it).
deb_from_url() {
    [ -n "$1" ] || die "no download found for this system"
    _d=$(download "$1" package.deb)
    _p=$(dpkg-deb -f "$_d" Package)
    apt_install "$_d"
    if [ -n "${2:-}" ]; then
        install -d -m 0755 "$AC_STATE"
        printf '%s\n' "$_p" > "$AC_STATE/$2.package"
    fi
}
# deb_of ID [DEFAULT]: the package name remembered by deb_from_url.
deb_of() { cat "$AC_STATE/$1.package" 2>/dev/null || printf '%s\n' "${2:-}"; }
# deb_forget ID: purges the package remembered by deb_from_url.
deb_forget() {
    _p=$(deb_of "$1" "${2:-}")
    [ -n "$_p" ] || die "unknown package name for $1"
    apt_purge "$_p"
    rm -f "$AC_STATE/$1.package"
}

# npm_global PACKAGE...: installs or updates global npm packages (Node.js in /usr/local).
npm_global() {
    need npm "Install Node.js first."
    log "npm install -g $*"
    npm install -g --no-fund --no-audit "$@"
}
# npm_remove PACKAGE...: removes global npm packages.
npm_remove() {
    need npm "Install Node.js first."
    log "npm uninstall -g $*"
    npm uninstall -g --no-fund --no-audit "$@"
}
# npm_version PACKAGE: prints the version of a global npm package (nothing if absent).
npm_version() {
    for _r in /usr/local/lib/node_modules /usr/lib/node_modules "$(npm root -g 2>/dev/null || true)"; do
        if [ -f "$_r/$1/package.json" ]; then
            sed -n 's/^[[:space:]]*"version":[[:space:]]*"\([^"]*\)".*/\1/p' "$_r/$1/package.json" | head -n 1
            return 0
        fi
    done
}
"#;

/// A shell command running `script` with the prelude.
#[must_use]
pub fn user_command(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(format!("{PRELUDE}\n{script}"));
    cmd
}

/// PATH of the scripts that run as root: system directories only.
const ROOT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
/// What root scripts keep from the environment: the locale, the terminal and the proxies that
/// sudo or pkexec let through.
const ROOT_KEEP: [&str; 12] = [
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "TERM",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
];

/// A shell command running `script` as root, in a fixed environment: a sudoers file that keeps
/// HOME, or `sudo -E`, must not put a directory the user can write to in root's PATH.
#[must_use]
pub fn system_command(script: &str) -> Command {
    let mut cmd = user_command(script);
    cmd.env_clear().env("PATH", ROOT_PATH).env("HOME", "/root").env("USER", "root").env("LOGNAME", "root");
    for key in ROOT_KEEP {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd
}

/// One step of a job.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Id of the app.
    pub id: String,
    /// What to do with it.
    pub action: Action,
    /// After a removal, also delete the app's settings and data in the home directory.
    pub purge: bool,
}

/// Adds the installation of missing requirements before the steps that need them.
///
/// ```
/// use agent_center::catalog::{Action, Catalog};
/// use agent_center::runner::{Step, with_requirements};
///
/// let cat = Catalog::parse(r#"
///     [[app]]
///     id = "node"
///     name = "Node.js"
///     category = "tools"
///     version = "node --version"
///     install = "apt_install nodejs"
///     remove = "apt_purge nodejs"
///
///     [[app]]
///     id = "codex"
///     name = "Codex CLI"
///     category = "agents"
///     requires = ["node"]
///     version = "codex --version"
///     install = "npm_global @openai/codex"
///     remove = "npm_remove @openai/codex"
/// "#)?;
/// let install = |id: &str| Step { id: id.into(), action: Action::Install, purge: false };
///
/// // Node.js is missing, so it is installed first.
/// let steps = with_requirements(&cat, vec![install("codex")], &|_| false);
/// assert_eq!(steps, vec![install("node"), install("codex")]);
///
/// // Node.js is there: nothing is added.
/// let steps = with_requirements(&cat, vec![install("codex")], &|app| app.id == "node");
/// assert_eq!(steps, vec![install("codex")]);
/// # Ok::<(), agent_center::catalog::ParseError>(())
/// ```
#[must_use]
pub fn with_requirements(cat: &Catalog, steps: Vec<Step>, is_installed: &dyn Fn(&App) -> bool) -> Vec<Step> {
    /// Queues the missing requirements of `id`, deepest first. `seen` holds the apps already
    /// queued or being visited, so a requirement cycle ends.
    fn visit<'a>(
        cat: &'a Catalog,
        id: &str,
        is_installed: &dyn Fn(&App) -> bool,
        res: &mut Vec<Step>,
        seen: &mut HashSet<&'a str>,
    ) {
        let Some(app) = cat.get(id) else { return };
        for r in &app.requires {
            if let Some(dep) = cat.get(r)
                && !is_installed(dep)
                && seen.insert(dep.id.as_str())
            {
                visit(cat, r, is_installed, res, seen);
                res.push(Step { id: r.clone(), action: Action::Install, purge: false });
            }
        }
    }
    let mut res = Vec::new();
    let mut seen = HashSet::new();
    for step in steps {
        if step.action == Action::Install
            && let Some(app) = cat.get(&step.id)
        {
            seen.insert(app.id.as_str());
            visit(cat, &step.id, is_installed, &mut res, &mut seen);
        }
        if !res.contains(&step) {
            res.push(step);
        }
    }
    res
}

/// True when this process runs as root.
#[must_use]
pub fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("Uid:")).map(|l| l.split_whitespace().nth(2) == Some("0")))
        .unwrap_or(false)
}

/// How a script reaches root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    /// The terminal's sudo, which asks for the password itself.
    SudoTty,
    /// sudo with Agent Center's password window.
    SudoAskpass,
    /// pkexec, which asks through the desktop's polkit agent.
    Pkexec,
}

/// The password window sudo opens (`SUDO_ASKPASS`), installed by the package.
pub const ASKPASS: &str = "/usr/lib/agent-center/askpass";

/// How system scripts will reach root: the terminal's sudo when there is one
/// (`interactive_tty`), else sudo with Agent Center's password window for members of the sudo
/// group, else pkexec.
#[must_use]
pub fn elevation(interactive_tty: bool) -> Elevation {
    let sudo = in_path("sudo");
    if sudo && interactive_tty {
        return Elevation::SudoTty;
    }
    // A sudo user, or sudo without pkexec: Agent Center's own password window.
    let groups = Command::new("id").arg("-Gn").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let sudoer = groups.is_ok_and(|g| g.split_whitespace().any(|g| g == "sudo" || g == "admin" || g == "wheel"));
    if sudo && (sudoer || !in_path("pkexec")) && Path::new(ASKPASS).exists() {
        return Elevation::SudoAskpass;
    }
    if in_path("pkexec") {
        return Elevation::Pkexec;
    }
    if sudo { Elevation::SudoAskpass } else { Elevation::SudoTty }
}

fn in_path(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

/// Why a job stopped.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunError {
    /// A step names an app the catalog does not have.
    #[error("unknown app {0}")]
    UnknownApp(String),
    /// A program could not be started, or its output could not be read.
    #[error("cannot run {program}: {error}")]
    Spawn {
        /// The program.
        program: String,
        /// What went wrong.
        error: io::Error,
    },
    /// A script exited with an error.
    #[error("{0}")]
    Exit(ExitStatus),
    /// sudo or pkexec did not run the system steps: wrong password, dialog cancelled, or a
    /// user who may not become root.
    #[error("authentication failed or was cancelled")]
    Authentication,
    /// One of the system steps failed; its messages are in the output.
    #[error("a step failed (see the messages above)")]
    SystemStep,
    /// A step of this app failed.
    #[error("{app}: {error}")]
    App {
        /// The app's name.
        app: String,
        /// What went wrong.
        error: Box<RunError>,
    },
}

impl RunError {
    fn of(app: &App) -> impl FnOnce(RunError) -> RunError + '_ {
        |error| RunError::App { app: app.name.clone(), error: Box::new(error) }
    }
}

/// Runs the steps in order, sending every output line to `out`. Stops at the first failure.
///
/// System-scope scripts run as root: consecutive ones are grouped into a single
/// `agent-center run-system` call so the password is asked once.
///
/// # Errors
///
/// Returns the [`RunError`] of the first step that fails.
pub fn run(cat: &Catalog, steps: &[Step], interactive_tty: bool, out: &mut dyn FnMut(&str)) -> Result<(), RunError> {
    let app_of = |step: &Step| cat.get(&step.id).ok_or_else(|| RunError::UnknownApp(step.id.clone()));
    let mut i = 0;
    while i < steps.len() {
        let step = &steps[i];
        let app = app_of(step)?;
        if step.action == Action::Remove && !app.remove_user.trim().is_empty() {
            out(&format!("==> Preparing the removal of {}", app.name));
            let mut cmd = user_command(&app.remove_user);
            cmd.env("AC_PURGE", if step.purge { "1" } else { "0" });
            stream(cmd, out).map_err(RunError::of(app))?;
        }
        match app.scope {
            Scope::User => {
                out(&format!("==> {} {}", step.action.verb_ing(), app.name));
                let mut cmd = user_command(app.script(step.action));
                cmd.env("AC_PURGE", if step.purge { "1" } else { "0" });
                stream(cmd, out).map_err(RunError::of(app))?;
                after(app, step, out);
                i += 1;
            }
            Scope::System => {
                // Group this step with the following system steps that need nothing first.
                let mut group = vec![(step, app)];
                for next in &steps[i + 1..] {
                    let next_app = app_of(next)?;
                    if next_app.scope != Scope::System
                        || (next.action == Action::Remove && !next_app.remove_user.is_empty())
                    {
                        break;
                    }
                    group.push((next, next_app));
                }
                if is_root() {
                    for &(s, a) in &group {
                        out(&format!("==> {} {}", s.action.verb_ing(), a.name));
                        stream(system_command(a.script(s.action)), out).map_err(RunError::of(a))?;
                    }
                } else {
                    let (cmd, how) = root_command(group.iter().map(|&(s, _)| s), interactive_tty);
                    let status = stream_status(cmd, out)?;
                    if !status.success() {
                        return Err(elevation_error(how, status));
                    }
                }
                for &(s, a) in &group {
                    after(a, s, out);
                }
                i += group.len();
            }
        }
    }
    Ok(())
}

/// After a removal: deletes the app's data in the home directory when asked.
fn after(app: &App, step: &Step, out: &mut dyn FnMut(&str)) {
    if step.action != Action::Remove || !step.purge {
        return;
    }
    for path in app.purge_paths() {
        if path.symlink_metadata().is_err() {
            continue;
        }
        out(&format!("==> Deleting {}", path.display()));
        let res = if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = res {
            out(&format!("warning: cannot delete {}: {e}", path.display()));
        }
    }
}

fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/agent-center"))
}

/// Exit code of `run-system` when a script fails. sudo exits with 1 and pkexec with 126 or 127
/// when they refuse to run it, so a failed script is told apart from a failed authentication.
const SCRIPT_FAILED: i32 = 3;

/// What a failed `run-system` call means, from the exit status of sudo or pkexec.
fn elevation_error(how: Elevation, status: ExitStatus) -> RunError {
    match (how, status.code()) {
        (_, Some(SCRIPT_FAILED)) => RunError::SystemStep,
        (Elevation::Pkexec, Some(126 | 127)) | (Elevation::SudoTty | Elevation::SudoAskpass, Some(1)) => {
            RunError::Authentication
        }
        _ => RunError::Exit(status),
    }
}

/// The sudo or pkexec command that runs a group of system steps as root.
fn root_command<'a>(group: impl Iterator<Item = &'a Step>, interactive_tty: bool) -> (Command, Elevation) {
    let mut args: Vec<String> = vec![exe().to_string_lossy().into_owned(), "run-system".into()];
    args.extend(group.map(|s| format!("{}:{}", s.id, s.action.as_str())));
    let how = elevation(interactive_tty);
    let mut cmd = match how {
        Elevation::SudoTty => {
            let mut c = Command::new("sudo");
            c.arg("--");
            c
        }
        Elevation::SudoAskpass => {
            let mut c = Command::new("sudo");
            c.env("SUDO_ASKPASS", ASKPASS).args(["-A", "--"]);
            c
        }
        Elevation::Pkexec => Command::new("pkexec"),
    };
    cmd.args(args);
    (cmd, how)
}

/// Runs a command, sending its output (stdout and stderr) line by line to `out`.
///
/// # Errors
///
/// Returns [`RunError::Spawn`] when the command cannot start, and [`RunError::Exit`] when it
/// exits with an error.
pub fn stream(cmd: Command, out: &mut dyn FnMut(&str)) -> Result<(), RunError> {
    let status = stream_status(cmd, out)?;
    if status.success() { Ok(()) } else { Err(RunError::Exit(status)) }
}

/// Like [`stream`], returning the exit status.
fn stream_status(mut cmd: Command, out: &mut dyn FnMut(&str)) -> Result<ExitStatus, RunError> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let spawn = |error| RunError::Spawn { program: program.clone(), error };
    let (reader, writer) = io::pipe().map_err(spawn)?;
    let writer2 = writer.try_clone().map_err(spawn)?;
    cmd.stdin(Stdio::null()).stdout(writer).stderr(writer2);
    let mut child = cmd.spawn().map_err(spawn)?;
    // The parent's copies of the write end must be closed so the reader sees EOF.
    drop(cmd);
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                // Progress bars redraw with carriage returns: keep the last state.
                let line = line.trim_end_matches(['\n', '\r']);
                let line = line.rsplit('\r').next().unwrap_or(line);
                out(&strip_ansi(line));
            }
        }
    }
    child.wait().map_err(spawn)
}

/// Removes terminal escape sequences (ECMA-48) and control characters but tabs.
fn strip_ansi(s: &str) -> String {
    let mut res = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if c >= ' ' || c == '\t' {
                res.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI (colors, cursor moves): parameters, then a final byte from @ to ~.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC (window title, links): up to BEL or ESC \.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            // Other sequences: intermediate bytes (space to /), then one final byte, as in
            // ESC ( B, which `tput sgr0` prints.
            Some(' '..='/') => {
                while chars.next_if(|c| (' '..='/').contains(c)).is_some() {}
                chars.next();
            }
            _ => {}
        }
    }
    res
}

/// `agent-center run-system id:action...`: the root side of [`run`], started through sudo or
/// pkexec. Scripts only come from the root-owned catalog directories ([`Catalog::load_system`]).
///
/// Returns the process exit code: 0 when every step succeeded, 1 when not running as root, 2 for
/// a bad step, 3 when a script failed.
#[must_use]
pub fn run_system(args: &[String]) -> i32 {
    if !is_root() {
        eprintln!("agent-center run-system must run as root");
        return 1;
    }
    let cat = Catalog::load_system();
    for arg in args {
        let Some((id, action)) = arg.split_once(':') else {
            eprintln!("bad step {arg}");
            return 2;
        };
        let (Some(app), Some(action)) = (cat.get(id), Action::parse(action)) else {
            eprintln!("unknown app or action: {arg}");
            return 2;
        };
        if app.scope != Scope::System {
            eprintln!("{id} is not a system app");
            return 2;
        }
        println!("==> {} {}", action.verb_ing(), app.name);
        let status = system_command(app.script(action)).stdin(Stdio::null()).status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("error: {} failed ({s})", app.name);
                return SCRIPT_FAILED;
            }
            Err(e) => {
                eprintln!("error: {}: {e}", app.name);
                return SCRIPT_FAILED;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escape_codes() {
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m done"), "ok done");
        assert_eq!(strip_ansi("\x1b(Bplain\x1b[m text\tend\x07"), "plain text\tend", "tput sgr0");
        assert_eq!(strip_ansi("\x1b]0;title\x07a\x1b]8;;https://x\x1b\\link\x1b]8;;\x1b\\"), "alink");
        assert_eq!(strip_ansi("\x1b[38;2;10;20;30mrgb\x1b[0K"), "rgb");
        assert_eq!(strip_ansi("cut\x1b"), "cut");
    }

    #[test]
    fn catalog_scripts_parse() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog");
        // SAFETY: tests in this module do not read the variable concurrently.
        unsafe { std::env::set_var("AGENT_CENTER_CATALOG", &dir) };
        let cat = Catalog::load();
        for app in &cat.apps {
            let latest = app.latest.strip_prefix("cmd:").unwrap_or("");
            for script in [&app.install, &app.update, &app.remove, &app.remove_user, &app.version, latest] {
                let out = Command::new("sh").arg("-n").arg("-c").arg(format!("{PRELUDE}\n{script}")).output().unwrap();
                assert!(out.status.success(), "{}: {}", app.id, String::from_utf8_lossy(&out.stderr));
            }
        }
    }

    #[test]
    fn failed_authentication_is_told_apart_from_a_failed_script() {
        use std::os::unix::process::ExitStatusExt;
        let exit = |code: i32| ExitStatus::from_raw(code << 8);
        let error = |how, code| elevation_error(how, exit(code)).to_string();
        let auth = "authentication failed or was cancelled";
        assert_eq!(error(Elevation::Pkexec, 126), auth);
        assert_eq!(error(Elevation::Pkexec, 127), auth);
        assert_eq!(error(Elevation::SudoAskpass, 1), auth);
        assert_eq!(error(Elevation::SudoTty, 1), auth);
        assert_eq!(error(Elevation::SudoTty, SCRIPT_FAILED), "a step failed (see the messages above)");
        assert_eq!(error(Elevation::Pkexec, 1), "exit status: 1");
    }

    #[test]
    fn requirement_cycles_end() {
        // Two apps that require each other.
        let cycle = Catalog::parse(
            "[[app]]\nid = \"x\"\nname = \"X\"\ncategory = \"c\"\nrequires = [\"y\"]\nversion = \"\"\n\
             install = \"\"\nremove = \"\"\n[[app]]\nid = \"y\"\nname = \"Y\"\ncategory = \"c\"\n\
             requires = [\"x\"]\nversion = \"\"\ninstall = \"\"\nremove = \"\"\n",
        )
        .unwrap();
        let install = |id: &str| Step { id: id.into(), action: Action::Install, purge: false };
        let steps = with_requirements(&cycle, vec![install("x")], &|_| false);
        assert_eq!(steps, [install("y"), install("x")]);
    }

    #[test]
    fn root_scripts_get_a_fixed_path() {
        let out = system_command("printf '%s|%s' \"$HOME\" \"$PATH\"").output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let (home, path) = text.split_once('|').unwrap();
        assert_eq!(home, "/root");
        // The prelude only prepends root's own ~/.local/bin to the fixed PATH.
        assert_eq!(
            path,
            format!("/root/.local/bin:/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin:{ROOT_PATH}")
        );
    }

    #[test]
    fn prelude_parses() {
        let status = user_command("log hello >/dev/null; deb_version no-such-package-xyz").status().unwrap();
        assert!(status.success());
    }
}
