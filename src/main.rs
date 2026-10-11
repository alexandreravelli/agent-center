//! Agent Center: an app center for AI coding tools, browsers and editors on Debian.
//!
//! Without arguments it opens the window; with a command it works in the terminal.

#[cfg(feature = "gui")]
mod gui;
#[cfg(feature = "gui")]
mod notifier;

use agent_center::catalog::{Action, App, Catalog};
use agent_center::runner::{self, Step, with_requirements};
use agent_center::versions;
use anyhow::Result;
use std::fmt;
use std::io::IsTerminal;
use std::process::ExitCode;
use std::thread;

const USAGE: &str = "\
Usage: agent-center [--updates | COMMAND]

Without a command, opens the Agent Center window (on the Updates page with --updates).

Commands:
  list [--installed] [--updates]    List the apps (with the latest versions for --updates)
  info <app>                        Show an app and the scripts Agent Center runs for it
  install <app>...                  Install apps (and what they need)
  update [<app>...]                 Update apps; without names, every app with an update
  remove [--purge] <app>...         Remove apps; --purge also deletes their settings and data
  categories                        List the categories
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

fn main() -> ExitCode {
    // Exit quietly when the reader goes away (`agent-center list | head`) instead of panicking.
    // SAFETY: restoring the default signal action before any other thread exists.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("agent-center: {e:#}");
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
        Some("notify") => notify(rest),
        Some("run-system") => Ok(exit_code(runner::run_system(rest))),
        Some("list") => list(rest),
        Some("categories") => {
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
        Some(other) => Err(Usage(format!("unknown command {other}\n\n{USAGE}")).into()),
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
        eprintln!("warning: {w}");
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

fn list(args: &[String]) -> Result<ExitCode> {
    let cat = load();
    let installed_only = args.iter().any(|a| a == "--installed");
    let updates = args.iter().any(|a| a == "--updates");
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
    for c in &cat.categories {
        let rows: Vec<usize> = (0..apps.len())
            .filter(|&i| apps[i].category == c.id)
            .filter(|&i| !installed_only || installed[i].is_some())
            .filter(|&i| {
                !updates || matches!((&installed[i], &latest[i]), (Some(a), Some(b)) if versions::is_newer(a, b))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        println!("{}", c.name);
        for i in rows {
            let a = apps[i];
            let state = match (&installed[i], &latest[i]) {
                (Some(v), Some(l)) if versions::is_newer(v, l) => format!("{v} -> {l}"),
                (Some(v), _) => v.clone(),
                (None, _) => "-".into(),
            };
            println!("  {:<18} {:<22} {:<20} {}", a.id, a.name, state, a.summary);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The catalog app with this id, or a usage error.
fn find<'a>(cat: &'a Catalog, id: &str) -> Result<&'a App, Usage> {
    cat.get(id).ok_or_else(|| Usage(format!("unknown app {id}. See `agent-center list`.")))
}

fn info(args: &[String]) -> Result<ExitCode> {
    let cat = load();
    let id = args.first().ok_or_else(|| Usage("name an app. See `agent-center list`.".into()))?;
    let app = find(&cat, id)?;
    println!("{} ({})", app.name, app.id);
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
            println!("\n# {title}\n{}", script.trim());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn act(action: Action, args: &[String]) -> Result<ExitCode> {
    let cat = load();
    let purge = args.iter().any(|a| a == "--purge");
    let named: Vec<&App> =
        args.iter().filter(|a| !a.starts_with('-')).map(|id| find(&cat, id)).collect::<Result<_, _>>()?;
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
    let tty = std::io::stdin().is_terminal();
    runner::run(&cat, &steps, tty, &mut |line| println!("{line}"))?;
    println!("Done.");
    Ok(ExitCode::SUCCESS)
}
