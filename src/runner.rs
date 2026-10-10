//! Runs catalog scripts: as the user, or as root through sudo or pkexec.

use crate::catalog::{Action, App, Catalog, Scope};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
pub fn user_command(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(format!("{PRELUDE}\n{script}"));
    cmd
}

/// One step of a job.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub id: String,
    pub action: Action,
    /// After a removal, also delete the app's settings and data in the home directory.
    pub purge: bool,
}

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
    Pkexec,
}

pub const ASKPASS: &str = "/usr/lib/agent-center/askpass";

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

/// Runs the steps in order, sending every output line to `out`. Stops at the first failure.
///
/// System-scope scripts run as root: consecutive ones are grouped into a single
/// `agent-center run-system` call so the password is asked once.
pub fn run(cat: &Catalog, steps: &[Step], interactive_tty: bool, out: &mut dyn FnMut(&str)) -> Result<(), String> {
    let mut i = 0;
    while i < steps.len() {
        let step = &steps[i];
        let app = cat.get(&step.id).ok_or_else(|| format!("unknown app {}", step.id))?;
        if step.action == Action::Remove && !app.remove_user.trim().is_empty() {
            out(&format!("==> Preparing the removal of {}", app.name));
            let mut cmd = user_command(&app.remove_user);
            cmd.env("AC_PURGE", if step.purge { "1" } else { "0" });
            stream(cmd, out).map_err(|e| format!("{}: {e}", app.name))?;
        }
        match app.scope {
            Scope::User => {
                out(&format!("==> {} {}", step.action.verb_ing(), app.name));
                let mut cmd = user_command(app.script(step.action));
                cmd.env("AC_PURGE", if step.purge { "1" } else { "0" });
                stream(cmd, out).map_err(|e| format!("{}: {e}", app.name))?;
                after(app, step, out);
                i += 1;
            }
            Scope::System => {
                // Group this step with the following system steps that need nothing first.
                let mut j = i + 1;
                while j < steps.len() {
                    let next = cat.get(&steps[j].id).ok_or_else(|| format!("unknown app {}", steps[j].id))?;
                    if next.scope != Scope::System
                        || (steps[j].action == Action::Remove && !next.remove_user.is_empty())
                    {
                        break;
                    }
                    j += 1;
                }
                let group = &steps[i..j];
                let result = if is_root() {
                    group.iter().try_for_each(|s| {
                        let a = cat.get(&s.id).unwrap();
                        out(&format!("==> {} {}", s.action.verb_ing(), a.name));
                        stream(user_command(a.script(s.action)), out).map_err(|e| format!("{}: {e}", a.name))
                    })
                } else {
                    stream(root_command(group, interactive_tty), out).map_err(|e| match e.as_str() {
                        "exit status 126" | "exit status 127" => "authentication failed or was cancelled".to_string(),
                        _ => e,
                    })
                };
                result?;
                for s in group {
                    after(cat.get(&s.id).unwrap(), s, out);
                }
                i = j;
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

fn root_command(group: &[Step], interactive_tty: bool) -> Command {
    let mut args: Vec<String> = vec![exe().to_string_lossy().into_owned(), "run-system".into()];
    args.extend(group.iter().map(|s| format!("{}:{}", s.id, s.action.as_str())));
    let mut cmd = match elevation(interactive_tty) {
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
    cmd
}

/// Runs a command, sending its output (stdout and stderr) line by line to `out`.
pub fn stream(mut cmd: Command, out: &mut dyn FnMut(&str)) -> Result<(), String> {
    let (reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
    let writer2 = writer.try_clone().map_err(|e| e.to_string())?;
    cmd.stdin(Stdio::null()).stdout(writer).stderr(writer2);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
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
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(status.to_string()) }
}

fn strip_ansi(s: &str) -> String {
    let mut res = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else if c >= ' ' || c == '\t' {
            res.push(c);
        }
    }
    res
}

/// `agent-center run-system id:action...`: the root side of `run`, started through sudo or pkexec.
/// Scripts only come from the root-owned catalog directories.
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
        let status = user_command(app.script(action)).stdin(Stdio::null()).status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("error: {} failed ({s})", app.name);
                return 1;
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
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
    fn prelude_parses() {
        let status = user_command("log hello >/dev/null; deb_version no-such-package-xyz").status().unwrap();
        assert!(status.success());
    }
}
