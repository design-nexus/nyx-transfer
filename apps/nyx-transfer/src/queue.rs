//! The transfers: a bar at the foot that shows what's copying even while the drawer is
//! closed, and the drawer that lists every transfer with what you can do with it.

use super::*;
use pane::{format_size, place_label};
use transfer_core::{JobOperation, JobStatus};

pub(super) struct Queue {
    pub(super) root: gtk::Box,
    bar: gtk::Box,
    bar_icon: gtk::Image,
    bar_title: gtk::Label,
    bar_progress: gtk::ProgressBar,
    bar_meta: gtk::Label,
    bar_more: gtk::Label,
    open: gtk::Button,
    drawer: gtk::Box,
    counts: gtk::Label,
    clear: gtk::Button,
    close: gtk::Button,
    list: gtk::Box,
    empty: gtk::Label,
}

pub(super) struct JobView {
    row: gtk::Box,
    icon: gtk::Image,
    title: gtk::Label,
    route: gtk::Label,
    progress: gtk::ProgressBar,
    meta: gtk::Label,
    actions: gtk::Box,
    /// The status the actions were made for.
    shown: Option<JobStatus>,
}

pub(super) fn build() -> Queue {
    let root = widgets::vbox(0);
    root.add_css_class("transfer-queue");

    let bar = widgets::hbox(10);
    bar.add_css_class("queue-bar");
    let bar_icon = gtk::Image::from_icon_name("transfer-xfer-symbolic");
    bar_icon.add_css_class("queue-bar-icon");
    bar.append(&bar_icon);
    let bar_title = widgets::label("", "queue-bar-title");
    bar_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    bar.append(&bar_title);
    let bar_progress = gtk::ProgressBar::new();
    bar_progress.add_css_class("queue-bar-progress");
    bar_progress.set_valign(gtk::Align::Center);
    bar.append(&bar_progress);
    let bar_meta = widgets::label("", "queue-meta");
    bar.append(&bar_meta);
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    let bar_more = widgets::label("", "queue-more");
    bar.append(&bar_more);
    let open = gtk::Button::new();
    open.add_css_class("flat");
    open.add_css_class("queue-toggle");
    open.set_tooltip_text(Some("Show the transfers"));
    let face = widgets::hbox(6);
    face.append(&gtk::Image::from_icon_name("transfer-chev-u-symbolic"));
    face.append(&gtk::Label::new(Some("Transfers")));
    open.set_child(Some(&face));
    bar.append(&open);
    root.append(&bar);

    let drawer = widgets::vbox(0);
    drawer.add_css_class("queue-drawer");
    drawer.set_visible(false);
    let head = widgets::hbox(10);
    head.add_css_class("queue-head");
    head.append(&widgets::label("Transfers", "queue-title"));
    let counts = widgets::label("", "queue-counts");
    counts.set_hexpand(true);
    head.append(&counts);
    let clear = gtk::Button::with_label("Clear finished");
    clear.add_css_class("flat");
    clear.set_tooltip_text(Some("Take finished transfers off the list"));
    head.append(&clear);
    let close = widgets::icon_button("transfer-chev-d-symbolic", "Hide transfers");
    head.append(&close);
    drawer.append(&head);
    let list = widgets::vbox(0);
    list.add_css_class("queue-list");
    let empty = widgets::label(
        "Nothing is copying. Select files and choose Copy to start.",
        "queue-empty",
    );
    list.append(&empty);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .propagate_natural_height(true)
        .max_content_height(280)
        .build();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
    drawer.append(&scroll);
    root.append(&drawer);

    Queue {
        root,
        bar,
        bar_icon,
        bar_title,
        bar_progress,
        bar_meta,
        bar_more,
        open,
        drawer,
        counts,
        clear,
        close,
        list,
        empty,
    }
}

pub(super) fn connect(state: &Rc<RefCell<State>>) {
    let s = state.borrow();
    let q = &s.queue;
    for (button, show) in [(&q.open, true), (&q.close, false)] {
        let state = state.clone();
        button.connect_clicked(move |_| show_drawer(&state, show));
    }
    let state = state.clone();
    q.clear
        .connect_clicked(move |_| control_job(state.clone(), String::new(), "clear"));
}

/// The drawer in place of the bar, or the bar again.
pub(super) fn show_drawer(state: &Rc<RefCell<State>>, show: bool) {
    let s = state.borrow();
    s.queue.drawer.set_visible(show);
    s.queue.bar.set_visible(!show);
}

fn rank(status: &JobStatus) -> u8 {
    match status {
        JobStatus::Running => 0,
        JobStatus::Queued | JobStatus::Paused => 1,
        JobStatus::Failed(_) => 2,
        JobStatus::Completed => 3,
        JobStatus::Cancelled => 4,
    }
}

/// The icon and verb for what a transfer does.
fn direction(job: &JobSnapshot) -> (&'static str, &'static str) {
    if !matches!(job.operation, JobOperation::Copy) {
        return ("transfer-sync-symbolic", "Syncing");
    }
    match (&job.source, &job.destination) {
        (Location::Local(_), Location::Remote { .. }) => ("transfer-arrow-u-symbolic", "Uploading"),
        (Location::Remote { .. }, Location::Local(_)) => {
            ("transfer-download-symbolic", "Downloading")
        }
        _ => ("transfer-xfer-symbolic", "Copying"),
    }
}

/// Where a copy goes from and to: the folders (or servers) the item sits in, so the
/// item's own name isn't repeated. Syncs name the two folders themselves.
fn ends(job: &JobSnapshot) -> (Location, Location) {
    if matches!(job.operation, JobOperation::Copy) {
        (job.source.parent(), job.destination.parent())
    } else {
        (job.source.clone(), job.destination.clone())
    }
}

fn route(job: &JobSnapshot, profiles: &[ConnectionProfile]) -> String {
    let (from, to) = ends(job);
    let mut text = format!(
        "{} → {}",
        place_label(&from, profiles),
        place_label(&to, profiles)
    );
    if let Location::Remote { path, .. } = &to {
        text.push_str(&format!(" · /{path}"));
    }
    text
}

fn eta_text(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{} h {} min", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{} min", seconds.div_ceil(60))
    } else {
        format!("{seconds} s")
    }
}

fn fraction(job: &JobSnapshot) -> Option<f64> {
    job.total_bytes
        .filter(|total| *total > 0)
        .map(|total| (job.bytes as f64 / total as f64).clamp(0.0, 1.0))
}

fn clock(seconds: i64) -> String {
    let Ok(time) = glib::DateTime::from_unix_local(seconds) else {
        return String::new();
    };
    let today = glib::DateTime::now_local()
        .map(|now| now.ymd() == time.ymd())
        .unwrap_or(false);
    time.format(if today { "at %H:%M" } else { "%-d %b" })
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn meta(job: &JobSnapshot) -> String {
    match &job.status {
        JobStatus::Running => {
            let mut parts = Vec::new();
            match job.total_bytes.filter(|t| *t > 0) {
                Some(total) => parts.push(format!(
                    "{} of {}",
                    format_size(job.bytes),
                    format_size(total)
                )),
                None if job.bytes > 0 => parts.push(format_size(job.bytes)),
                None => {}
            }
            if let Some(speed) = job.speed_bytes_per_sec.filter(|s| *s > 0.0) {
                parts.push(format!("{}/s", format_size(speed as u64)));
            }
            if let Some(eta) = job.eta_seconds {
                parts.push(format!("{} left", eta_text(eta)));
            }
            if parts.is_empty() {
                "Starting…".into()
            } else {
                parts.join(" · ")
            }
        }
        JobStatus::Queued => "Waiting".into(),
        JobStatus::Paused => "Paused".into(),
        JobStatus::Failed(error) => match job.retries {
            0 => format!("Failed: {error}"),
            1 => format!("Failed: {error}, after 1 try"),
            n => format!("Failed: {error}, after {n} tries"),
        },
        JobStatus::Completed => {
            let size = job.total_bytes.unwrap_or(job.bytes);
            let when = clock(job.updated_at);
            if size > 0 {
                format!("Done {when} · {}", format_size(size))
            } else {
                format!("Done {when}")
            }
        }
        JobStatus::Cancelled => "Cancelled".into(),
    }
}

fn job_actions(state: &Rc<RefCell<State>>, job: &JobSnapshot, actions: &gtk::Box) {
    while let Some(child) = actions.first_child() {
        actions.remove(&child);
    }
    let control = |icon: &str, tip: &str, action: &'static str| {
        let button = widgets::icon_button(icon, tip);
        let (s, id) = (state.clone(), job.id.clone());
        button.connect_clicked(move |_| control_job(s.clone(), id.clone(), action));
        button
    };
    match &job.status {
        JobStatus::Running => actions.append(&control("transfer-x-symbolic", "Cancel", "cancel")),
        JobStatus::Queued => {
            actions.append(&control("transfer-pause-symbolic", "Pause", "pause"));
            actions.append(&control("transfer-x-symbolic", "Cancel", "cancel"));
        }
        JobStatus::Paused => {
            actions.append(&control("transfer-play-symbolic", "Resume", "resume"));
            actions.append(&control("transfer-x-symbolic", "Cancel", "cancel"));
        }
        JobStatus::Failed(_) => {
            let retry = widgets::labeled_button("transfer-refresh-symbolic", "Try again");
            retry.add_css_class("retry");
            let (s, id) = (state.clone(), job.id.clone());
            retry.connect_clicked(move |_| control_job(s.clone(), id.clone(), "retry"));
            actions.append(&retry);
        }
        JobStatus::Completed => {
            let done = gtk::Image::from_icon_name("transfer-check-c-symbolic");
            done.add_css_class("done-mark");
            actions.append(&done);
        }
        JobStatus::Cancelled => {}
    }
}

fn new_view() -> JobView {
    let row = widgets::hbox(12);
    row.add_css_class("queue-row");
    let icon = gtk::Image::from_icon_name("transfer-xfer-symbolic");
    icon.add_css_class("queue-icon");
    row.append(&icon);
    let text = widgets::vbox(4);
    text.set_hexpand(true);
    let first = widgets::hbox(10);
    let title = widgets::label("", "queue-name");
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title.set_max_width_chars(48);
    first.append(&title);
    let route = widgets::label("", "queue-route");
    route.set_ellipsize(gtk::pango::EllipsizeMode::End);
    route.set_hexpand(true);
    first.append(&route);
    text.append(&first);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("queue-progress");
    progress.set_halign(gtk::Align::Start);
    text.append(&progress);
    let meta = widgets::label("", "queue-meta");
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&meta);
    row.append(&text);
    let actions = widgets::hbox(4);
    actions.set_valign(gtk::Align::Center);
    row.append(&actions);
    JobView {
        row,
        icon,
        title,
        route,
        progress,
        meta,
        actions,
        shown: None,
    }
}

pub(super) fn render(state: &Rc<RefCell<State>>, mut jobs: Vec<JobSnapshot>) {
    jobs.sort_by_key(|job| (rank(&job.status), job.created_at));
    let ids: std::collections::HashSet<_> = jobs.iter().map(|j| j.id.clone()).collect();
    {
        let mut s = state.borrow_mut();
        let gone: Vec<_> = s
            .jobs
            .keys()
            .filter(|id| !ids.contains(*id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(view) = s.jobs.remove(&id) {
                s.queue.list.remove(&view.row);
            }
        }
    }
    let mut previous: Option<gtk::Widget> = None;
    for job in &jobs {
        let mut s = state.borrow_mut();
        if !s.jobs.contains_key(&job.id) {
            let view = new_view();
            s.queue.list.append(&view.row);
            s.jobs.insert(job.id.clone(), view);
        }
        let list = s.queue.list.clone();
        let profiles = s.profiles.clone();
        let view = s.jobs.get_mut(&job.id).expect("job view");
        list.reorder_child_after(&view.row, previous.as_ref());
        previous = Some(view.row.clone().upcast());
        for class in ["run", "wait", "fail", "done"] {
            view.row.remove_css_class(class);
        }
        view.row.add_css_class(match job.status {
            JobStatus::Running => "run",
            JobStatus::Queued | JobStatus::Paused => "wait",
            JobStatus::Failed(_) => "fail",
            JobStatus::Completed | JobStatus::Cancelled => "done",
        });
        view.icon.set_icon_name(Some(direction(job).0));
        view.title.set_text(&job.name);
        view.route.set_text(&route(job, &profiles));
        view.meta.set_text(&meta(job));
        view.meta.set_tooltip_text(Some(&job.progress));
        let finished = matches!(job.status, JobStatus::Completed | JobStatus::Cancelled);
        view.progress.set_visible(!finished);
        match (fraction(job), &job.status) {
            (Some(f), _) => view.progress.set_fraction(f),
            (None, JobStatus::Running) => view.progress.pulse(),
            _ => view.progress.set_fraction(0.0),
        }
        let refresh_actions = view.shown.as_ref().is_none_or(|shown| {
            std::mem::discriminant(shown) != std::mem::discriminant(&job.status)
        });
        if refresh_actions {
            view.shown = Some(job.status.clone());
            let actions = view.actions.clone();
            drop(s);
            job_actions(state, job, &actions);
        }
    }
    render_summary(state, &jobs);
}

/// The drawer's counts and the bar: the running transfer, or the most pressing news.
fn render_summary(state: &Rc<RefCell<State>>, jobs: &[JobSnapshot]) {
    let s = state.borrow();
    let q = &s.queue;
    let count = |f: fn(&JobStatus) -> bool| jobs.iter().filter(|j| f(&j.status)).count();
    let running = count(|s| *s == JobStatus::Running);
    let waiting = count(|s| matches!(s, JobStatus::Queued | JobStatus::Paused));
    let failed = count(|s| matches!(s, JobStatus::Failed(_)));
    let done = count(|s| *s == JobStatus::Completed);
    q.empty.set_visible(jobs.is_empty());
    q.clear.set_sensitive(done > 0);
    let parts: Vec<String> = [
        (running, "running"),
        (waiting, "waiting"),
        (failed, "failed"),
        (done, "done"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    q.counts.set_text(&parts.join(" · "));

    let escape = |t: &str| glib::markup_escape_text(t).to_string();
    q.bar_icon.remove_css_class("bad");
    q.bar_icon.remove_css_class("dim");
    q.bar_title.set_tooltip_text(None);
    let mut more = Vec::new();
    if let Some(job) = jobs.iter().find(|j| j.status == JobStatus::Running) {
        let (icon, verb) = direction(job);
        let (_, to) = ends(job);
        q.bar_icon.set_icon_name(Some(icon));
        q.bar_title.set_markup(&format!(
            "<b>{verb} {}</b> to {}",
            escape(&job.name),
            escape(&place_label(&to, &s.profiles))
        ));
        q.bar_progress.set_visible(true);
        let mut meta = Vec::new();
        match fraction(job) {
            Some(f) => {
                q.bar_progress.set_fraction(f);
                meta.push(format!("{:.0}%", f * 100.0));
            }
            None => q.bar_progress.pulse(),
        }
        if let Some(eta) = job.eta_seconds {
            meta.push(format!("{} left", eta_text(eta)));
        }
        q.bar_meta.set_text(&meta.join(" · "));
        if running > 1 {
            more.push(format!("{} more running", running - 1));
        }
        if waiting > 0 {
            more.push(format!("{waiting} waiting"));
        }
    } else {
        q.bar_progress.set_visible(false);
        q.bar_meta.set_text("");
        let (icon, title) = if waiting > 0 {
            (
                "transfer-xfer-symbolic",
                format!("<b>{waiting} waiting</b>"),
            )
        } else if failed > 0 {
            q.bar_icon.add_css_class("bad");
            (
                "transfer-alert-symbolic",
                if failed == 1 {
                    "<b>A transfer failed</b>".to_owned()
                } else {
                    format!("<b>{failed} transfers failed</b>")
                },
            )
        } else if done > 0 {
            (
                "transfer-check-c-symbolic",
                "All transfers finished".to_owned(),
            )
        } else {
            q.bar_icon.add_css_class("dim");
            ("transfer-xfer-symbolic", "No transfers yet".to_owned())
        };
        q.bar_icon.set_icon_name(Some(icon));
        q.bar_title.set_markup(&title);
    }
    if failed > 0 && (running > 0 || waiting > 0) {
        more.push(format!("{failed} failed"));
    }
    q.bar_more.set_text(&more.join(" · "));
    if failed > 0 {
        q.bar_more.add_css_class("bad");
    } else {
        q.bar_more.remove_css_class("bad");
    }
}

/// The transfer service can't be reached: say so in the bar instead of the queue.
pub(super) fn unavailable(state: &Rc<RefCell<State>>, error: &str) {
    let s = state.borrow();
    let q = &s.queue;
    q.bar_icon.set_icon_name(Some("transfer-alert-symbolic"));
    q.bar_icon.add_css_class("bad");
    q.bar_title
        .set_markup("<b>The transfer service isn't running</b>");
    q.bar_title.set_tooltip_text(Some(error));
    q.bar_progress.set_visible(false);
    q.bar_meta.set_text("");
    q.bar_more.set_text("");
    q.counts.set_text("The transfer service isn't running");
}
