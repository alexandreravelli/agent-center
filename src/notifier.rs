//! `agent-center notify`: started with the desktop session (XDG autostart). It checks the
//! installed apps for updates a few minutes after login, then once a day, and shows a desktop
//! notification when new versions are out. Clicking it opens Agent Center on the Updates page.

use crate::catalog::{App, Catalog};
use crate::versions;
use gtk::gio;
use gtk::glib::{self, VariantTy};
use gtk::prelude::*;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::thread;

/// Leaves the network and the disks to the session while it starts.
const FIRST_CHECK_SECS: u32 = 180;
const CHECK_EVERY_SECS: u32 = 24 * 60 * 60;
/// Owned on the session bus, so a session runs one notifier.
const BUS_NAME: &str = "io.github.alexandreravelli.AgentCenter.Notifier";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";

struct Update {
    id: String,
    name: String,
    latest: String,
}

impl Update {
    fn key(&self) -> String {
        format!("{} {}", self.id, self.latest)
    }
}

pub fn run(args: &[String]) -> i32 {
    let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("agent-center: no desktop session bus: {e}");
            return 1;
        }
    };
    if !own_name(&conn) {
        eprintln!("agent-center: the update notifier is already running");
        return 0;
    }
    // The id of the notification on screen, which a click refers to.
    let shown = Rc::new(Cell::new(0u32));
    let _clicks = {
        let shown = shown.clone();
        conn.subscribe_to_signal(
            None,
            Some(NOTIFICATIONS),
            Some("ActionInvoked"),
            Some(NOTIFICATIONS_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if let Some((id, _action)) = signal.parameters.get::<(u32, String)>()
                    && id != 0
                    && id == shown.get()
                {
                    open_updates();
                }
            },
        )
    };

    let (tx, rx) = async_channel::unbounded::<Vec<Update>>();
    let check = move || {
        let tx = tx.clone();
        thread::spawn(move || tx.send_blocking(find_updates()).ok());
    };
    let first = if args.iter().any(|a| a == "--now") { 0 } else { FIRST_CHECK_SECS };
    {
        let check = check.clone();
        glib::timeout_add_seconds_local_once(first, check);
    }
    glib::timeout_add_seconds_local(CHECK_EVERY_SECS, move || {
        check();
        glib::ControlFlow::Continue
    });
    glib::spawn_future_local(async move {
        while let Ok(updates) = rx.recv().await {
            notify(&conn, &updates, &shown);
        }
    });
    glib::MainLoop::new(None, false).run();
    0
}

fn own_name(conn: &gio::DBusConnection) -> bool {
    // Flag 4: DBUS_NAME_FLAG_DO_NOT_QUEUE. Reply 1: DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER.
    conn.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&(BUS_NAME, 4u32).to_variant()),
        VariantTy::new("(u)").ok(),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
    )
    .ok()
    .and_then(|v| v.get::<(u32,)>())
    .is_some_and(|(reply,)| reply == 1)
}

/// The installed apps with a newer version, in catalog order.
fn find_updates() -> Vec<Update> {
    let cat = Catalog::load();
    let apps: Vec<&App> = cat.apps.iter().collect();
    crate::par_map(&apps, |a| {
        let installed = versions::installed(a)?;
        let latest = versions::latest(a).ok()?;
        versions::is_newer(&installed, &latest).then(|| Update { id: a.id.clone(), name: a.name.clone(), latest })
    })
    .into_iter()
    .flatten()
    .collect()
}

/// Shows the notification when there is a version it has not announced yet, so the same
/// updates do not come back every day.
fn notify(conn: &gio::DBusConnection, updates: &[Update], shown: &Rc<Cell<u32>>) {
    let state = state_file();
    let announced = std::fs::read_to_string(&state).unwrap_or_default();
    let keys: Vec<String> = updates.iter().map(Update::key).collect();
    if !keys.iter().any(|k| !announced.lines().any(|l| l == k)) {
        return;
    }
    let summary = match updates.len() {
        1 => "1 update available".to_string(),
        n => format!("{n} updates available"),
    };
    let names: Vec<&str> = updates.iter().map(|u| u.name.as_str()).collect();
    let body = match names.len() {
        1..=3 => names.join(", "),
        n => format!("{}, and {} more", names[..2].join(", "), n - 2),
    };
    // "default" is a click on the notification (a button on XFCE).
    let actions: Vec<String> = vec!["default".into(), "Show Updates".into()];
    let hints: HashMap<String, glib::Variant> =
        HashMap::from([("desktop-entry".to_string(), "agent-center".to_variant())]);
    let params = ("Agent Center", shown.get(), "agent-center", summary, body, actions, hints, -1i32).to_variant();
    let shown = shown.clone();
    let text = keys.join("\n") + "\n";
    conn.call(
        Some(NOTIFICATIONS),
        NOTIFICATIONS_PATH,
        NOTIFICATIONS,
        "Notify",
        Some(&params),
        VariantTy::new("(u)").ok(),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |res| match res {
            Ok(v) => {
                if let Some((id,)) = v.get::<(u32,)>() {
                    shown.set(id);
                }
                if let Some(dir) = state.parent() {
                    std::fs::create_dir_all(dir).ok();
                }
                std::fs::write(&state, text).ok();
            }
            Err(e) => eprintln!("agent-center: cannot show the notification: {e}"),
        },
    );
}

/// The versions already announced: ~/.local/state/agent-center/announced.
fn state_file() -> PathBuf {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::catalog::home_dir().map(|h| h.join(".local/state")))
        .unwrap_or_else(std::env::temp_dir);
    dir.join("agent-center/announced")
}

fn open_updates() {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/agent-center"));
    match Command::new(exe).arg("--updates").spawn() {
        Ok(child) => crate::gui::reap(child),
        Err(e) => eprintln!("agent-center: cannot open the window: {e}"),
    }
}
