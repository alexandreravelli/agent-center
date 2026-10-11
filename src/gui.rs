//! The Agent Center window (GTK 3).

use agent_center::catalog::{self, Action, App, Catalog};
use agent_center::runner::{self, Step};
use agent_center::versions;
use anyhow::{anyhow, bail};
use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{cairo, gio, glib, pango};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::rc::Rc;
use std::sync::{Arc, PoisonError};
use std::thread;

const APP_ID: &str = "io.github.alexandreravelli.AgentCenter";

const CSS: &str = "
.ac-sidebar { border-right: 1px solid alpha(currentColor, 0.15); }
.ac-sidebar row { padding: 8px 14px; }
.ac-count { font-size: smaller; opacity: 0.7; }
.ac-title { font-size: 1.4em; font-weight: bold; }
.ac-header { font-weight: bold; padding: 14px 16px 4px 16px; opacity: 0.8; }
.ac-name { font-weight: bold; }
.ac-meta { font-size: smaller; }
.ac-update { color: #3584e4; font-weight: bold; }
.ac-log { font-family: monospace; font-size: 9pt; }
";

#[derive(Clone, PartialEq)]
enum View {
    All,
    Installed,
    Updates,
    Category(String),
}

#[derive(Clone, Copy, PartialEq)]
enum JobState {
    Idle,
    Queued,
    Running,
}

struct Entry {
    installed: Option<String>,
    latest: Option<String>,
    checking: bool,
    job: JobState,
}

impl Entry {
    fn has_update(&self) -> bool {
        matches!((&self.installed, &self.latest), (Some(i), Some(l)) if versions::is_newer(i, l))
    }
}

struct RowWidgets {
    row: gtk::ListBoxRow,
    meta: gtk::Label,
    spinner: gtk::Spinner,
    primary: gtk::Button,
    open: gtk::Button,
    remove: gtk::Button,
}

enum Msg {
    Installed(usize, Option<String>),
    Latest(usize, Option<String>),
    Line(String),
    JobDone(Vec<usize>, Result<(), runner::RunError>),
}

struct Ui {
    cat: Arc<Catalog>,
    /// The catalog index of the app on each list row: featured apps first, then catalog order.
    order: Vec<usize>,
    entries: RefCell<Vec<Entry>>,
    rows: RefCell<Vec<RowWidgets>>,
    window: gtk::ApplicationWindow,
    list: gtk::ListBox,
    view: RefCell<View>,
    search: gtk::SearchEntry,
    title: gtk::Label,
    subtitle: gtk::Label,
    empty: gtk::Label,
    stack: gtk::Stack,
    updates_count: gtk::Label,
    installed_count: gtk::Label,
    update_all: gtk::Button,
    refresh: gtk::Button,
    status: gtk::Label,
    status_spinner: gtk::Spinner,
    log: gtk::TextBuffer,
    log_view: gtk::TextView,
    log_end: gtk::TextMark,
    details: gtk::ToggleButton,
    queue: RefCell<VecDeque<Vec<Step>>>,
    running: Cell<bool>,
    /// The last job failed: its message stays in the status bar until the next action.
    failed: Cell<bool>,
    pending_checks: Cell<usize>,
    tx: async_channel::Sender<Msg>,
}

/// Opens the window, or brings the open one to the front. `--updates` shows the Updates page.
pub fn run(args: &[String]) -> i32 {
    let app =
        gtk::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    app.connect_activate(build);
    // A second `agent-center` hands its arguments to the running one.
    app.connect_command_line(|app, cmd| {
        app.activate();
        if cmd.arguments().iter().any(|a| a.to_str() == Some("--updates")) {
            app.activate_action("show-updates", None);
        }
        glib::ExitCode::SUCCESS
    });
    let mut argv = vec!["agent-center".to_string()];
    argv.extend(args.iter().cloned());
    app.run_with_args(&argv);
    0
}

fn build(app: &gtk::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS.as_bytes()).ok();
    if let Some(screen) = gtk::gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(&screen, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    let cat = Arc::new(Catalog::load());
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Agent Center")
        .icon_name("agent-center")
        .default_width(1000)
        .default_height(680)
        .build();

    // Header bar: search, refresh, update all.
    let header = gtk::HeaderBar::builder().show_close_button(true).title("Agent Center").build();
    let search = gtk::SearchEntry::builder().placeholder_text("Search apps").width_chars(24).build();
    let refresh = gtk::Button::from_icon_name(Some("view-refresh-symbolic"), gtk::IconSize::Button);
    refresh.set_tooltip_text(Some("Check for updates"));
    let update_all = gtk::Button::with_label("Update All");
    update_all.style_context().add_class("suggested-action");
    update_all.set_sensitive(false);
    header.pack_start(&search);
    header.pack_end(&update_all);
    header.pack_end(&refresh);
    window.set_titlebar(Some(&header));

    // Sidebar.
    let sidebar = gtk::ListBox::new();
    sidebar.style_context().add_class("ac-sidebar");
    sidebar.set_selection_mode(gtk::SelectionMode::Browse);
    sidebar.set_size_request(210, -1);
    let installed_count = gtk::Label::new(None);
    let updates_count = gtk::Label::new(None);
    let mut views = vec![
        (View::All, "All Apps".to_string(), None),
        (View::Installed, "Installed".into(), Some(installed_count.clone())),
        (View::Updates, "Updates".into(), Some(updates_count.clone())),
    ];
    for c in &cat.categories {
        views.push((View::Category(c.id.clone()), c.name.clone(), None));
    }
    for (i, (_, name, count)) in views.iter().enumerate() {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        b.pack_start(&gtk::Label::builder().label(name).xalign(0.0).build(), true, true, 0);
        if let Some(count) = count {
            count.style_context().add_class("ac-count");
            b.pack_end(count, false, false, 0);
        }
        let row = gtk::ListBoxRow::new();
        row.add(&b);
        sidebar.add(&row);
        if i == 2 {
            let sep = gtk::ListBoxRow::new();
            sep.add(&gtk::Separator::new(gtk::Orientation::Horizontal));
            sep.set_selectable(false);
            sep.set_activatable(false);
            sidebar.add(&sep);
        }
    }

    // Main area.
    let title = gtk::Label::builder().xalign(0.0).build();
    title.style_context().add_class("ac-title");
    let subtitle = gtk::Label::builder().xalign(0.0).wrap(true).build();
    subtitle.style_context().add_class("dim-label");
    let heading = gtk::Box::new(gtk::Orientation::Vertical, 4);
    heading.set_margin_start(18);
    heading.set_margin_end(18);
    heading.set_margin_top(16);
    heading.set_margin_bottom(8);
    heading.pack_start(&title, false, false, 0);
    heading.pack_start(&subtitle, false, false, 0);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let empty = gtk::Label::new(Some("Nothing here."));
    empty.style_context().add_class("dim-label");
    let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    scroller.add(&list);
    let stack = gtk::Stack::new();
    stack.add_named(&scroller, "list");
    stack.add_named(&empty, "empty");

    // Status bar and log.
    let log = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
    let log_end = gtk::TextMark::new(Some("end"), false);
    log.add_mark(&log_end, &log.end_iter());
    let log_view = gtk::TextView::builder()
        .buffer(&log)
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    log_view.style_context().add_class("ac-log");
    log_view.set_left_margin(8);
    log_view.set_top_margin(6);
    let log_scroller = gtk::ScrolledWindow::builder().height_request(190).build();
    log_scroller.add(&log_view);
    let revealer = gtk::Revealer::new();
    revealer.add(&log_scroller);
    let status_spinner = gtk::Spinner::new();
    let status = gtk::Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).build();
    let details = gtk::ToggleButton::with_label("Show Log");
    details.set_relief(gtk::ReliefStyle::None);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.set_border_width(6);
    bar.pack_start(&status_spinner, false, false, 4);
    bar.pack_start(&status, true, true, 0);
    bar.pack_end(&details, false, false, 0);
    {
        let revealer = revealer.clone();
        details.connect_toggled(move |b| revealer.set_reveal_child(b.is_active()));
    }

    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.pack_start(&heading, false, false, 0);
    main.pack_start(&stack, true, true, 0);
    main.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);
    main.pack_start(&revealer, false, false, 0);
    main.pack_start(&bar, false, false, 0);

    let sidebar_scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).build();
    sidebar_scroller.add(&sidebar);
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.pack_start(&sidebar_scroller, false, false, 0);
    body.pack_start(&main, true, true, 0);
    window.add(&body);

    let (tx, rx) = async_channel::unbounded();
    let entries =
        cat.apps.iter().map(|_| Entry { installed: None, latest: None, checking: true, job: JobState::Idle }).collect();
    let mut order: Vec<usize> = (0..cat.apps.len()).collect();
    order.sort_by_key(|&i| !cat.apps[i].featured);
    let ui = Rc::new(Ui {
        cat: cat.clone(),
        order,
        entries: RefCell::new(entries),
        rows: RefCell::new(Vec::new()),
        window: window.clone(),
        list: list.clone(),
        view: RefCell::new(View::All),
        search: search.clone(),
        title,
        subtitle,
        empty,
        stack,
        updates_count,
        installed_count,
        update_all: update_all.clone(),
        refresh: refresh.clone(),
        status,
        status_spinner,
        log,
        log_view,
        log_end,
        details,
        queue: RefCell::new(VecDeque::new()),
        running: Cell::new(false),
        failed: Cell::new(false),
        pending_checks: Cell::new(0),
        tx,
    });

    for (i, app) in cat.apps.iter().enumerate() {
        let row = ui.make_row(i, app);
        ui.rows.borrow_mut().push(row);
    }
    for &i in &ui.order {
        list.add(&ui.rows.borrow()[i].row);
    }
    {
        let ui = ui.clone();
        list.set_filter_func(Some(Box::new(move |row| ui.visible(row.index() as usize))));
    }
    {
        let ui = ui.clone();
        list.set_header_func(Some(Box::new(move |row, before| ui.header(row, before))));
    }
    {
        let ui = ui.clone();
        sidebar.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let mut i = row.index() as usize;
            if i > 2 {
                i -= 1; // separator
            }
            if let Some((view, ..)) = views.get(i) {
                *ui.view.borrow_mut() = view.clone();
                ui.refilter();
            }
        });
    }
    sidebar.select_row(sidebar.row_at_index(0).as_ref());
    {
        let show_updates = gio::SimpleAction::new("show-updates", None);
        let (sidebar, window) = (sidebar.clone(), window.clone());
        show_updates.connect_activate(move |_, _| {
            sidebar.select_row(sidebar.row_at_index(2).as_ref());
            window.present();
        });
        app.add_action(&show_updates);
    }
    {
        let ui = ui.clone();
        search.connect_search_changed(move |_| ui.refilter());
    }
    {
        let ui = ui.clone();
        refresh.connect_clicked(move |_| ui.check_all(true));
    }
    {
        let ui = ui.clone();
        update_all.connect_clicked(move |_| ui.update_all());
    }
    {
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                ui.handle(msg);
            }
        });
    }
    for w in &cat.warnings {
        ui.log_line(&format!("warning: {w}"));
    }

    window.show_all();
    ui.refilter();
    ui.check_all(false);
}

impl Ui {
    fn make_row(self: &Rc<Self>, i: usize, app: &App) -> RowWidgets {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        b.set_margin_start(16);
        b.set_margin_end(16);
        b.set_margin_top(10);
        b.set_margin_bottom(10);
        b.pack_start(&avatar(app), false, false, 0);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let name = gtk::Label::builder().label(&app.name).xalign(0.0).build();
        name.style_context().add_class("ac-name");
        let summary = gtk::Label::builder().label(&app.summary).xalign(0.0).wrap(true).max_width_chars(70).build();
        let meta = gtk::Label::builder().xalign(0.0).use_markup(true).build();
        meta.style_context().add_class("ac-meta");
        meta.style_context().add_class("dim-label");
        text.pack_start(&name, false, false, 0);
        text.pack_start(&summary, false, false, 0);
        text.pack_start(&meta, false, false, 2);
        b.pack_start(&text, true, true, 0);

        let spinner = gtk::Spinner::new();
        let primary = gtk::Button::with_label("Install");
        let open = gtk::Button::with_label("Open");
        open.set_tooltip_text(Some(&if app.command.is_empty() {
            format!("Start {}", app.name)
        } else {
            format!("Run {} in a terminal", app.command)
        }));
        let remove = gtk::Button::with_label("Remove");
        let info = gtk::Button::with_label("Details");
        info.set_tooltip_text(Some("What Agent Center runs for this app"));
        info.set_relief(gtk::ReliefStyle::None);
        for w in [&primary, &open, &remove, &info] {
            w.set_valign(gtk::Align::Center);
        }
        primary.set_size_request(92, -1);
        b.pack_end(&info, false, false, 0);
        b.pack_end(&remove, false, false, 0);
        b.pack_end(&primary, false, false, 0);
        b.pack_end(&open, false, false, 0);
        b.pack_end(&spinner, false, false, 0);

        {
            let ui = self.clone();
            primary.connect_clicked(move |_| {
                let action = {
                    let e = &ui.entries.borrow()[i];
                    if e.installed.is_none() { Action::Install } else { Action::Update }
                };
                ui.start(i, action, false);
            });
        }
        {
            let ui = self.clone();
            open.connect_clicked(move |_| ui.open(i));
        }
        {
            let ui = self.clone();
            remove.connect_clicked(move |_| ui.confirm_remove(i));
        }
        {
            let ui = self.clone();
            info.connect_clicked(move |_| ui.show_info(i));
        }

        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.add(&b);
        RowWidgets { row, meta, spinner, primary, open, remove }
    }

    fn visible(&self, pos: usize) -> bool {
        let i = self.order[pos];
        let app = &self.cat.apps[i];
        let e = &self.entries.borrow()[i];
        let ok = match &*self.view.borrow() {
            View::All => true,
            View::Installed => e.installed.is_some(),
            View::Updates => e.has_update(),
            View::Category(c) => app.category == *c,
        };
        let q = self.search.text().to_lowercase();
        ok && (q.is_empty()
            || app.name.to_lowercase().contains(&q)
            || app.id.contains(&q)
            || app.summary.to_lowercase().contains(&q))
    }

    /// The heading of the list row at `pos` in the views that mix categories.
    fn group(&self, pos: usize) -> String {
        let app = &self.cat.apps[self.order[pos]];
        if app.featured { "Featured".into() } else { self.cat.category_name(&app.category) }
    }

    /// Category headings in the views that mix categories.
    fn header(&self, row: &gtk::ListBoxRow, before: Option<&gtk::ListBoxRow>) {
        if matches!(*self.view.borrow(), View::Category(_)) {
            row.set_header(None::<&gtk::Widget>);
            return;
        }
        let group = self.group(row.index() as usize);
        let same = before.is_some_and(|b| self.group(b.index() as usize) == group);
        if same {
            row.set_header(None::<&gtk::Widget>);
        } else if row.header().is_none() || before.is_none() {
            let l = gtk::Label::builder().label(&group).xalign(0.0).build();
            l.style_context().add_class("ac-header");
            row.set_header(Some(&l));
        }
    }

    fn refilter(&self) {
        let (title, subtitle) = match &*self.view.borrow() {
            View::All => ("All Apps".to_string(), "AI coding tools, browsers and editors.".to_string()),
            View::Installed => ("Installed".into(), "Apps installed on this computer.".into()),
            View::Updates => ("Updates".into(), "Installed apps with a newer version.".into()),
            View::Category(c) => {
                let cat = self.cat.categories.iter().find(|x| x.id == *c);
                (cat.map(|x| x.name.clone()).unwrap_or_default(), cat.map(|x| x.summary.clone()).unwrap_or_default())
            }
        };
        self.title.set_text(&title);
        self.subtitle.set_text(&subtitle);
        self.list.invalidate_filter();
        self.list.invalidate_headers();
        let any = (0..self.cat.apps.len()).any(|i| self.visible(i));
        if !any {
            let text = match &*self.view.borrow() {
                View::Updates if self.pending_checks.get() > 0 => "Checking for updates...",
                View::Updates => "Everything is up to date.",
                View::Installed => "No app from the catalog is installed yet.",
                _ => "No app matches your search.",
            };
            self.empty.set_text(text);
        }
        self.stack.set_visible_child_name(if any { "list" } else { "empty" });
    }

    fn update_row(&self, i: usize) {
        let app = &self.cat.apps[i];
        let e = &self.entries.borrow()[i];
        let r = &self.rows.borrow()[i];
        let busy = e.job != JobState::Idle;
        let mut parts: Vec<String> = Vec::new();
        match (&e.installed, &e.latest) {
            (Some(v), Some(l)) if versions::is_newer(v, l) => {
                parts.push(format!("Installed {}", esc(v)));
                parts.push(format!("<span weight=\"bold\" foreground=\"#3584e4\">Update to {}</span>", esc(l)));
            }
            (Some(v), _) => parts.push(format!("Installed {}", esc(v))),
            (None, Some(l)) => parts.push(format!("Version {}", esc(l))),
            (None, None) if e.checking => parts.push("Checking...".into()),
            (None, None) => {}
        }
        parts.push(esc(&app.method));
        match e.job {
            JobState::Queued => parts.insert(0, "<b>Waiting</b>".into()),
            JobState::Running => parts.insert(0, "<b>Working...</b>".into()),
            JobState::Idle => {}
        }
        r.meta.set_markup(&parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("  ·  "));
        if busy {
            r.spinner.start();
            r.spinner.show();
        } else {
            r.spinner.stop();
            r.spinner.hide();
        }
        let ctx = r.primary.style_context();
        match &e.installed {
            None => {
                r.primary.set_label("Install");
                r.primary.show();
                ctx.add_class("suggested-action");
                r.open.hide();
                r.remove.hide();
            }
            Some(_) => {
                r.primary.set_label("Update");
                ctx.remove_class("suggested-action");
                r.primary.set_visible(e.has_update());
                r.open.set_visible(can_open(app));
                r.remove.show();
            }
        }
        r.primary.set_sensitive(!busy && !e.checking);
        r.open.set_sensitive(!busy);
        r.remove.set_sensitive(!busy);
    }

    fn update_counts(&self) {
        let entries = self.entries.borrow();
        let installed = entries.iter().filter(|e| e.installed.is_some()).count();
        let updates = entries.iter().filter(|e| e.has_update() && e.job == JobState::Idle).count();
        self.installed_count.set_text(&installed.to_string());
        self.updates_count.set_text(&if updates > 0 { updates.to_string() } else { String::new() });
        self.update_all.set_sensitive(updates > 0);
        self.update_all.set_label(&if updates > 0 { format!("Update All ({updates})") } else { "Update All".into() });
    }

    /// Detects installed versions, then fetches the latest versions of installed apps.
    fn check_all(self: &Rc<Self>, user_asked: bool) {
        let ids: Vec<usize> =
            (0..self.cat.apps.len()).filter(|&i| self.entries.borrow()[i].job == JobState::Idle).collect();
        if user_asked {
            self.failed.set(false);
            self.set_status("Checking for updates...", true);
        }
        self.check(ids);
    }

    fn check(self: &Rc<Self>, ids: Vec<usize>) {
        if ids.is_empty() {
            return;
        }
        self.refresh.set_sensitive(false);
        for &i in &ids {
            self.entries.borrow_mut()[i].checking = true;
            self.update_row(i);
        }
        self.pending_checks.set(self.pending_checks.get() + ids.len());
        let workers = 8.min(ids.len());
        let queue = Arc::new(std::sync::Mutex::new(ids));
        for _ in 0..workers {
            let (queue, cat, tx) = (queue.clone(), self.cat.clone(), self.tx.clone());
            thread::spawn(move || {
                loop {
                    // A poisoned lock still holds a valid list of indexes.
                    let Some(i) = queue.lock().unwrap_or_else(PoisonError::into_inner).pop() else { break };
                    let app = &cat.apps[i];
                    let installed = versions::installed(app);
                    let known = installed.is_some();
                    tx.send_blocking(Msg::Installed(i, installed)).ok();
                    // Latest versions only for installed apps: fewer requests (GitHub allows 60 an hour).
                    let latest = if known && !app.latest.is_empty() { versions::latest(app).ok() } else { None };
                    tx.send_blocking(Msg::Latest(i, latest)).ok();
                }
            });
        }
    }

    fn handle(self: &Rc<Self>, msg: Msg) {
        match msg {
            Msg::Installed(i, v) => {
                self.entries.borrow_mut()[i].installed = v;
                self.update_row(i);
            }
            Msg::Latest(i, l) => {
                {
                    let mut entries = self.entries.borrow_mut();
                    let e = &mut entries[i];
                    if l.is_some() || e.installed.is_none() {
                        e.latest = l;
                    }
                    e.checking = false;
                }
                self.update_row(i);
                let left = self.pending_checks.get().saturating_sub(1);
                self.pending_checks.set(left);
                if left == 0 {
                    self.refresh.set_sensitive(true);
                    if !self.running.get() && !self.failed.get() {
                        let n = self.entries.borrow().iter().filter(|e| e.has_update()).count();
                        let text = match n {
                            0 => "Everything is up to date.".to_string(),
                            1 => "1 update available.".to_string(),
                            n => format!("{n} updates available."),
                        };
                        self.set_status(&text, false);
                    }
                }
                self.update_counts();
                self.refilter();
            }
            Msg::Line(line) => self.log_line(&line),
            Msg::JobDone(ids, result) => {
                self.running.set(false);
                for &i in &ids {
                    self.entries.borrow_mut()[i].job = JobState::Idle;
                }
                match result {
                    Ok(()) => {
                        self.log_line("==> Done.");
                        self.set_status("Done.", false);
                    }
                    Err(e) => {
                        self.log_line(&format!("error: {e}"));
                        let name = match ids.as_slice() {
                            [i] => self.cat.apps[*i].name.clone(),
                            _ => "A step".into(),
                        };
                        self.set_status(&format!("{name} failed. See the log for details."), false);
                        self.failed.set(true);
                        self.details.set_active(true);
                    }
                }
                self.check(ids);
                self.next_job();
            }
        }
    }

    fn set_status(&self, text: &str, busy: bool) {
        self.status.set_text(text);
        if busy {
            self.status_spinner.start();
        } else {
            self.status_spinner.stop();
        }
    }

    fn log_line(&self, line: &str) {
        let mut end = self.log.end_iter();
        if self.log.char_count() > 0 {
            self.log.insert(&mut end, "\n");
        }
        self.log.insert(&mut end, line);
        self.log_view.scroll_mark_onscreen(&self.log_end);
        if self.running.get() && line.starts_with("==> ") {
            self.status.set_text(line.trim_start_matches("==> "));
        }
    }

    fn start(self: &Rc<Self>, i: usize, action: Action, purge: bool) {
        let step = Step { id: self.cat.apps[i].id.clone(), action, purge };
        self.enqueue(vec![step]);
    }

    fn update_all(self: &Rc<Self>) {
        let steps: Vec<Step> = (0..self.cat.apps.len())
            .filter(|&i| {
                let e = &self.entries.borrow()[i];
                e.has_update() && e.job == JobState::Idle
            })
            .map(|i| Step { id: self.cat.apps[i].id.clone(), action: Action::Update, purge: false })
            .collect();
        self.enqueue(steps);
    }

    fn enqueue(self: &Rc<Self>, steps: Vec<Step>) {
        if steps.is_empty() {
            return;
        }
        let steps = {
            let entries = self.entries.borrow();
            let cat = &self.cat;
            runner::with_requirements(cat, steps, &|a: &App| {
                cat.apps.iter().position(|x| x.id == a.id).is_some_and(|i| entries[i].installed.is_some())
            })
        };
        for s in &steps {
            if let Some(i) = self.index(&s.id) {
                self.entries.borrow_mut()[i].job = JobState::Queued;
                self.update_row(i);
            }
        }
        self.update_counts();
        self.queue.borrow_mut().push_back(steps);
        self.next_job();
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.cat.apps.iter().position(|a| a.id == id)
    }

    fn next_job(self: &Rc<Self>) {
        if self.running.get() {
            return;
        }
        let Some(steps) = self.queue.borrow_mut().pop_front() else { return };
        self.running.set(true);
        self.failed.set(false);
        let ids: Vec<usize> = steps.iter().filter_map(|s| self.index(&s.id)).collect();
        for &i in &ids {
            self.entries.borrow_mut()[i].job = JobState::Running;
            self.update_row(i);
        }
        let names: Vec<&str> = ids.iter().map(|&i| self.cat.apps[i].name.as_str()).collect();
        let first = &steps[0];
        let text = if steps.len() == 1 {
            format!("{} {}...", first.action.verb_ing(), names[0])
        } else {
            format!("Working on {}...", names.join(", "))
        };
        self.set_status(&text, true);
        if steps.len() > 1 {
            self.log_line(&format!("==> {text}"));
        }
        let (cat, tx) = (self.cat.clone(), self.tx.clone());
        thread::spawn(move || {
            let result = runner::run(&cat, &steps, false, &mut |line| {
                tx.send_blocking(Msg::Line(line.to_string())).ok();
            });
            tx.send_blocking(Msg::JobDone(ids, result)).ok();
        });
    }

    fn open(&self, i: usize) {
        let app = &self.cat.apps[i];
        if let Err(e) = launch(app) {
            self.log_line(&format!("error: cannot open {}: {e}", app.name));
            self.set_status(&format!("Cannot open {}: {e}", app.name), false);
        }
    }

    fn confirm_remove(self: &Rc<Self>, i: usize) {
        let app = &self.cat.apps[i];
        let dialog = gtk::MessageDialog::new(
            Some(&self.window),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            gtk::MessageType::Question,
            gtk::ButtonsType::None,
            &format!("Remove {}?", app.name),
        );
        dialog.set_secondary_text(Some(&format!("Installed as: {} ({}).", app.method, app.scope.description())));
        let purge = gtk::CheckButton::with_label("Also delete its settings and data");
        let paths: Vec<String> = app.purge_paths().iter().map(|p| p.display().to_string()).collect();
        if !paths.is_empty()
            && let Ok(area) = dialog.message_area().downcast::<gtk::Box>()
        {
            let list = gtk::Label::builder().label(paths.join("\n")).xalign(0.0).selectable(true).build();
            list.style_context().add_class("ac-log");
            list.style_context().add_class("dim-label");
            list.set_margin_start(26);
            area.pack_start(&purge, false, false, 0);
            area.pack_start(&list, false, false, 0);
        }
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        let ok = dialog.add_button("Remove", gtk::ResponseType::Accept);
        ok.style_context().add_class("destructive-action");
        let ui = self.clone();
        dialog.connect_response(move |d, resp| {
            if resp == gtk::ResponseType::Accept {
                ui.start(i, Action::Remove, purge.is_active());
            }
            d.close();
        });
        dialog.show_all();
    }

    fn show_info(&self, i: usize) {
        let app = &self.cat.apps[i];
        let dialog = gtk::Dialog::with_buttons(
            Some(&app.name),
            Some(&self.window),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[("Close", gtk::ResponseType::Close)],
        );
        dialog.set_default_size(700, 560);
        let content = dialog.content_area();
        content.set_spacing(6);
        content.set_border_width(12);

        let grid = gtk::Grid::builder().row_spacing(4).column_spacing(14).build();
        let e = &self.entries.borrow()[i];
        let mut rows: Vec<(&str, String)> = vec![
            ("Category", self.cat.category_name(&app.category)),
            ("Installed", e.installed.clone().unwrap_or_else(|| "No".into())),
        ];
        if let Some(l) = &e.latest {
            rows.push(("Latest", l.clone()));
        }
        rows.push(("Method", format!("{} ({})", app.method, app.scope.description())));
        if !app.requires.is_empty() {
            rows.push(("Requires", app.requires.join(", ")));
        }
        if !app.purge.is_empty() {
            rows.push(("Settings and data", app.purge.join("  ")));
        }
        if !app.command.is_empty() {
            rows.push(("Command", app.command.clone()));
        } else if !app.desktop.is_empty() {
            rows.push(("Desktop entry", app.desktop.join(", ")));
        }
        for (n, (k, v)) in rows.iter().enumerate() {
            let key = gtk::Label::builder().label(*k).xalign(0.0).build();
            key.style_context().add_class("dim-label");
            let val = gtk::Label::builder().label(v).xalign(0.0).selectable(true).wrap(true).build();
            grid.attach(&key, 0, n as i32, 1, 1);
            grid.attach(&val, 1, n as i32, 1, 1);
        }
        let summary = gtk::Label::builder().label(&app.summary).xalign(0.0).wrap(true).build();
        content.pack_start(&summary, false, false, 0);
        if !app.homepage.is_empty() {
            let link = gtk::LinkButton::with_label(&app.homepage, &app.homepage);
            link.set_halign(gtk::Align::Start);
            content.pack_start(&link, false, false, 0);
        }
        content.pack_start(&grid, false, false, 4);
        if !app.notes.trim().is_empty() {
            let notes = gtk::Label::builder().label(app.notes.trim()).xalign(0.0).wrap(true).selectable(true).build();
            content.pack_start(&notes, false, false, 4);
        }

        let mut text = String::new();
        for (title, script) in [
            ("Install", app.install.as_str()),
            ("Update", app.script(Action::Update)),
            ("Remove (runs as you first)", app.remove_user.as_str()),
            ("Remove", app.remove.as_str()),
            ("Installed version", app.version.as_str()),
        ] {
            if !script.trim().is_empty() {
                text.push_str(&format!("# {title}\n{}\n\n", script.trim()));
            }
        }
        text.push_str("# Helper functions available to every script\n");
        text.push_str(runner::PRELUDE);
        let buffer = gtk::TextBuffer::builder().text(text.as_str()).build();
        let view = gtk::TextView::builder().buffer(&buffer).editable(false).wrap_mode(gtk::WrapMode::None).build();
        view.style_context().add_class("ac-log");
        view.set_left_margin(8);
        view.set_top_margin(6);
        let label = gtk::Label::builder().label("<b>What Agent Center runs</b>").use_markup(true).xalign(0.0).build();
        let scroller = gtk::ScrolledWindow::builder().vexpand(true).shadow_type(gtk::ShadowType::In).build();
        scroller.add(&view);
        content.pack_start(&label, false, false, 6);
        content.pack_start(&scroller, true, true, 0);
        dialog.connect_response(|d, _| d.close());
        dialog.show_all();
    }
}

/// True when the Open button can start the app: it has a command, or one of its desktop entries
/// exists.
fn can_open(app: &App) -> bool {
    !app.command.is_empty() || app.desktop.iter().any(|id| desktop_entry(id).is_some())
}

/// The file of the desktop entry `id` ("firefox.desktop"), in ~/.local/share/applications or a
/// system data directory.
fn desktop_entry(id: &str) -> Option<PathBuf> {
    std::iter::once(glib::user_data_dir())
        .chain(glib::system_data_dirs())
        .map(|dir| dir.join("applications").join(id))
        .find(|p| p.is_file())
}

/// Starts the app: its command in a terminal, or its first desktop entry found.
fn launch(app: &App) -> anyhow::Result<()> {
    if app.command.is_empty() {
        let id = app
            .desktop
            .iter()
            .find(|id| desktop_entry(id).is_some())
            .ok_or_else(|| anyhow!("no desktop entry for {}", app.name))?;
        let info = gio::AppInfo::all()
            .into_iter()
            .find(|a| a.id().as_deref() == Some(id.as_str()))
            .ok_or_else(|| anyhow!("{id} cannot be started"))?;
        let ctx = gtk::gdk::Display::default().and_then(|d| d.app_launch_context());
        return Ok(info.launch(&[], ctx.as_ref())?);
    }
    // A login, interactive shell reads the files installers add to PATH (~/.profile, ~/.bashrc).
    // The shell stays open after the command ends.
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| s.starts_with('/') && !s.contains('\''))
        .unwrap_or_else(|| "/bin/bash".into());
    let script = format!("{}; exec '{shell}'", app.command);
    let home = catalog::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    // x-terminal-emulator is Debian's default terminal (xfce4-terminal on XFCE).
    for (terminal, flag) in [("x-terminal-emulator", "-e"), ("xfce4-terminal", "-x"), ("gnome-terminal", "--")] {
        let child = Command::new(terminal)
            .arg(flag)
            .args([shell.as_str(), "-l", "-i", "-c", &script])
            .current_dir(&home)
            .spawn();
        if let Ok(child) = child {
            reap(child);
            return Ok(());
        }
    }
    bail!("no terminal found. Install one with: sudo apt install xfce4-terminal")
}

/// Waits for a started program in the background, so it does not stay a zombie when it exits.
pub fn reap(mut child: Child) {
    thread::spawn(move || child.wait());
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// The app's icon from the icon theme, or a colored badge with its initials.
fn avatar(app: &App) -> gtk::Widget {
    const SIZE: i32 = 44;
    let tile_rgb = parse_color(&app.logo_background).unwrap_or((1.0, 1.0, 1.0));
    if let Some(tile) = app.logo_path().and_then(|path| logo_tile(path, tile_rgb, SIZE)) {
        return tile;
    }
    if !app.icon.is_empty() && gtk::IconTheme::default().is_some_and(|t| t.has_icon(&app.icon)) {
        let img = gtk::Image::from_icon_name(Some(&app.icon), gtk::IconSize::Dialog);
        img.set_pixel_size(SIZE);
        img.set_valign(gtk::Align::Center);
        return img.upcast();
    }
    let initials: String = if app.badge.is_empty() {
        let mut c = app.name.chars().filter(|c| c.is_alphanumeric());
        c.next().into_iter().chain(c.next().map(|c| c.to_ascii_lowercase())).collect()
    } else {
        app.badge.clone()
    };
    let (r, g, b) = parse_color(&app.color).unwrap_or((0.35, 0.4, 0.5));
    let area = gtk::DrawingArea::new();
    area.set_size_request(SIZE, SIZE);
    area.set_valign(gtk::Align::Center);
    area.connect_draw(move |_, cr| {
        let s = SIZE as f64;
        rounded_rect(cr, 0.0, 0.0, s, s, 10.0);
        cr.set_source_rgb(r, g, b);
        cr.fill().ok();
        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(if initials.chars().count() > 1 { 17.0 } else { 20.0 });
        if let Ok(ext) = cr.text_extents(&initials) {
            cr.move_to((s - ext.width()) / 2.0 - ext.x_bearing(), (s - ext.height()) / 2.0 - ext.y_bearing());
            // Dark text on light badges.
            let light = 0.299 * r + 0.587 * g + 0.114 * b > 0.65;
            if light {
                cr.set_source_rgb(0.1, 0.1, 0.1);
            } else {
                cr.set_source_rgb(1.0, 1.0, 1.0);
            }
            cr.show_text(&initials).ok();
        }
        glib::Propagation::Stop
    });
    area.upcast()
}

/// The vendor's logo on a rounded tile (white unless the app says otherwise), so dark and light
/// logos both read in any theme. None when the file cannot be loaded (no SVG loader, broken file).
fn logo_tile(path: PathBuf, (r, g, b): (f64, f64, f64), size: i32) -> Option<gtk::Widget> {
    let inner = size - 12;
    let first = Pixbuf::from_file_at_scale(&path, inner, inner, true).ok()?;
    // The logo rendered for the current scale factor, so it stays sharp on HiDPI screens.
    let cache = RefCell::new((1, first));
    let area = gtk::DrawingArea::new();
    area.set_size_request(size, size);
    area.set_valign(gtk::Align::Center);
    area.connect_draw(move |area, cr| {
        let s = size as f64;
        rounded_rect(cr, 0.5, 0.5, s - 1.0, s - 1.0, 10.0);
        cr.set_source_rgb(r, g, b);
        cr.fill_preserve().ok();
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
        cr.set_line_width(1.0);
        cr.stroke().ok();
        let sf = area.scale_factor().max(1);
        let mut cache = cache.borrow_mut();
        if cache.0 != sf
            && let Ok(pb) = Pixbuf::from_file_at_scale(&path, inner * sf, inner * sf, true)
        {
            *cache = (sf, pb);
        }
        let (scale, pb) = &*cache;
        let k = 1.0 / f64::from(*scale);
        let (w, h) = (f64::from(pb.width()) * k, f64::from(pb.height()) * k);
        cr.save().ok();
        cr.translate(((s - w) / 2.0).round(), ((s - h) / 2.0).round());
        cr.scale(k, k);
        cr.set_source_pixbuf(pb, 0.0, 0.0);
        cr.paint().ok();
        cr.restore().ok();
        glib::Propagation::Stop
    });
    Some(area.upcast())
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

fn parse_color(s: &str) -> Option<(f64, f64, f64)> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some((((n >> 16) & 255) as f64 / 255.0, ((n >> 8) & 255) as f64 / 255.0, (n & 255) as f64 / 255.0))
}

/// `agent-center askpass PROMPT`: the password window sudo opens (SUDO_ASKPASS). Prints the
/// password on stdout; exits with 1 when cancelled.
pub fn askpass(prompt: &str) -> i32 {
    if gtk::init().is_err() {
        return 1;
    }
    let dialog = gtk::Dialog::with_buttons(
        Some("Authentication Required"),
        None::<&gtk::Window>,
        gtk::DialogFlags::MODAL,
        &[("Cancel", gtk::ResponseType::Cancel), ("Authenticate", gtk::ResponseType::Ok)],
    );
    dialog.set_icon_name(Some("agent-center"));
    dialog.set_default_response(gtk::ResponseType::Ok);
    dialog.set_keep_above(true);
    dialog.set_position(gtk::WindowPosition::Center);
    let content = dialog.content_area();
    content.set_border_width(14);
    content.set_spacing(10);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = gtk::Image::from_icon_name(Some("dialog-password"), gtk::IconSize::Dialog);
    let text = gtk::Label::builder()
        .label(format!(
            "<b>Agent Center needs administrator rights</b>\n\n{}",
            esc(prompt.trim().trim_end_matches(':'))
        ))
        .use_markup(true)
        .xalign(0.0)
        .wrap(true)
        .build();
    head.pack_start(&icon, false, false, 0);
    head.pack_start(&text, true, true, 0);
    let entry = gtk::Entry::builder()
        .visibility(false)
        .activates_default(true)
        .input_purpose(gtk::InputPurpose::Password)
        .build();
    content.pack_start(&head, false, false, 0);
    content.pack_start(&entry, false, false, 0);
    dialog.show_all();
    entry.grab_focus();
    let ok = dialog.run() == gtk::ResponseType::Ok;
    let password = entry.text().to_string();
    // SAFETY: the dialog is not used after this point.
    unsafe { dialog.destroy() };
    if ok {
        println!("{password}");
        0
    } else {
        1
    }
}
