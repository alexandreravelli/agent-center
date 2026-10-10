//! The catalog: every app Agent Center knows, read from TOML files.
//!
//! Files are read from these directories, in order. An app id defined again in a later
//! directory replaces the earlier definition:
//!
//! 1. `/usr/share/agent-center/catalog` (shipped with the package)
//! 2. `/etc/agent-center/catalog.d` (added by the administrator)
//! 3. `~/.config/agent-center/catalog.d` (added by the user; `scope = "user"` apps only)

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const SYSTEM_DIRS: [&str; 2] = ["/usr/share/agent-center/catalog", "/etc/agent-center/catalog.d"];
/// The vendors' logos shipped with Agent Center (`AGENT_CENTER_LOGOS` overrides it for development).
const LOGO_DIR: &str = "/usr/share/agent-center/logos";

/// Where an app is installed, which decides whether its scripts run as root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// System-wide (APT, `/usr/local`): scripts run as root through sudo or pkexec.
    #[default]
    System,
    /// In the user's home directory: scripts run as the user.
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Install,
    Update,
    Remove,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s {
            "install" => Some(Action::Install),
            "update" => Some(Action::Update),
            "remove" => Some(Action::Remove),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Action::Install => "install",
            Action::Update => "update",
            Action::Remove => "remove",
        }
    }

    /// "Installing", "Updating", "Removing".
    pub fn verb_ing(self) -> &'static str {
        match self {
            Action::Install => "Installing",
            Action::Update => "Updating",
            Action::Remove => "Removing",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub id: String,
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub homepage: String,
    /// Listed first, under "Featured", in the views that mix categories.
    #[serde(default)]
    pub featured: bool,
    /// The vendor's logo: a file name in the logo directory, without `.svg` (default: the
    /// id), or an absolute path to an SVG or PNG file.
    #[serde(default)]
    pub logo: String,
    /// Color of the tile behind the logo (default: white), for logos drawn on a dark background.
    #[serde(default)]
    pub logo_background: String,
    /// Icon name from the icon theme, used when there is no logo file.
    #[serde(default)]
    pub icon: String,
    /// Background color of the lettered badge shown when there is no icon.
    #[serde(default)]
    pub color: String,
    /// One or two characters for that badge (default: the name's first two letters).
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub scope: Scope,
    /// How the app was installed, in a few words ("npm package", "Mozilla APT repository").
    #[serde(default)]
    pub method: String,
    /// Other catalog apps this one needs (installed first by `install`).
    #[serde(default)]
    pub requires: Vec<String>,
    /// Shell command printing the installed version; it fails or prints nothing when the app
    /// is not installed. Runs as the user.
    pub version: String,
    /// Where to find the latest version: `npm:<package>`, `github:<owner>/<repo>`,
    /// `pypi:<package>`, `apt:<package>` or `cmd:<shell command>`. Empty when unknown.
    #[serde(default)]
    pub latest: String,
    pub install: String,
    /// Defaults to the install script.
    #[serde(default)]
    pub update: String,
    pub remove: String,
    /// Run as the user before `remove` (for example to stop a user service).
    #[serde(default)]
    pub remove_user: String,
    /// Settings and data left in the home directory, deleted on request after a removal.
    #[serde(default)]
    pub purge: Vec<String>,
    /// Shown in the details window.
    #[serde(default)]
    pub notes: String,
    /// What the Open button starts once the app is installed: a desktop entry
    /// (`firefox.desktop`) for apps with a window, or a command run in a terminal (`claude`) for
    /// command-line tools. Empty: no Open button.
    #[serde(default)]
    pub open: String,
}

impl App {
    /// The logo file, when there is one.
    pub fn logo_path(&self) -> Option<PathBuf> {
        let name = if self.logo.is_empty() { &self.id } else { &self.logo };
        let path = if name.starts_with('/') {
            PathBuf::from(name)
        } else {
            let dir = std::env::var_os("AGENT_CENTER_LOGOS").map_or_else(|| PathBuf::from(LOGO_DIR), PathBuf::from);
            dir.join(format!("{name}.svg"))
        };
        path.is_file().then_some(path)
    }

    pub fn script(&self, action: Action) -> &str {
        match action {
            Action::Install => &self.install,
            Action::Update if self.update.trim().is_empty() => &self.install,
            Action::Update => &self.update,
            Action::Remove => &self.remove,
        }
    }

    /// The purge paths with `~` expanded, keeping only paths inside the home directory.
    pub fn purge_paths(&self) -> Vec<PathBuf> {
        let Some(home) = home_dir() else { return Vec::new() };
        self.purge
            .iter()
            .filter_map(|p| {
                let rest = p.strip_prefix("~/")?;
                if rest.is_empty() || rest.split('/').any(|c| c == ".." || c.is_empty()) {
                    return None;
                }
                Some(home.join(rest))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    category: Vec<Category>,
    #[serde(default)]
    app: Vec<App>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    pub categories: Vec<Category>,
    pub apps: Vec<App>,
    /// Problems found while reading the files, for the log.
    pub warnings: Vec<String>,
}

impl Catalog {
    /// The full catalog seen by the user.
    pub fn load() -> Catalog {
        if let Some(dir) = std::env::var_os("AGENT_CENTER_CATALOG") {
            return Catalog::load_dirs(&[(PathBuf::from(dir), true)]);
        }
        let mut dirs: Vec<(PathBuf, bool)> = SYSTEM_DIRS.iter().map(|d| (PathBuf::from(d), true)).collect();
        if let Some(home) = home_dir() {
            dirs.push((home.join(".config/agent-center/catalog.d"), false));
        }
        Catalog::load_dirs(&dirs)
    }

    /// Only the root-owned directories: what scripts running as root may come from.
    pub fn load_system() -> Catalog {
        let dirs: Vec<(PathBuf, bool)> = SYSTEM_DIRS.iter().map(|d| (PathBuf::from(d), true)).collect();
        Catalog::load_dirs(&dirs)
    }

    /// `(directory, trusted)`: apps from untrusted directories may only use `scope = "user"`.
    fn load_dirs(dirs: &[(PathBuf, bool)]) -> Catalog {
        let mut cat = Catalog::default();
        let mut index: HashMap<String, usize> = HashMap::new();
        for (dir, trusted) in dirs {
            for path in toml_files(dir) {
                let file: File = match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| toml::from_str(&s).map_err(|e| e.to_string()))
                {
                    Ok(f) => f,
                    Err(e) => {
                        cat.warnings.push(format!("{}: {e}", path.display()));
                        continue;
                    }
                };
                for c in file.category {
                    match cat.categories.iter_mut().find(|x| x.id == c.id) {
                        Some(x) => *x = c,
                        None => cat.categories.push(c),
                    }
                }
                for app in file.app {
                    if !trusted && app.scope == Scope::System {
                        cat.warnings.push(format!(
                            "{}: app \"{}\" skipped: apps defined in your home directory must use scope = \"user\"",
                            path.display(),
                            app.id
                        ));
                        continue;
                    }
                    match index.get(&app.id) {
                        Some(&i) => cat.apps[i] = app,
                        None => {
                            index.insert(app.id.clone(), cat.apps.len());
                            cat.apps.push(app);
                        }
                    }
                }
            }
        }
        cat
    }

    pub fn get(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id)
    }

    pub fn category_name(&self, id: &str) -> String {
        self.categories.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_string())
    }
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    files
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_catalog_is_valid() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog");
        let cat = Catalog::load_dirs(&[(dir, true)]);
        assert!(cat.warnings.is_empty(), "{:?}", cat.warnings);
        assert!(cat.apps.len() > 10);
        let mut ids = std::collections::HashSet::new();
        for app in &cat.apps {
            assert!(ids.insert(&app.id), "duplicate id {}", app.id);
            assert!(cat.categories.iter().any(|c| c.id == app.category), "{}: unknown category", app.id);
            assert!(!app.version.trim().is_empty(), "{}: no version command", app.id);
            assert!(!app.remove.trim().is_empty(), "{}: no remove script", app.id);
            for r in &app.requires {
                assert!(cat.get(r).is_some(), "{}: unknown requirement {r}", app.id);
            }
            let ok = ["npm:", "github:", "pypi:", "apt:", "cmd:"].iter().any(|p| app.latest.starts_with(p));
            assert!(app.latest.is_empty() || ok, "{}: bad latest source", app.id);
            for p in &app.purge {
                assert!(p.starts_with("~/") && !p.contains(".."), "{}: purge path {p} must be in ~/", app.id);
            }
            if app.open.ends_with(".desktop") {
                assert!(!app.open.contains('/'), "{}: open names a desktop file ID, not a path", app.id);
            }
        }
    }

    #[test]
    fn shipped_apps_have_logos() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cat = Catalog::load_dirs(&[(root.join("catalog"), true)]);
        for app in &cat.apps {
            let name = if app.logo.is_empty() { &app.id } else { &app.logo };
            assert!(!name.contains('/'), "{}: shipped logos are file names", app.id);
            let path = root.join("logos").join(format!("{name}.svg"));
            assert!(path.is_file(), "{}: missing {}", app.id, path.display());
        }
    }

    #[test]
    fn purge_paths_stay_in_home() {
        let app = App { purge: vec!["~/.claude".into(), "~/../etc".into(), "/etc".into(), "~/".into()], ..sample() };
        let home = home_dir().unwrap();
        assert_eq!(app.purge_paths(), vec![home.join(".claude")]);
    }

    pub fn sample() -> App {
        toml::from_str(
            r#"id = "x"
name = "X"
category = "agents"
version = "true"
install = "true"
remove = "true""#,
        )
        .unwrap()
    }
}
