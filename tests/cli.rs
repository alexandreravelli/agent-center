//! The command line, run against a sample catalog of apps that live in a temporary directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    /// A catalog with two user apps: `hello` (which requires `base`) and `base`. Their "install"
    /// writes a version into a state file, their "remove" deletes it.
    fn new() -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let (catalog, state) = (dir.path().join("catalog"), dir.path().join("state"));
        std::fs::create_dir_all(&catalog).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        let state = state.display();
        let text = format!(
            r#"
[[category]]
id = "tools"
name = "Tools"

[[app]]
id = "hello"
name = "Hello"
category = "tools"
scope = "user"
summary = "Says hello."
requires = ["base"]
purge = ["~/.hello"]
version = "cat '{state}/hello' 2>/dev/null"
latest = "cmd:echo 2.0.0"
install = "echo 1.0.0 > '{state}/hello'"
update = "echo 2.0.0 > '{state}/hello'"
remove = "rm '{state}/hello'"

[[app]]
id = "base"
name = "Base"
category = "tools"
scope = "user"
version = "cat '{state}/base' 2>/dev/null"
install = "echo 0.1.0 > '{state}/base'"
remove = "rm '{state}/base'"
"#
        );
        std::fs::write(catalog.join("10-tools.toml"), text).unwrap();
        Sandbox { dir }
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn state(&self, id: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join("state").join(id)).ok().map(|s| s.trim().to_string())
    }

    fn run(&self, args: &str) -> Run {
        let out = Command::new(env!("CARGO_BIN_EXE_agent-center"))
            .args(args.split_whitespace())
            .env("AGENT_CENTER_CATALOG", self.dir.path().join("catalog"))
            .env("HOME", self.home())
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_TOKEN")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        Run(out)
    }
}

struct Run(Output);

impl Run {
    fn code(&self) -> Option<i32> {
        self.0.status.code()
    }

    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.0.stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.0.stderr).into_owned()
    }

    /// Asserts a success and returns stdout.
    fn ok(&self) -> String {
        assert_eq!(self.code(), Some(0), "stdout:\n{}\nstderr:\n{}", self.stdout(), self.stderr());
        self.stdout()
    }
}

#[test]
fn install_update_and_remove_an_app() {
    let sb = Sandbox::new();
    let list = sb.run("list").ok();
    assert!(list.starts_with("Tools\n"), "{list}");
    assert!(list.lines().any(|l| l.contains("hello") && l.contains(" - ")), "{list}");

    // The requirement is installed first.
    let out = sb.run("install hello").ok();
    let installing: Vec<&str> = out.lines().filter(|l| l.starts_with("==> ")).collect();
    assert_eq!(installing, ["==> Installing Base", "==> Installing Hello"]);
    assert!(out.ends_with("Done.\n"));
    assert_eq!((sb.state("base").as_deref(), sb.state("hello").as_deref()), (Some("0.1.0"), Some("1.0.0")));
    assert_eq!(sb.run("install hello").ok(), "Hello is already installed.\n");

    let installed = sb.run("list --installed").ok();
    assert!(installed.contains("1.0.0") && installed.contains("0.1.0"), "{installed}");
    let updates = sb.run("list --updates").ok();
    assert!(updates.contains("1.0.0 -> 2.0.0") && !updates.contains("base"), "{updates}");

    // Without names, `update` updates every app with a newer version.
    let out = sb.run("update").ok();
    assert!(out.contains("hello: 1.0.0 -> 2.0.0"), "{out}");
    assert_eq!(sb.state("hello").as_deref(), Some("2.0.0"));
    assert_eq!(sb.run("update").ok(), "Checking for updates...\nEverything is up to date.\n");

    // --purge also deletes the app's data in the home directory.
    std::fs::create_dir_all(sb.home().join(".hello/cache")).unwrap();
    let out = sb.run("remove --purge hello").ok();
    assert!(out.contains("==> Deleting"), "{out}");
    assert!(!sb.home().join(".hello").exists());
    assert_eq!(sb.state("hello"), None);
    assert_eq!(sb.state("base").as_deref(), Some("0.1.0"), "requirements stay");
    assert_eq!(sb.run("remove hello").ok(), "Hello is not installed.\n");
}

#[test]
fn dry_run_shows_the_scripts_and_runs_nothing() {
    let sb = Sandbox::new();
    let out = sb.run("install --dry-run hello").ok();
    assert!(out.starts_with("Dry run: nothing is changed."), "{out}");
    let steps: Vec<&str> = out.lines().filter(|l| l.starts_with("==> ")).collect();
    assert_eq!(steps, ["==> Installing Base (as you)", "==> Installing Hello (as you)"]);
    assert!(out.contains("\n    echo 1.0.0 > "), "{out}");
    assert_eq!((sb.state("base"), sb.state("hello")), (None, None));

    sb.run("install hello").ok();
    std::fs::create_dir_all(sb.home().join(".hello")).unwrap();
    let out = sb.run("remove hello --purge --dry-run").ok();
    assert!(out.contains("==> Removing Hello (as you)\n    rm "), "{out}");
    assert!(out.contains(&format!("==> Deleting {}", sb.home().join(".hello").display())), "{out}");
    assert!(sb.home().join(".hello").exists() && sb.state("hello").is_some(), "nothing removed");
}

#[test]
fn json_output() {
    let sb = Sandbox::new();
    sb.run("install base").ok();
    let list: serde_json::Value = serde_json::from_str(&sb.run("list --json").ok()).unwrap();
    assert_eq!(list.as_array().map(Vec::len), Some(2));
    assert_eq!(list[0]["id"], "hello");
    assert_eq!(list[0]["installed"], serde_json::Value::Null);
    assert_eq!(list[1]["installed"], "0.1.0");
    assert_eq!(list[1]["scope"], "user");

    sb.run("install hello").ok();
    let updates: serde_json::Value = serde_json::from_str(&sb.run("list --updates --json").ok()).unwrap();
    assert_eq!(updates.as_array().map(Vec::len), Some(1));
    assert_eq!((&updates[0]["latest"], &updates[0]["update"]), (&"2.0.0".into(), &true.into()));

    let info: serde_json::Value = serde_json::from_str(&sb.run("info hello --json").ok()).unwrap();
    assert_eq!(info["installed"], "1.0.0");
    assert_eq!(info["latest"], "2.0.0");
    assert_eq!(info["latest_error"], serde_json::Value::Null);
    assert_eq!(info["requires"], serde_json::json!(["base"]));
    assert!(info["scripts"]["update"].as_str().is_some_and(|s| s.starts_with("echo 2.0.0")));
}

/// Runs `script` in `shell` with the sandbox's agent-center first in PATH; None when the shell
/// is not installed.
fn in_shell(sb: &Sandbox, shell: &str, script: &str) -> Option<String> {
    let bin = Path::new(env!("CARGO_BIN_EXE_agent-center")).parent().unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let out = Command::new(shell)
        .args(["-c", script])
        .env("PATH", path)
        .env("AGENT_CENTER_CATALOG", sb.dir.path().join("catalog"))
        .output()
        .ok()?;
    assert!(out.status.success(), "{shell}: {}", String::from_utf8_lossy(&out.stderr));
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn bash_completes_commands_options_and_apps() {
    let sb = Sandbox::new();
    assert_eq!(sb.run("list --ids").ok(), "hello\nbase\n");
    let complete = |line: &str| {
        let script = format!(
            "source <(agent-center completions bash); COMP_WORDS=({line}); COMP_CWORD=$((${{#COMP_WORDS[@]}} - 1)); \
             _agent_center; printf '%s\\n' \"${{COMPREPLY[@]}}\""
        );
        in_shell(&sb, "bash", &script)
    };
    assert_eq!(complete("agent-center ins").as_deref(), Some("install\n"));
    assert_eq!(complete("agent-center install he").as_deref(), Some("hello\n"));
    assert_eq!(complete("agent-center remove --").as_deref(), Some("--purge\n--dry-run\n"));
    assert_eq!(complete("agent-center completions f").as_deref(), Some("fish\n"));
}

#[test]
fn zsh_and_fish_scripts_parse() {
    let sb = Sandbox::new();
    // Skipped where the shell is not installed.
    let zsh = in_shell(&sb, "zsh", "agent-center completions zsh | zsh -n");
    let fish = in_shell(&sb, "fish", "agent-center completions fish | source; complete -C 'agent-center install he'");
    assert!(zsh.is_none_or(|out| out.is_empty()));
    assert!(fish.as_ref().is_none_or(|out| out.starts_with("hello")), "{fish:?}");
}

#[test]
fn a_failing_script_stops_the_job() {
    let sb = Sandbox::new();
    std::fs::remove_dir_all(sb.dir.path().join("state")).unwrap();
    let run = sb.run("install base");
    assert_eq!(run.code(), Some(1));
    assert!(run.stderr().starts_with("agent-center: Base: exit status: "), "{}", run.stderr());
    assert!(!run.stdout().contains("Done."));
}

#[test]
fn info_shows_the_scripts() {
    let sb = Sandbox::new();
    let out = sb.run("info hello").ok();
    assert!(out.starts_with("Hello (hello)\nSays hello.\n"), "{out}");
    assert!(out.contains("(in your home directory)"), "{out}");
    assert!(out.contains("Requires:  base"), "{out}");
    assert!(out.contains("# Install\necho 1.0.0 > "), "{out}");
    assert!(out.contains("# Update\necho 2.0.0 > "), "{out}");
    assert_eq!(sb.run("categories").ok(), "tools        Tools\n");
}

#[test]
fn command_line_mistakes_exit_with_2() {
    let sb = Sandbox::new();
    for (args, message) in [
        ("frobnicate", "agent-center: unknown command frobnicate. See `agent-center --help`."),
        ("isntall hello", "agent-center: unknown command isntall. Did you mean install?"),
        ("info", "agent-center: name an app."),
        ("info nope", "agent-center: unknown app nope. See `agent-center list`."),
        ("info hello base", "agent-center: info shows one app at a time."),
        ("install hello helo", "agent-center: unknown app helo. Did you mean hello?"),
        ("install hello --purge", "agent-center: unknown option --purge for install."),
        ("list --instaled", "agent-center: unknown option --instaled for list. Did you mean --installed?"),
        ("remove", "agent-center: name at least one app."),
    ] {
        let run = sb.run(args);
        assert_eq!(run.code(), Some(2), "{args}");
        assert!(run.stderr().starts_with(message), "{args}: {}", run.stderr());
        assert!(run.stdout().is_empty(), "{args}");
    }
    assert_eq!(sb.state("hello"), None, "nothing ran");
}

#[test]
fn version_and_help() {
    let sb = Sandbox::new();
    assert_eq!(sb.run("--version").ok(), format!("agent-center {}\n", env!("CARGO_PKG_VERSION")));
    assert!(sb.run("--help").ok().starts_with("Usage: agent-center"));
}

#[test]
fn run_system_refuses_to_run_without_root() {
    if agent_center::runner::is_root() {
        return;
    }
    let run = Sandbox::new().run("run-system hello:install");
    assert_eq!(run.code(), Some(1));
    assert_eq!(run.stderr(), "agent-center run-system must run as root\n");
}

#[test]
fn shipped_catalog_lists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new(env!("CARGO_BIN_EXE_agent-center"))
        .arg("categories")
        .env("AGENT_CENTER_CATALOG", root.join("catalog"))
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("AI Agents"));
}
