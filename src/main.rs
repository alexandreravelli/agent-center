//! Agent Center: an app center for AI coding tools, browsers and editors on Debian.
//!
//! Without arguments it opens the window; with a command it works in the terminal.

mod completions;
#[cfg(feature = "gui")]
mod gui;
#[cfg(feature = "gui")]
mod notifier;
mod style;

use agent_center::catalog::{Action, App, Catalog, Scope};
use agent_center::runner::{self, Step, with_requirements};
use agent_center::versions;
use anyhow::Result;
use std::fmt;
use std::io::IsTerminal;
use std::process::ExitCode;
use std::thread;
use style::Style;

const USAGE: &str = "\
Usage: agent-center [--updates | COMMAND]

Without a command, opens the Agent Center window (on the Updates page with --updates).

Commands:
  list [--installed] [--updates]    List the apps (with the latest versions for --updates);
                                    --ids prints the app ids only
  info <app>                        Show an app and the scripts Agent Center runs for it
                                    (list and info take --json: print JSON for scripts)
  install <app>...                  Install apps (and what they need)
  update [<app>...]                 Update apps; without names, every app with an update
  remove [--purge] <app>...         Remove apps; --purge also deletes their settings and data
                                    (install, update and remove take --dry-run: show the
                                    scripts they would run, and run nothing)
  categories                        List the categories
  completions <bash|zsh|fish>       Print the shell completion script (the package installs it)
  notify [--now]                    Check for updates once a day and show a desktop
                                    notification (started at login; --now checks at once)

Options:
  -h, --help       Show this help
  -V, --version    Show the version";

/// A mistake on the command line, reported with exit code 2.
#[derive(Debug)]
struct Usage(String);

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Usage {}

/// The commands, for suggestions.
const COMMANDS: [&str; 11] = [
    "list",
    "info",
    "install",
    "update",
    "remove",
    "categories",
    "notify",
    "completions",
    "help",
    "upgrade",
    "uninstall",
];

/// The arguments of a command: app names, and the options it accepts.
struct Args<'a> {
    names: Vec<&'a str>,
    options: Vec<&'a str>,
}

impl<'a> Args<'a> {
    /// Splits `args` into names and options, refusing options `command` does not have.
    fn parse(command: &str, args: &'a [String], accepted: &[&str]) -> Result<Args<'a>, Usage> {
        let (mut names, mut options) = (Vec::new(), Vec::new());
        for arg in args {
            if !arg.starts_with('-') {
                names.push(arg.as_str());
            } else if accepted.contains(&arg.as_str()) {
                options.push(arg.as_str());
            } else {
                let hint = did_you_mean(&similar(arg, accepted.iter().copied()));
                return Err(Usage(format!("unknown option {arg} for {command}.{hint}")));
            }
        }
        Ok(Args { names, options })
    }

    fn has(&self, option: &str) -> bool {
        self.options.contains(&option)
    }
}

/// Up to three `candidates` close to `word`, closest first: a few typos away, or starting with it
/// (`claude` gives `claude-code`).
fn similar<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let word = word.to_lowercase();
    let typos = (word.chars().count() / 4).max(1);
    let mut found: Vec<(usize, &str)> = candidates
        .into_iter()
        .filter_map(|c| {
            let d = distance(&word, &c.to_lowercase());
            let prefix = word.chars().count() >= 3 && c.to_lowercase().starts_with(&word);
            (d <= typos || prefix).then_some((d, c))
        })
        .collect();
    found.sort_unstable();
    found.dedup_by_key(|(_, c)| *c);
    found.into_iter().take(3).map(|(_, c)| c).collect()
}

/// Edits (insertion, deletion, substitution, swap of two neighbours) from `a` to `b`.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut rows = vec![(0..=b.len()).collect::<Vec<usize>>()];
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row[j] = (rows[i - 1][j] + 1).min(row[j - 1] + 1).min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(rows[i - 2][j - 2] + 1);
            }
        }
        rows.push(row);
    }
    rows[a.len()][b.len()]
}

/// " Did you mean x, y or z?", or nothing.
fn did_you_mean(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => format!(" Did you mean {one}?"),
        [first @ .., last] => format!(" Did you mean {} or {last}?", first.join(", ")),
    }
}

fn main() -> ExitCode {
    // Exit quietly when the reader goes away (`agent-center list | head`) instead of panicking.
    // SAFETY: restoring the default signal action before any other thread exists.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {e:#}", Style::stderr().red("agent-center:"));
            if e.is::<Usage>() { ExitCode::from(2) } else { ExitCode::FAILURE }
        }
    }
}

fn dispatch(args: &[String]) -> Result<ExitCode> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        None | Some("--updates") => Ok(open_window(args)),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        Some("-V" | "--version") => {
            println!("agent-center {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Some("askpass") => Ok(askpass(rest)),
        Some("notify") => {
            Args::parse("notify", rest, &["--now"])?;
            notify(rest)
        }
        Some("run-system") => Ok(exit_code(runner::run_system(rest))),
        Some("list") => list(&Args::parse("list", rest, &["--installed", "--updates", "--json", "--ids"])?),
        Some("completions") => {
            let args = Args::parse("completions", rest, &[])?;
            let shell = args.names.first().copied().unwrap_or_default();
            let script = completions::script(shell).ok_or_else(|| {
                let hint = did_you_mean(&similar(shell, completions::SHELLS));
                Usage(format!("name a shell: bash, zsh or fish.{hint}"))
            })?;
            print!("{script}");
            Ok(ExitCode::SUCCESS)
        }
        Some("categories") => {
            Args::parse("categories", rest, &[])?;
            let cat = load();
            for c in &cat.categories {
                println!("{:<12} {}", c.id, c.name);
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("info") => info(rest),
        Some("install") => act(Action::Install, rest),
        Some("update" | "upgrade") => act(Action::Update, rest),
        Some("remove" | "uninstall") => act(Action::Remove, rest),
        Some(other) => {
            let hint = did_you_mean(&similar(other, COMMANDS));
            Err(Usage(format!("unknown command {other}.{hint} See `agent-center --help`.")).into())
        }
    }
}

/// The exit code of a process status (0 to 255), else a plain failure.
fn exit_code(code: i32) -> ExitCode {
    u8::try_from(code).map_or(ExitCode::FAILURE, ExitCode::from)
}

#[cfg(feature = "gui")]
fn open_window(args: &[String]) -> ExitCode {
    exit_code(gui::run(args))
}

#[cfg(not(feature = "gui"))]
fn open_window(_: &[String]) -> ExitCode {
    println!("{USAGE}");
    ExitCode::SUCCESS
}

#[cfg(feature = "gui")]
#[expect(clippy::unnecessary_wraps, reason = "same signature as without the gui feature, which fails")]
fn notify(args: &[String]) -> Result<ExitCode> {
    Ok(exit_code(notifier::run(args)))
}

#[cfg(not(feature = "gui"))]
fn notify(_: &[String]) -> Result<ExitCode> {
    anyhow::bail!("notifications need the window (built without the gui feature)")
}

#[cfg(feature = "gui")]
fn askpass(args: &[String]) -> ExitCode {
    exit_code(gui::askpass(args.first().map_or("Password:", String::as_str)))
}

#[cfg(not(feature = "gui"))]
fn askpass(_: &[String]) -> ExitCode {
    ExitCode::FAILURE
}

fn load() -> Catalog {
    let cat = Catalog::load();
    for w in &cat.warnings {
        eprintln!("{} {w}", Style::stderr().yellow("warning:"));
    }
    cat
}

/// Runs `f` on every app in parallel (version checks spawn processes and fetch URLs).
fn par_map<T: Send>(apps: &[&App], f: impl Fn(&App) -> T + Sync) -> Vec<T> {
    let f = &f;
    thread::scope(|s| {
        let handles: Vec<_> = apps
            .chunks(apps.len().div_ceil(8).max(1))
            .map(|chunk| s.spawn(move || chunk.iter().map(|a| f(a)).collect::<Vec<T>>()))
            .collect();
        // A panic in a worker is a bug: pass it on.
        handles.into_iter().flat_map(|h| h.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    })
}

fn list(args: &Args) -> Result<ExitCode> {
    let cat = load();
    // For shell completion: fast, no version check.
    if args.has("--ids") {
        for app in &cat.apps {
            println!("{}", app.id);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let installed_only = args.has("--installed");
    let updates = args.has("--updates");
    let apps: Vec<&App> = cat.apps.iter().collect();
    let installed = par_map(&apps, versions::installed);
    let latest: Vec<Option<String>> = if updates {
        let mut v = par_map(&apps, |a| if a.latest.is_empty() { None } else { versions::latest(a).ok() });
        // Only installed apps matter here.
        for (l, i) in v.iter_mut().zip(&installed) {
            if i.is_none() {
                *l = None;
            }
        }
        v
    } else {
        vec![None; apps.len()]
    };
    let update = |i: usize| matches!((&installed[i], &latest[i]), (Some(a), Some(b)) if versions::is_newer(a, b));
    let rows = |category: &str| -> Vec<usize> {
        (0..apps.len())
            .filter(|&i| apps[i].category == category)
            .filter(|&i| !installed_only || installed[i].is_some())
            .filter(|&i| !updates || update(i))
            .collect()
    };
    if args.has("--json") {
        let list: Vec<serde_json::Value> = cat
            .categories
            .iter()
            .flat_map(|c| rows(&c.id))
            .map(|i| {
                let a = apps[i];
                serde_json::json!({
                    "id": a.id,
                    "name": a.name,
                    "category": a.category,
                    "summary": a.summary,
                    "scope": scope_name(a.scope),
                    "installed": installed[i],
                    "latest": latest[i],
                    "update": update(i),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(ExitCode::SUCCESS);
    }
    let style = Style::stdout();
    for c in &cat.categories {
        let rows = rows(&c.id);
        if rows.is_empty() {
            continue;
        }
        println!("{}", style.bold(&c.name));
        for i in rows {
            let a = apps[i];
            // Padded before it is painted: escape codes have no width.
            let state = match (&installed[i], &latest[i]) {
                (Some(v), Some(l)) if versions::is_newer(v, l) => {
                    style.yellow(&format!("{:<20}", format!("{v} -> {l}")))
                }
                (Some(v), _) => style.green(&format!("{v:<20}")),
                (None, _) => style.dim(&format!("{:<20}", "-")),
            };
            println!("  {:<18} {:<22} {state} {}", a.id, a.name, a.summary);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The value of `scope` in the catalog.
fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::System => "system",
        Scope::User => "user",
    }
}

/// The catalog app with this id, or a usage error that suggests close ids.
fn find<'a>(cat: &'a Catalog, id: &str) -> Result<&'a App, Usage> {
    cat.get(id).ok_or_else(|| {
        let hint = did_you_mean(&similar(id, cat.apps.iter().map(|a| a.id.as_str())));
        Usage(format!("unknown app {id}.{hint} See `agent-center list`."))
    })
}

fn info(args: &[String]) -> Result<ExitCode> {
    let args = Args::parse("info", args, &["--json"])?;
    let cat = load();
    let app = match args.names[..] {
        [id] => find(&cat, id)?,
        [] => return Err(Usage("name an app. See `agent-center list`.".into()).into()),
        _ => return Err(Usage("info shows one app at a time.".into()).into()),
    };
    if args.has("--json") {
        let latest = (!app.latest.is_empty()).then(|| versions::latest(app));
        let info = serde_json::json!({
            "id": app.id,
            "name": app.name,
            "summary": app.summary,
            "category": app.category,
            "category_name": cat.category_name(&app.category),
            "homepage": app.homepage,
            "method": app.method,
            "scope": scope_name(app.scope),
            "installed": versions::installed(app),
            "latest": latest.as_ref().and_then(|l| l.as_ref().ok()),
            "latest_error": latest.as_ref().and_then(|l| l.as_ref().err()).map(ToString::to_string),
            "requires": app.requires,
            "purge": app.purge,
            "command": app.command,
            "desktop": app.desktop,
            "notes": app.notes.trim(),
            "scripts": {
                "install": app.install,
                "update": app.script(Action::Update),
                "remove_user": app.remove_user,
                "remove": app.remove,
                "version": app.version,
            },
        });
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(ExitCode::SUCCESS);
    }
    let style = Style::stdout();
    println!("{} ({})", style.bold(&app.name), app.id);
    println!("{}", app.summary);
    println!("Category:  {}", cat.category_name(&app.category));
    println!("Homepage:  {}", app.homepage);
    println!("Method:    {} ({})", app.method, app.scope.description());
    println!("Installed: {}", versions::installed(app).unwrap_or_else(|| "no".into()));
    if !app.latest.is_empty() {
        println!("Latest:    {}", versions::latest(app).unwrap_or_else(|e| format!("unknown ({e})")));
    }
    if !app.requires.is_empty() {
        println!("Requires:  {}", app.requires.join(", "));
    }
    if !app.purge.is_empty() {
        println!("Data:      {}", app.purge.join(" "));
    }
    if !app.command.is_empty() {
        println!("Command:   {}", app.command);
    } else if !app.desktop.is_empty() {
        println!("Starts:    {}", app.desktop.join(", "));
    }
    if !app.notes.is_empty() {
        println!("\n{}", app.notes.trim());
    }
    for (title, script) in [
        ("Install", app.install.as_str()),
        ("Update", app.script(Action::Update)),
        ("Remove (as you first)", app.remove_user.as_str()),
        ("Remove", app.remove.as_str()),
    ] {
        if !script.trim().is_empty() {
            println!("\n{}\n{}", style.step(&format!("# {title}")), script.trim());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn act(action: Action, args: &[String]) -> Result<ExitCode> {
    let accepted: &[&str] = if action == Action::Remove { &["--purge", "--dry-run"] } else { &["--dry-run"] };
    let args = Args::parse(action.as_str(), args, accepted)?;
    let cat = load();
    let purge = args.has("--purge");
    let named: Vec<&App> = args.names.iter().map(|id| find(&cat, id)).collect::<Result<_, _>>()?;
    let mut steps: Vec<Step> = Vec::new();
    match action {
        Action::Update if named.is_empty() => {
            println!("Checking for updates...");
            let apps: Vec<&App> = cat.apps.iter().collect();
            let found = par_map(&apps, |a| {
                let v = versions::installed(a)?;
                let l = versions::latest(a).ok()?;
                versions::is_newer(&v, &l).then(|| (a.id.clone(), v, l))
            });
            for (id, v, l) in found.into_iter().flatten() {
                println!("  {id}: {v} -> {l}");
                steps.push(Step { id, action, purge: false });
            }
            if steps.is_empty() {
                println!("Everything is up to date.");
                return Ok(ExitCode::SUCCESS);
            }
        }
        _ if named.is_empty() => return Err(Usage("name at least one app. See `agent-center list`.".into()).into()),
        _ => {
            for app in named {
                let present = versions::installed(app).is_some();
                match action {
                    Action::Install if present => {
                        println!("{} is already installed.", app.name);
                        continue;
                    }
                    Action::Update | Action::Remove if !present => {
                        println!("{} is not installed.", app.name);
                        continue;
                    }
                    _ => {}
                }
                steps.push(Step { id: app.id.clone(), action, purge });
            }
        }
    }
    if steps.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    let steps = with_requirements(&cat, steps, &|a| versions::installed(a).is_some());
    if args.has("--dry-run") {
        print_plan(&cat, &steps);
        return Ok(ExitCode::SUCCESS);
    }
    let tty = std::io::stdin().is_terminal();
    let style = Style::stdout();
    runner::run(&cat, &steps, tty, &mut |line| {
        if line.starts_with("==> ") { println!("{}", style.step(line)) } else { println!("{line}") }
    })?;
    println!("{}", style.green("Done."));
    Ok(ExitCode::SUCCESS)
}

/// What `steps` would run, in order, for `--dry-run`.
fn print_plan(cat: &Catalog, steps: &[Step]) {
    let style = Style::stdout();
    println!("{}", style.bold("Dry run: nothing is changed. Agent Center would run, in this order:"));
    let script = |title: String, who: &str, text: &str| {
        println!("\n{}", style.step(&format!("==> {title} ({who})")));
        for line in text.trim().lines() {
            println!("    {line}");
        }
    };
    for step in steps {
        let Some(app) = cat.get(&step.id) else { continue };
        if step.action == Action::Remove && !app.remove_user.trim().is_empty() {
            script(format!("Preparing the removal of {}", app.name), "as you", &app.remove_user);
        }
        let who = match app.scope {
            Scope::System => "as root",
            Scope::User => "as you",
        };
        script(format!("{} {}", step.action.verb_ing(), app.name), who, app.script(step.action));
        if step.action == Action::Remove && step.purge {
            for path in app.purge_paths() {
                println!("\n{}", style.step(&format!("==> Deleting {}", path.display())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typos_and_prefixes_are_suggested() {
        let ids = ["claude-code", "claude-desktop", "codex", "cursor", "firefox"];
        assert_eq!(similar("claude", ids), ["claude-code", "claude-desktop"]);
        assert_eq!(similar("codx", ids), ["codex"]);
        assert_eq!(similar("firfeox", ids), ["firefox"]);
        assert_eq!(similar("isntall", COMMANDS), ["install"]);
        assert_eq!(similar("zed", ids), Vec::<&str>::new());
        assert_eq!(similar("--instaled", ["--installed", "--updates"]), ["--installed"]);
    }

    #[test]
    fn distance_counts_edits() {
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("ab", "ba"), 1);
        assert_eq!(distance("same", "same"), 0);
    }

    #[test]
    fn suggestions_read_naturally() {
        assert_eq!(did_you_mean(&[]), "");
        assert_eq!(did_you_mean(&["a"]), " Did you mean a?");
        assert_eq!(did_you_mean(&["a", "b", "c"]), " Did you mean a, b or c?");
    }

    #[test]
    fn unknown_options_are_refused() {
        let args: Vec<String> = ["codex", "--purge"].map(String::from).to_vec();
        let parsed = Args::parse("remove", &args, &["--purge"]).unwrap();
        assert_eq!((parsed.has("--purge"), parsed.names), (true, vec!["codex"]));
        let err = Args::parse("install", &args, &[]).err().unwrap();
        assert_eq!(err.to_string(), "unknown option --purge for install.");
    }
}
