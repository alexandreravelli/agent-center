//! Installed and latest versions.

use crate::catalog::App;
use crate::runner;
use std::cmp::Ordering;
use std::io::Write;
use std::process::{Command, Stdio};

/// The installed version, or `None` when the app is not installed.
pub fn installed(app: &App) -> Option<String> {
    let out = runner::user_command(&app.version).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // Something printed but no number: installed, version unknown.
    Some(extract(text).unwrap_or_else(|| "?".into()))
}

/// The latest available version, from the app's `latest` source.
pub fn latest(app: &App) -> Result<String, String> {
    let src = app.latest.trim();
    let (kind, arg) = src.split_once(':').ok_or("no source")?;
    let arg = arg.trim();
    let text = match kind {
        "npm" => json_field(&format!("https://registry.npmjs.org/{arg}/latest"), &["version"])?,
        "github" => json_field(&format!("https://api.github.com/repos/{arg}/releases/latest"), &["tag_name"])?,
        "pypi" => json_field(&format!("https://pypi.org/pypi/{arg}/json"), &["info", "version"])?,
        "apt" => apt_candidate(arg)?,
        "cmd" => {
            let out = runner::user_command(arg).stdin(Stdio::null()).stderr(Stdio::null()).output();
            let out = out.map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err("version command failed".into());
            }
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
        _ => return Err(format!("unknown source {kind}")),
    };
    extract(&text).ok_or_else(|| format!("no version in {:?}", text.trim()))
}

/// The GitHub token from the environment, if any. Optional: it raises GitHub's limit of 60
/// requests per hour.
fn github_token() -> Option<String> {
    ["GITHUB_TOKEN", "GH_TOKEN"].iter().find_map(|k| std::env::var(k).ok().filter(|t| !t.is_empty()))
}

/// The curl command for `url`, and the header lines to write on its standard input. Secrets go
/// through stdin: every user on the machine can read a command line (`ps`), not a pipe.
fn curl(url: &str, token: Option<&str>) -> (Command, Option<String>) {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "20", "-H", "User-Agent: agent-center"]);
    let mut stdin = None;
    if url.starts_with("https://api.github.com/") {
        cmd.args(["-H", "Accept: application/vnd.github+json"]);
        if let Some(token) = token {
            cmd.args(["-H", "@-"]);
            stdin = Some(format!("Authorization: Bearer {token}\n"));
        }
    }
    cmd.arg(url);
    (cmd, stdin)
}

fn fetch(url: &str) -> Result<String, String> {
    let (mut cmd, headers) = curl(url, github_token().as_deref());
    cmd.stdin(if headers.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("curl: {e}"))?;
    if let (Some(headers), Some(mut stdin)) = (headers, child.stdin.take()) {
        // Dropping `stdin` closes the pipe, so curl stops reading headers.
        stdin.write_all(headers.as_bytes()).map_err(|e| format!("curl: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.trim().trim_start_matches("curl: ").to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn json_field(url: &str, path: &[&str]) -> Result<String, String> {
    let body = fetch(url)?;
    let mut v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    for key in path {
        v = v.get(key).cloned().ok_or_else(|| format!("no \"{key}\" in the answer"))?;
    }
    v.as_str().map(String::from).ok_or_else(|| "not a string".into())
}

/// The version APT would install, from the package lists of the last `apt-get update`.
fn apt_candidate(pkg: &str) -> Result<String, String> {
    let out = Command::new("apt-cache")
        .args(["policy", "--", pkg])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let cand = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Candidate:"))
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "(none)")
        .ok_or("not in the APT package lists")?;
    Ok(cand.to_string())
}

/// The first version-looking token: digits and dots with at least one dot ("2.1.4", "144.0"),
/// skipping a Debian epoch ("1:144.0-1" gives "144.0-1"), keeping a pre-release suffix.
pub fn extract(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let starts =
            bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'v');
        if starts {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            // Epoch: "1:144.0".
            if j < bytes.len() && bytes[j] == b':' && text[i..j].chars().all(|c| c.is_ascii_digit()) {
                i = j + 1;
                continue;
            }
            let core = text[i..j].trim_end_matches('.');
            if core.contains('.') {
                let mut end = i + core.len();
                if end == j && j < bytes.len() && (bytes[j] == b'-' || bytes[j] == b'+' || bytes[j] == b'~') {
                    end = j + 1;
                    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || b".-+~".contains(&bytes[end])) {
                        end += 1;
                    }
                }
                return Some(text[i..end].to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

/// Compares the numeric parts ("1.10.0" > "1.9.2"); a pre-release ("2.0.0-beta.1") is older
/// than its release, and a Debian revision ("144.0-1") is ignored.
pub fn compare(a: &str, b: &str) -> Ordering {
    let parse = |v: &str| {
        let (core, rest) = match v.find(['-', '+', '~']) {
            Some(i) => (&v[..i], &v[i + 1..]),
            None => (v, ""),
        };
        let nums: Vec<u64> = core.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        // "-beta.1", "~rc2" are pre-releases; "-1" (Debian revision), "~build1" are not.
        let r = rest.to_ascii_lowercase();
        let pre = ["alpha", "beta", "rc", "pre", "preview", "dev", "nightly", "canary", "next"]
            .iter()
            .any(|p| r.starts_with(p));
        (nums, pre)
    };
    let (na, pa) = parse(a);
    let (nb, pb) = parse(b);
    let len = na.len().max(nb.len());
    for i in 0..len {
        let (x, y) = (na.get(i).copied().unwrap_or(0), nb.get(i).copied().unwrap_or(0));
        if x != y {
            return x.cmp(&y);
        }
    }
    pb.cmp(&pa)
}

/// True when `latest` is newer than `installed`.
pub fn is_newer(installed: &str, latest: &str) -> bool {
    installed != "?" && compare(latest, installed) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_versions() {
        assert_eq!(extract("2.1.4 (Claude Code)").as_deref(), Some("2.1.4"));
        assert_eq!(extract("codex-cli 0.46.0").as_deref(), Some("0.46.0"));
        assert_eq!(extract("rust-v0.46.0").as_deref(), Some("0.46.0"));
        assert_eq!(extract("v24.10.0\n").as_deref(), Some("24.10.0"));
        assert_eq!(extract("1:144.0.2-1").as_deref(), Some("144.0.2-1"));
        assert_eq!(extract("Zed 0.207.3 abc").as_deref(), Some("0.207.3"));
        assert_eq!(extract("aider 0.86.1").as_deref(), Some("0.86.1"));
        assert_eq!(extract("1.2.0-beta.3 x").as_deref(), Some("1.2.0-beta.3"));
        assert_eq!(extract("x86_64 build 12").as_deref(), None);
        assert_eq!(extract("installed").as_deref(), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("1.9.2", "1.10.0"));
        assert!(!is_newer("1.10.0", "1.10.0"));
        assert!(!is_newer("2.0.0", "2.0.0-beta.1"));
        assert!(is_newer("2.0.0-beta.1", "2.0.0"));
        assert!(!is_newer("144.0-1", "144.0"));
        assert!(is_newer("144.0-1", "144.0.1-1"));
        assert!(!is_newer("144.0.2~build1", "144.0.2"));
        assert!(is_newer("2.1.0-rc.1", "2.1.0"));
        assert!(!is_newer("?", "1.0"));
    }

    #[test]
    fn github_token_stays_off_the_command_line() {
        let url = "https://api.github.com/repos/o/r/releases/latest";
        let (cmd, stdin) = curl(url, Some("s3cret"));
        assert!(cmd.get_args().all(|a| !a.to_string_lossy().contains("s3cret")));
        assert_eq!(stdin.as_deref(), Some("Authorization: Bearer s3cret\n"));
        // Other hosts never get the token.
        let (_, stdin) = curl("https://registry.npmjs.org/x/latest", Some("s3cret"));
        assert_eq!(stdin, None);
    }
}
