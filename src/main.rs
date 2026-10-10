//! Agent Center: an app center for AI coding tools, browsers and editors on Debian.
//!
//! Without arguments it opens the window; with a command it works in the terminal.

mod catalog;
#[cfg(feature = "gui")]
mod gui;
#[cfg(feature = "gui")]
mod notifier;
mod runner;
mod versions;

use catalog::{Action, App, Catalog};
use runner::Step;
use std::io::IsTerminal;
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

fn main() {
    // Exit quietly when the reader goes away (`agent-center list | head`) instead of panicking.
    // SAFETY: restoring the default signal action before any other thread exists.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None | Some("--updates") => open_window(&args),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            0
        }
        Some("-V" | "--version") => {
            println!("agent-center {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("askpass") => askpass(&args[1..]),
        Some("notify") => notify(&args[1..]),
        Some("run-system") => runner::run_system(&args[1..]),
        Some("list") => list(&args[1..]),
        Some("categories") => {
            let cat = load();
            for c in &cat.categories {
                println!("{:<12} {}", c.id, c.name);
            }
            0
        }
        Some("info") => info(&args[1..]),
        Some("install") => act(Action::Install, &args[1..]),
        Some("update" | "upgrade") => act(Action::Update, &args[1..]),
        Some("remove" | "uninstall") => act(Action::Remove, &args[1..]),
        Some(other) => {
            eprintln!("agent-center: unknown command {other}\n\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(feature = "gui")]
fn open_window(args: &[String]) -> i32 {
    gui::run(args)
}

#[cfg(not(feature = "gui"))]
fn open_window(_: &[String]) -> i32 {
    println!("{USAGE}");
    0
}

#[cfg(feature = "gui")]
fn notify(args: &[String]) -> i32 {
    notifier::run(args)
}

#[cfg(not(feature = "gui"))]
fn notify(_: &[String]) -> i32 {
    eprintln!("agent-center: notifications need the window (built without the gui feature)");
    1
}

#[cfg(feature = "gui")]
fn askpass(args: &[String]) -> i32 {
    gui::askpass(args.first().map(String::as_str).unwrap_or("Password:"))
}

#[cfg(not(feature = "gui"))]
fn askpass(_: &[String]) -> i32 {
    1
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
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

fn list(args: &[String]) -> i32 {
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
    0
}

fn info(args: &[String]) -> i32 {
    let cat = load();
    let Some(app) = args.first().and_then(|id| cat.get(id)) else {
        eprintln!("agent-center: unknown app. See `agent-center list`.");
        return 2;
    };
    println!("{} ({})", app.name, app.id);
    println!("{}", app.summary);
    println!("Category:  {}", cat.category_name(&app.category));
    println!("Homepage:  {}", app.homepage);
    println!("Method:    {} ({})", app.method, scope_name(app));
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
    0
}

pub fn scope_name(app: &App) -> &'static str {
    match app.scope {
        catalog::Scope::System => "system-wide, needs administrator rights",
        catalog::Scope::User => "in your home directory",
    }
}

/// Adds the installation of missing requirements before the steps that need them.
pub fn with_requirements(cat: &Catalog, steps: Vec<Step>, is_installed: &dyn Fn(&App) -> bool) -> Vec<Step> {
    fn visit(cat: &Catalog, id: &str, is_installed: &dyn Fn(&App) -> bool, res: &mut Vec<Step>, depth: usize) {
        let Some(app) = cat.get(id) else { return };
        if depth > 8 {
            return;
        }
        for r in &app.requires {
            if let Some(dep) = cat.get(r)
                && !is_installed(dep)
                && !res.iter().any(|s| s.id == *r)
            {
                visit(cat, r, is_installed, res, depth + 1);
                res.push(Step { id: r.clone(), action: Action::Install, purge: false });
            }
        }
    }
    let mut res = Vec::new();
    for step in steps {
        if step.action == Action::Install {
            visit(cat, &step.id, is_installed, &mut res, 0);
        }
        if !res.contains(&step) {
            res.push(step);
        }
    }
    res
}

fn act(action: Action, args: &[String]) -> i32 {
    let cat = load();
    let purge = args.iter().any(|a| a == "--purge");
    let ids: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    for id in &ids {
        if cat.get(id).is_none() {
            eprintln!("agent-center: unknown app {id}. See `agent-center list`.");
            return 2;
        }
    }
    let mut steps: Vec<Step> = Vec::new();
    match action {
        Action::Update if ids.is_empty() => {
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
                return 0;
            }
        }
        _ if ids.is_empty() => {
            eprintln!("agent-center: name at least one app. See `agent-center list`.");
            return 2;
        }
        _ => {
            for id in ids {
                let app = cat.get(id).unwrap();
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
                steps.push(Step { id: id.clone(), action, purge });
            }
        }
    }
    if steps.is_empty() {
        return 0;
    }
    let steps = with_requirements(&cat, steps, &|a| versions::installed(a).is_some());
    let tty = std::io::stdin().is_terminal();
    match runner::run(&cat, &steps, tty, &mut |line| println!("{line}")) {
        Ok(()) => {
            println!("Done.");
            0
        }
        Err(e) => {
            eprintln!("agent-center: {e}");
            1
        }
    }
}
