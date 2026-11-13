use gtk::prelude::*;
use gtk::{gio, glib};
use nyx_ui::widgets;
use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc, sync::Arc, time::Duration};
use transfer_core::{
    Auth, ConnectionProfile, FileEntry, JobSnapshot, Location, Protocol, SyncAction, SyncMode,
    SyncPreview, SyncWorkflow,
};
use transfer_storage as store;
mod icons;
mod pane;
mod places;
mod queue;
mod settings;
mod workflows;
use pane::{Pane, refresh, render_pane};

const APP_ID: &str = "io.github.design_nexus.nyx.Transfer";
/// Window width under which the sidebar becomes an icon rail and the panes drop their
/// Modified column.
const NARROW_WIDTH: i32 = nyx_ui::sidebar::NARROW_WIDTH;

#[derive(Clone)]
struct TabSnapshot {
    locations: [Location; 2],
    histories: [Vec<Location>; 2],
    positions: [usize; 2],
    filters: [String; 2],
    sorts: [u32; 2],
    descending: [bool; 2],
    hidden: [bool; 2],
}
struct State {
    window: gtk::ApplicationWindow,
    overlay: gtk::Overlay,
    panes: Vec<Pane>,
    tabs: Vec<TabSnapshot>,
    active_tab: usize,
    active_pane: usize,
    tab_strip: gtk::Box,
    app_title: Option<gtk::Widget>,
    narrow: bool,
    profiles: Vec<ConnectionProfile>,
    workflows: Vec<SyncWorkflow>,
    sidebar: gtk::Box,
    nav: gtk::Box,
    place_items: Vec<(gtk::Button, Location)>,
    queue: queue::Queue,
    jobs: HashMap<String, queue::JobView>,
    runtime: Arc<tokio::runtime::Runtime>,
    client: Arc<tokio::sync::OnceCell<transfer_ipc::Client>>,
    sender: std::sync::mpsc::Sender<Message>,
    sync_dialog: Option<SyncDialog>,
    workflow_dialog: Option<workflows::WorkflowDialog>,
}
struct SyncDialog {
    window: gtk::Window,
    request_id: String,
    source: Location,
    destination: Location,
    mode: SyncMode,
    preview: Option<SyncPreview>,
    report: gtk::TextView,
    status: gtk::Label,
    queue: gtk::Button,
}
type ListResult = Result<(Vec<FileEntry>, HashMap<String, usize>), String>;
enum Message {
    Listed(usize, Location, ListResult),
    Jobs(Result<Vec<JobSnapshot>, String>),
    QueueResult(Result<(), String>),
    ControlResult(Result<(), String>),
    Saved(Result<ConnectionProfile, String>),
    Remotes(Result<(PathBuf, Vec<transfer_rclone::AvailableRemote>), String>),
    SyncPreview(String, Result<SyncPreview, String>),
    WorkflowPreview(String, Result<transfer_core::WorkflowPreview, String>),
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .ok();
    nyx_core::app::init(nyx_core::app::App {
        slug: "nyx-transfer",
        name: "Transfer",
    });
    // Developer aid: a different id gives a separate instance (a test copy on a virtual
    // monitor), so it doesn't hand its arguments to the copy that's open.
    let id = std::env::var("NYX_TRANSFER_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
    let app = gtk::Application::builder().application_id(id).build();
    app.connect_activate(build);
    app.run()
}

/// Transfer's own symbolic icons (`transfer-*-symbolic`), written to the cache once and
/// added to the icon search path.
fn install_icons() {
    let dir = nyx_core::paths::cache_dir().join("icons");
    let _ = std::fs::create_dir_all(&dir);
    for (name, svg) in icons::ICONS {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*svg) {
            let _ = std::fs::write(&path, svg);
        }
    }
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&dir);
    }
}

/// A header button with an icon and a label, and a chevron when it opens a menu.
fn header_button(icon: &str, label: &str, tooltip: &str, menu: bool) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("header-action");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some(tooltip));
    let face = widgets::hbox(6);
    face.append(&gtk::Image::from_icon_name(icon));
    face.append(&gtk::Label::new(Some(label)));
    if menu {
        let chevron = gtk::Image::from_icon_name("transfer-chev-d-symbolic");
        chevron.add_css_class("chevron");
        face.append(&chevron);
    }
    button.set_child(Some(&face));
    button
}

fn find_by_class(root: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if root.has_css_class(class) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Some(found) = find_by_class(&c, class) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn build(app: &gtk::Application) {
    nyx_theme::use_design_2();
    nyx_theme::install(nyx_theme::Style {
        window_class: "transfer-window",
        base_skip: &[],
        css: include_str!("style.css"),
        no_glow_css: "",
        extra_colors: None,
        prefs: settings::style_prefs,
        set_theme: Some(settings::set_theme),
    });
    install_icons();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Transfer")
        .default_width(1280)
        .default_height(800)
        .build();
    window.add_css_class("transfer-window");
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_icon_name(Some(APP_ID));
    nyx_ui::keys::install(
        &window,
        nyx_ui::keys::Common {
            settings: Some(settings::open),
            close: false,
            ..Default::default()
        },
    );
    let profiles = store::load().unwrap_or_default();
    let saved_workflows = store::load_workflows().unwrap_or_default();
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .expect("Tokio runtime"),
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    // The header: the app, the sidebar's toggle and the tabs; Connect, Sync folders and
    // Settings at the right.
    let header = nyx_ui::chrome::app_header(&window, "transfer-xfer-symbolic", "Transfer");
    let app_title = find_by_class(header.root.upcast_ref(), "app-title");
    let collapse = widgets::bar_button(
        "transfer-sidebar-symbolic",
        "Show or hide the sidebar (Ctrl+B)",
    );
    header.start.append(&collapse);
    let tab_strip = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    tab_strip.add_css_class("transfer-tabs");
    let tab_scroll = gtk::ScrolledWindow::builder()
        .child(&tab_strip)
        .propagate_natural_width(true)
        .max_content_width(640)
        .build();
    tab_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
    tab_scroll.set_valign(gtk::Align::Center);
    tab_strip.set_valign(gtk::Align::Center);
    header.start.append(&tab_scroll);
    let connect = header_button(
        "transfer-server-symbolic",
        "Connect",
        "Connect to a server or import an rclone remote",
        true,
    );
    header.end.append(&connect);
    let sync_folders = header_button(
        "transfer-sync-symbolic",
        "Sync folders",
        "Compare the two panes' folders and copy what changed",
        false,
    );
    header.end.append(&sync_folders);
    header
        .end
        .append(&nyx_ui::chrome::settings_button(settings::open));
    let (sidebar, nav) = places::build();
    let panes_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    panes_row.add_css_class("transfer-panes");
    panes_row.set_homogeneous(true);
    panes_row.set_vexpand(true);
    let left = pane::make_pane(0);
    let right = pane::make_pane(1);
    panes_row.append(&left.root);
    panes_row.append(&right.root);
    let queue = queue::build();
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&panes_row);
    page.append(&queue.root);
    let frame = nyx_ui::frame::AppFrame::new(&header.root);
    frame.set_sidebar(&sidebar);
    frame.set_page(&page);
    frame.set_flush(true);
    let overlay = frame.install(&window);
    settings::attach(&overlay, &window);
    let panes = vec![left, right];
    let first_tab = TabSnapshot::from_panes(&panes);
    let state = Rc::new(RefCell::new(State {
        window: window.clone(),
        overlay,
        panes,
        tabs: vec![first_tab],
        active_tab: 0,
        active_pane: 0,
        tab_strip,
        app_title,
        narrow: false,
        profiles,
        workflows: saved_workflows,
        sidebar,
        nav,
        place_items: Vec::new(),
        queue,
        jobs: HashMap::new(),
        runtime,
        client: Arc::new(tokio::sync::OnceCell::new()),
        sender,
        sync_dialog: None,
        workflow_dialog: None,
    }));
    for index in 0..2 {
        pane::connect_pane(state.clone(), index);
        refresh(state.clone(), index);
    }
    pane::set_active_pane(&state, 0);
    places::render(&state);
    render_tabs(state.clone());
    queue::connect(&state);
    {
        let s = state.clone();
        collapse.connect_clicked(move |_| toggle_sidebar(&s));
    }
    {
        let s = state.clone();
        connect.connect_clicked(move |button| connect_menu(&s, button));
    }
    {
        let s = state.clone();
        let w = window.clone();
        sync_folders.connect_clicked(move |_| sync_dialog(&w, s.clone()));
    }
    {
        let s = state.clone();
        let w = window.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let ctrl = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let alt = modifiers.contains(gtk::gdk::ModifierType::ALT_MASK);
            if nyx_ui::settings_card::is_open() {
                if key == gtk::gdk::Key::Escape {
                    nyx_ui::settings_card::escape();
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }
            match key {
                gtk::gdk::Key::q if ctrl => w.close(),
                gtk::gdk::Key::w if ctrl => {
                    if s.borrow().tabs.len() > 1 {
                        let active = s.borrow().active_tab;
                        close_tab(s.clone(), active);
                    } else {
                        w.close();
                    }
                }
                gtk::gdk::Key::t if ctrl => new_tab(s.clone()),
                gtk::gdk::Key::b if ctrl => toggle_sidebar(&s),
                gtk::gdk::Key::Tab if ctrl => {
                    let next = {
                        let state = s.borrow();
                        (state.active_tab + 1) % state.tabs.len()
                    };
                    switch_tab(s.clone(), next);
                }
                gtk::gdk::Key::f if ctrl => {
                    let active = s.borrow().active_pane;
                    pane::focus_filter(&s, active);
                }
                gtk::gdk::Key::l if ctrl => {
                    let active = s.borrow().active_pane;
                    pane::focus_path(&s, active);
                }
                gtk::gdk::Key::h if ctrl => {
                    let active = s.borrow().active_pane;
                    pane::toggle_hidden(&s, active);
                }
                gtk::gdk::Key::r if ctrl => {
                    let active = s.borrow().active_pane;
                    refresh(s.clone(), active);
                }
                gtk::gdk::Key::Left if alt => {
                    let active = s.borrow().active_pane;
                    go_history(s.clone(), active, -1);
                }
                gtk::gdk::Key::Right if alt => {
                    let active = s.borrow().active_pane;
                    go_history(s.clone(), active, 1);
                }
                gtk::gdk::Key::F5 => {
                    refresh(s.clone(), 0);
                    refresh(s.clone(), 1);
                }
                gtk::gdk::Key::F1 => show_shortcuts(&w),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        window.add_controller(keys);
    }
    {
        let s = state.clone();
        let last = std::cell::Cell::new(0);
        window.add_tick_callback(move |w, _| {
            let width = if w.width() > 0 {
                w.width()
            } else {
                w.default_width()
            };
            if width != last.get() {
                last.set(width);
                layout(&s, width);
            }
            glib::ControlFlow::Continue
        });
    }
    {
        let s = state.clone();
        window.connect_realize(move |w| layout(&s, w.default_width()));
    }
    dev_aids(&state);
    request_jobs(state.clone());
    {
        let s = state.clone();
        glib::timeout_add_local(Duration::from_secs(1), move || {
            request_jobs(s.clone());
            glib::ControlFlow::Continue
        });
    }
    glib::timeout_add_local(Duration::from_millis(100), move || {
        while let Ok(message) = receiver.try_recv() {
            handle_message(state.clone(), message);
        }
        glib::ControlFlow::Continue
    });
    window.present();
}

/// Developer aids for checking a test copy on a virtual monitor, where nothing can be
/// clicked: NYX_TRANSFER_RIGHT opens a saved place (`name` or `name:path`) or a folder in
/// the right pane, NYX_TRANSFER_SELECT selects `name/name` in the left pane once it has
/// loaded, and NYX_TRANSFER_DRAWER=1 opens the transfers.
fn dev_aids(state: &Rc<RefCell<State>>) {
    if let Ok(place) = std::env::var("NYX_TRANSFER_RIGHT") {
        let (name, path) = place.split_once(':').unwrap_or((&place, ""));
        let location = state
            .borrow()
            .profiles
            .iter()
            .find(|p| p.name == name)
            .map(|p| Location::Remote {
                profile_id: p.id.clone(),
                path: path.trim_matches('/').to_owned(),
            })
            .unwrap_or_else(|| Location::Local(PathBuf::from(&place)));
        navigate(state.clone(), 1, location);
    }
    if std::env::var("NYX_TRANSFER_DRAWER").as_deref() == Ok("1") {
        queue::show_drawer(state, true);
    }
    if let Ok(names) = std::env::var("NYX_TRANSFER_SELECT") {
        let s = state.clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            let names: Vec<&str> = names.split('/').collect();
            let list = s.borrow().panes[0].list.clone();
            let mut row = list.first_child();
            while let Some(r) = row {
                if let Some(r) = r.downcast_ref::<gtk::ListBoxRow>()
                    && names.contains(&r.widget_name().as_str())
                {
                    list.select_row(Some(r));
                }
                row = r.next_sibling();
            }
        });
    }
}

/// Fit the window's width: the icon rail, the header's name, inactive tabs and the
/// panes' Modified column go below [`NARROW_WIDTH`].
fn layout(state: &Rc<RefCell<State>>, width: i32) {
    nyx_ui::settings_card::fit(&state.borrow().window);
    let narrow = width > 0 && width < NARROW_WIDTH;
    let compact = narrow || settings::get().sidebar_collapsed;
    let (sidebar, title, changed) = {
        let mut s = state.borrow_mut();
        let changed = s.narrow != narrow || !s.sidebar.has_css_class("sized");
        s.narrow = narrow;
        (s.sidebar.clone(), s.app_title.clone(), changed)
    };
    sidebar.add_css_class("sized");
    places::apply_compact(&sidebar, compact);
    let queue = state.borrow().queue.root.clone();
    if narrow {
        queue.add_css_class("narrow");
    } else {
        queue.remove_css_class("narrow");
    }
    if let Some(title) = title {
        title.set_visible(!narrow);
    }
    if changed {
        render_tabs(state.clone());
        for index in 0..2 {
            render_pane(state, index);
        }
    }
}

fn toggle_sidebar(state: &Rc<RefCell<State>>) {
    settings::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    let (sidebar, narrow) = {
        let s = state.borrow();
        (s.sidebar.clone(), s.narrow)
    };
    places::apply_compact(&sidebar, narrow || settings::get().sidebar_collapsed);
}

fn connect_menu(state: &Rc<RefCell<State>>, anchor: &gtk::Button) {
    use nyx_ui::menu;
    let (s1, s2) = (state.clone(), state.clone());
    menu::popup_below(
        anchor,
        vec![
            menu::item("Connect to a server…", move || {
                let window = s1.borrow().window.clone();
                connection_dialog(&window, s1.clone());
            })
            .icon("transfer-server-symbolic"),
            menu::item("Import an rclone remote…", move || request_remotes(s2.clone()))
                .icon("transfer-cloud-symbolic"),
        ],
    );
}

/// A short message over the window.
fn notify(state: &Rc<RefCell<State>>, text: &str) {
    let overlay = state.borrow().overlay.clone();
    nyx_ui::toast::show(&overlay, text);
}

fn show_shortcuts(window: &gtk::ApplicationWindow) {
    gtk::AlertDialog::builder()
        .message("Transfer shortcuts")
        .detail("Ctrl+T  New tab\nCtrl+W  Close tab\nCtrl+Tab  Next tab\nCtrl+B  Show or hide the sidebar\nCtrl+F  Filter active pane\nCtrl+L  Edit location\nAlt+Left/Right  Browse history\nCtrl+H  Toggle hidden files\nCtrl+R / F5  Refresh\nCtrl+Q  Close Transfer")
        .buttons(["Close"])
        .build()
        .show(Some(window));
}

impl TabSnapshot {
    fn from_panes(panes: &[Pane]) -> Self {
        Self {
            locations: [panes[0].location.clone(), panes[1].location.clone()],
            histories: [panes[0].history.clone(), panes[1].history.clone()],
            positions: [panes[0].history_position, panes[1].history_position],
            filters: [
                panes[0].filter.text().to_string(),
                panes[1].filter.text().to_string(),
            ],
            sorts: [panes[0].sort, panes[1].sort],
            descending: [panes[0].descending, panes[1].descending],
            hidden: [panes[0].show_hidden, panes[1].show_hidden],
        }
    }
}

fn tab_location_label(location: &Location, profiles: &[ConnectionProfile]) -> String {
    let label = match location {
        Location::Local(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "This computer".into()),
        Location::Remote { profile_id, .. } => profiles
            .iter()
            .find(|p| &p.id == profile_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Remote".into()),
    };
    if label.chars().count() > 18 {
        format!("{}…", label.chars().take(17).collect::<String>())
    } else {
        label
    }
}

fn render_tabs(state: Rc<RefCell<State>>) {
    let (strip, tabs, active, profiles, narrow) = {
        let s = state.borrow();
        (
            s.tab_strip.clone(),
            s.tabs.clone(),
            s.active_tab,
            s.profiles.clone(),
            s.narrow,
        )
    };
    while let Some(child) = strip.first_child() {
        strip.remove(&child);
    }
    for (index, tab) in tabs.iter().enumerate() {
        let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        group.add_css_class("transfer-tab");
        group.set_valign(gtk::Align::Center);
        if index == active {
            group.add_css_class("active");
        } else if narrow {
            continue;
        }
        let select = gtk::Button::new();
        let face = widgets::hbox(7);
        face.append(&gtk::Image::from_icon_name("transfer-xfer-symbolic"));
        let label = gtk::Label::new(Some(&format!(
            "{} ⇄ {}",
            tab_location_label(&tab.locations[0], &profiles),
            tab_location_label(&tab.locations[1], &profiles)
        )));
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        if narrow {
            label.set_max_width_chars(14);
        }
        face.append(&label);
        select.set_child(Some(&face));
        select.set_tooltip_text(Some(&format!(
            "{} ⇄ {}",
            location_name(&tab.locations[0], &profiles),
            location_name(&tab.locations[1], &profiles)
        )));
        let close = gtk::Button::from_icon_name("transfer-x-symbolic");
        close.add_css_class("tab-close");
        close.set_valign(gtk::Align::Center);
        close.set_tooltip_text(Some("Close tab (Ctrl+W)"));
        close.set_sensitive(tabs.len() > 1);
        group.append(&select);
        group.append(&close);
        strip.append(&group);
        {
            let s = state.clone();
            select.connect_clicked(move |_| switch_tab(s.clone(), index));
        }
        {
            let s = state.clone();
            close.connect_clicked(move |_| close_tab(s.clone(), index));
        }
    }
    let add = gtk::Button::from_icon_name("transfer-plus-symbolic");
    add.add_css_class("tab-add");
    add.set_valign(gtk::Align::Center);
    add.set_tooltip_text(Some("New tab (Ctrl+T)"));
    strip.append(&add);
    add.connect_clicked(move |_| new_tab(state.clone()));
}

fn save_active_tab(state: &mut State) {
    let snapshot = TabSnapshot::from_panes(&state.panes);
    state.tabs[state.active_tab] = snapshot;
}

fn restore_tab(state: Rc<RefCell<State>>, index: usize) {
    let (tab, filters) = {
        let mut s = state.borrow_mut();
        if index >= s.tabs.len() {
            return;
        }
        s.active_tab = index;
        (
            s.tabs[index].clone(),
            [s.panes[0].filter.clone(), s.panes[1].filter.clone()],
        )
    };
    {
        let mut s = state.borrow_mut();
        for pane_index in 0..2 {
            let pane = &mut s.panes[pane_index];
            pane.location = tab.locations[pane_index].clone();
            pane.history = tab.histories[pane_index].clone();
            pane.history_position = tab.positions[pane_index];
            pane.sort = tab.sorts[pane_index];
            pane.descending = tab.descending[pane_index];
            pane.show_hidden = tab.hidden[pane_index];
            pane.entries.clear();
            pane.item_counts.clear();
        }
    }
    for (pane_index, filter) in filters.iter().enumerate() {
        filter.set_text(&tab.filters[pane_index]);
        refresh(state.clone(), pane_index);
    }
    pane::set_active_pane(&state, 0);
    render_tabs(state);
}

fn switch_tab(state: Rc<RefCell<State>>, index: usize) {
    if index == state.borrow().active_tab {
        return;
    }
    save_active_tab(&mut state.borrow_mut());
    restore_tab(state, index);
}

fn new_tab(state: Rc<RefCell<State>>) {
    let index = {
        let mut s = state.borrow_mut();
        save_active_tab(&mut s);
        let copy = s.tabs[s.active_tab].clone();
        s.tabs.push(copy);
        s.tabs.len() - 1
    };
    restore_tab(state, index);
}

fn close_tab(state: Rc<RefCell<State>>, index: usize) {
    let next = {
        let mut s = state.borrow_mut();
        if s.tabs.len() <= 1 || index >= s.tabs.len() {
            return;
        }
        save_active_tab(&mut s);
        s.tabs.remove(index);
        if index < s.active_tab {
            s.active_tab -= 1;
        } else if index == s.active_tab {
            s.active_tab = s.active_tab.min(s.tabs.len() - 1);
        }
        s.active_tab
    };
    restore_tab(state, next);
}

fn go_history(state: Rc<RefCell<State>>, index: usize, offset: isize) {
    let changed = {
        let mut s = state.borrow_mut();
        let pane = &mut s.panes[index];
        let next = pane.history_position as isize + offset;
        if next < 0 || next >= pane.history.len() as isize {
            false
        } else {
            pane.history_position = next as usize;
            pane.location = pane.history[pane.history_position].clone();
            save_active_tab(&mut s);
            true
        }
    };
    if changed {
        refresh(state.clone(), index);
        render_tabs(state.clone());
        places::mark(&state);
    }
}

fn navigate(state: Rc<RefCell<State>>, index: usize, location: Location) {
    if let Err(error) = location.validate() {
        notify(&state, &error.to_string());
        return;
    }
    {
        let mut s = state.borrow_mut();
        let pane = &mut s.panes[index];
        if pane.location == location {
            drop(s);
            refresh(state, index);
            return;
        }
        pane.history.truncate(pane.history_position + 1);
        pane.history.push(location.clone());
        pane.history_position = pane.history.len() - 1;
        pane.location = location;
        save_active_tab(&mut s);
    }
    refresh(state.clone(), index);
    render_tabs(state.clone());
    places::mark(&state);
}

/// Where a saved server or remote opens.
fn profile_home(profile: &ConnectionProfile) -> Location {
    Location::Remote {
        profile_id: profile.id.clone(),
        path: profile.default_dir.trim_matches('/').to_owned(),
    }
}

fn location_name(location: &Location, profiles: &[ConnectionProfile]) -> String {
    match location {
        Location::Local(path) => path.display().to_string(),
        Location::Remote { profile_id, path } => {
            let name = profiles
                .iter()
                .find(|p| &p.id == profile_id)
                .map(|p| p.name.as_str())
                .unwrap_or("Remote");
            format!("{name}:{path}")
        }
    }
}

fn handle_message(state: Rc<RefCell<State>>, message: Message) {
    match message {
        Message::Listed(index, location, result) => {
            pane::listed(&state, index, location, result);
        }
        Message::Jobs(Ok(jobs)) => queue::render(&state, jobs),
        Message::Jobs(Err(e)) => queue::unavailable(&state, &e),
        Message::QueueResult(result) => match result {
            Ok(()) => request_jobs(state),
            Err(e) => notify(&state, &format!("Could not queue the transfer: {e}")),
        },
        Message::ControlResult(result) => match result {
            Ok(()) => request_jobs(state),
            Err(e) => notify(&state, &e),
        },
        Message::Saved(result) => match result {
            Ok(profile) => {
                let name = profile.name.clone();
                state.borrow_mut().profiles.push(profile);
                places::render(&state);
                notify(&state, &format!("Saved {name}"));
            }
            Err(e) => notify(&state, &e),
        },
        Message::Remotes(result) => match result {
            Ok((config, remotes)) => {
                let window = state.borrow().window.clone();
                import_remote_dialog(&window, state.clone(), config, remotes);
            }
            Err(e) => notify(&state, &e),
        },
        Message::SyncPreview(request_id, result) => {
            let mut s = state.borrow_mut();
            let Some(dialog) = s
                .sync_dialog
                .as_mut()
                .filter(|dialog| dialog.request_id == request_id)
            else {
                return;
            };
            match result {
                Ok(preview) => {
                    let copied = preview
                        .changes
                        .iter()
                        .filter(|c| c.action == SyncAction::Copy)
                        .count();
                    let updated = preview
                        .changes
                        .iter()
                        .filter(|c| c.action == SyncAction::Update)
                        .count();
                    let deleted = preview
                        .changes
                        .iter()
                        .filter(|c| c.action == SyncAction::Delete)
                        .count();
                    dialog.status.set_text(&format!(
                        "{copied} new · {updated} changed · {deleted} to delete"
                    ));
                    let report = preview
                        .changes
                        .iter()
                        .map(|c| {
                            let verb = match c.action {
                                SyncAction::Copy => "COPY",
                                SyncAction::Update => "UPDATE",
                                SyncAction::Delete => "DELETE",
                            };
                            format!("{verb}  {}", c.path)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    dialog.report.buffer().set_text(if report.is_empty() {
                        "No changes."
                    } else {
                        &report
                    });
                    dialog.queue.set_sensitive(!preview.changes.is_empty());
                    dialog.preview = Some(preview);
                }
                Err(e) => {
                    dialog.status.set_text(&e);
                    dialog.report.buffer().set_text("");
                    dialog.queue.set_sensitive(false);
                    dialog.preview = None;
                }
            }
        }
        Message::WorkflowPreview(request_id, result) => {
            workflows::handle_preview(&state, &request_id, result);
        }
    }
}

fn enqueue_job(state: Rc<RefCell<State>>, source: Location, destination: Location) {
    let (runtime, client, sender) = {
        let s = state.borrow();
        (s.runtime.clone(), s.client.clone(), s.sender.clone())
    };
    runtime.spawn(async move {
        let result = async {
            let client = client
                .get_or_try_init(transfer_ipc::Client::connect)
                .await
                .map_err(|e| e.to_string())?;
            client
                .enqueue(&source, &destination)
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        .await;
        let _ = sender.send(Message::QueueResult(result));
    });
}

fn request_jobs(state: Rc<RefCell<State>>) {
    let (runtime, client, sender) = {
        let s = state.borrow();
        (s.runtime.clone(), s.client.clone(), s.sender.clone())
    };
    runtime.spawn(async move {
        let result = async {
            let client = client
                .get_or_try_init(transfer_ipc::Client::connect)
                .await
                .map_err(|e| e.to_string())?;
            client.jobs().await.map_err(|e| e.to_string())
        }
        .await;
        let _ = sender.send(Message::Jobs(result));
    });
}

fn control_job(state: Rc<RefCell<State>>, id: String, action: &'static str) {
    let (runtime, client, sender) = {
        let s = state.borrow();
        (s.runtime.clone(), s.client.clone(), s.sender.clone())
    };
    runtime.spawn(async move {
        let result = async {
            let client = client
                .get_or_try_init(transfer_ipc::Client::connect)
                .await
                .map_err(|e| e.to_string())?;
            match action {
                "pause" => client.pause(&id).await,
                "resume" => client.resume(&id).await,
                "cancel" => client.cancel(&id).await,
                "retry" => client.retry(&id).await,
                "clear" => client.clear_completed().await,
                _ => return Err("Unknown queue action".to_owned()),
            }
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        .await;
        let _ = sender.send(Message::ControlResult(result));
    });
}


/// Keep a way out visible even when the connection or workflow form scrolls.
fn dismissible_dialog(window: &gtk::Window, label: &str) {
    let content = window.child();
    window.set_child(gtk::Widget::NONE);
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if let Some(content) = content {
        content.set_vexpand(true);
        layout.append(&content);
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    actions.set_margin_top(8);
    actions.set_margin_bottom(16);
    actions.set_margin_start(16);
    actions.set_margin_end(16);
    let dismiss = gtk::Button::with_label(label);
    dismiss.set_tooltip_text(Some(&format!("{label} (Esc)")));
    let weak = window.downgrade();
    dismiss.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    actions.append(&dismiss);
    layout.append(&actions);
    window.set_child(Some(&layout));
    let keys = gtk::EventControllerKey::new();
    let weak = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    window.add_controller(keys);
}

fn sync_dialog(parent: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) {
    if let Some(existing) = &state.borrow().sync_dialog {
        existing.window.present();
        return;
    }
    let (left, right, left_name, right_name) = {
        let s = state.borrow();
        let left = s.panes[0].location.clone();
        let right = s.panes[1].location.clone();
        let left_name = location_name(&left, &s.profiles);
        let right_name = location_name(&right, &s.profiles);
        (left, right, left_name, right_name)
    };
    let window = gtk::Window::builder()
        .title("Synchronize folders")
        .transient_for(parent)
        .modal(true)
        .default_width(660)
        .default_height(540)
        .build();
    window.add_css_class("transfer-window");
    window.add_css_class("transfer-dialog");
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.set_margin_top(16);
    form.set_margin_bottom(16);
    form.set_margin_start(16);
    form.set_margin_end(16);
    let source_label = gtk::Label::new(Some(&format!("Left: {left_name}\nRight: {right_name}")));
    source_label.set_xalign(0.0);
    source_label.set_wrap(true);
    form.append(&source_label);
    let direction = gtk::DropDown::from_strings(&["Left → right", "Right → left"]);
    dropdown_field(&form, "Direction", &direction);
    let mode = gtk::DropDown::from_strings(&[
        "Copy new and changed files",
        "Mirror (delete destination extras)",
    ]);
    dropdown_field(&form, "Mode", &mode);
    let preview_button = gtk::Button::with_label("Preview changes");
    form.append(&preview_button);
    let status = gtk::Label::new(Some("Choose a direction and preview the changes."));
    status.set_xalign(0.0);
    status.set_wrap(true);
    form.append(&status);
    let report = gtk::TextView::new();
    report.set_editable(false);
    report.set_cursor_visible(false);
    report.set_monospace(true);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&report)
        .vexpand(true)
        .build();
    form.append(&scroll);
    let queue = gtk::Button::with_label("Queue synchronization");
    queue.add_css_class("suggested-action");
    queue.set_sensitive(false);
    form.append(&queue);
    window.set_child(Some(&form));
    state.borrow_mut().sync_dialog = Some(SyncDialog {
        window: window.clone(),
        request_id: String::new(),
        source: left.clone(),
        destination: right.clone(),
        mode: SyncMode::Copy,
        preview: None,
        report: report.clone(),
        status: status.clone(),
        queue: queue.clone(),
    });
    for choice in [&direction, &mode] {
        let s = state.clone();
        choice.connect_selected_notify(move |_| {
            if let Some(dialog) = s.borrow_mut().sync_dialog.as_mut() {
                dialog.preview = None;
                dialog.queue.set_sensitive(false);
                dialog.status.set_text("Preview changes before queueing.");
                dialog.report.buffer().set_text("");
            }
        });
    }
    {
        let s = state.clone();
        preview_button.connect_clicked(move |_| {
            let (source, destination) = if direction.selected() == 0 {
                (left.clone(), right.clone())
            } else {
                (right.clone(), left.clone())
            };
            let sync_mode = if mode.selected() == 0 {
                SyncMode::Copy
            } else {
                SyncMode::Mirror
            };
            let request_id = uuid::Uuid::new_v4().to_string();
            let (runtime, client, sender) = {
                let mut state = s.borrow_mut();
                let Some(dialog) = state.sync_dialog.as_mut() else {
                    return;
                };
                dialog.request_id = request_id.clone();
                dialog.source = source.clone();
                dialog.destination = destination.clone();
                dialog.mode = sync_mode;
                dialog.preview = None;
                dialog.queue.set_sensitive(false);
                dialog.status.set_text("Comparing folders…");
                dialog.report.buffer().set_text("");
                (
                    state.runtime.clone(),
                    state.client.clone(),
                    state.sender.clone(),
                )
            };
            runtime.spawn(async move {
                let result = async {
                    let client = client
                        .get_or_try_init(transfer_ipc::Client::connect)
                        .await
                        .map_err(|e| e.to_string())?;
                    client
                        .preview_sync(&source, &destination, sync_mode)
                        .await
                        .map_err(|e| e.to_string())
                }
                .await;
                let _ = sender.send(Message::SyncPreview(request_id, result));
            });
        });
    }
    {
        let s = state.clone();
        let window = window.clone();
        queue.connect_clicked(move |_| {
            let chosen = {
                let state = s.borrow();
                state.sync_dialog.as_ref().and_then(|dialog| dialog.preview.as_ref().map(|preview| {
                    (dialog.source.clone(), dialog.destination.clone(), dialog.mode, preview.fingerprint.clone(),
                     preview.changes.iter().filter(|change| change.action == SyncAction::Delete).count())
                }))
            };
            let Some((source, destination, sync_mode, fingerprint, deletions)) = chosen else { return };
            let s = s.clone();
            let window = window.clone();
            glib::spawn_future_local(async move {
                if sync_mode == SyncMode::Mirror {
                    let prompt = gtk::AlertDialog::builder()
                        .message("Queue directory mirror?")
                        .detail(format!("This will delete {deletions} destination files shown in the preview. Changes to the plan will stop the job."))
                        .buttons(["Cancel", "Queue mirror"])
                        .cancel_button(0)
                        .default_button(0)
                        .build();
                    if prompt.choose_future(Some(&window)).await != Ok(1) { return; }
                }
                queue_sync_job(s, source, destination, sync_mode, fingerprint);
                window.close();
            });
        });
    }
    {
        let s = state.clone();
        window.connect_hide(move |_| {
            s.borrow_mut().sync_dialog = None;
        });
    }
    dismissible_dialog(&window, "Cancel");
    window.present();
}

fn queue_sync_job(
    state: Rc<RefCell<State>>,
    source: Location,
    destination: Location,
    mode: SyncMode,
    fingerprint: String,
) {
    let (runtime, client, sender) = {
        let s = state.borrow();
        (s.runtime.clone(), s.client.clone(), s.sender.clone())
    };
    runtime.spawn(async move {
        let result = async {
            let client = client
                .get_or_try_init(transfer_ipc::Client::connect)
                .await
                .map_err(|e| e.to_string())?;
            client
                .enqueue_sync(&source, &destination, mode, &fingerprint)
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        .await;
        let _ = sender.send(Message::QueueResult(result));
    });
}

fn request_remotes(state: Rc<RefCell<State>>) {
    let (runtime, sender) = {
        let s = state.borrow();
        (s.runtime.clone(), s.sender.clone())
    };
    runtime.spawn(async move {
        let config = transfer_rclone::default_config_path();
        let result = transfer_rclone::available_remotes(&config)
            .await
            .map(|remotes| (config, remotes))
            .map_err(|e| e.to_string());
        let _ = sender.send(Message::Remotes(result));
    });
}

fn import_remote_dialog(
    parent: &gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    config: PathBuf,
    remotes: Vec<transfer_rclone::AvailableRemote>,
) {
    if remotes.is_empty() {
        notify(&state, "No remotes found in the rclone config.");
        return;
    }
    let dialog = gtk::Window::builder()
        .title("Import rclone remote")
        .transient_for(parent)
        .modal(true)
        .default_width(420)
        .build();
    dialog.add_css_class("transfer-window");
    dialog.add_css_class("transfer-dialog");
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.set_margin_top(16);
    form.set_margin_bottom(16);
    form.set_margin_start(16);
    form.set_margin_end(16);
    let config_label = gtk::Label::new(Some(&format!("Config: {}", config.display())));
    config_label.set_xalign(0.0);
    config_label.set_wrap(true);
    form.append(&config_label);
    let names: Vec<String> = remotes
        .iter()
        .map(|r| format!("{} · {}", r.name, r.kind))
        .collect();
    let labels: Vec<&str> = names.iter().map(String::as_str).collect();
    let selector = gtk::DropDown::from_strings(&labels);
    dropdown_field(&form, "Remote", &selector);
    let name = field(&form, "Display name", "");
    name.set_text(&remotes[0].name);
    let directory = field(&form, "Starting directory (optional)", "");
    let detail = gtk::Label::new(Some(
        "Transfer uses this rclone config and its authentication. rclone may refresh provider tokens in that file.",
    ));
    detail.set_wrap(true);
    detail.set_xalign(0.0);
    form.append(&detail);
    let save = gtk::Button::with_label("Import remote");
    save.add_css_class("suggested-action");
    form.append(&save);
    dialog.set_child(Some(&form));
    {
        let names: Vec<String> = remotes.iter().map(|r| r.name.clone()).collect();
        let name = name.clone();
        selector.connect_selected_notify(move |selector| {
            if let Some(remote) = names.get(selector.selected() as usize) {
                name.set_text(remote);
            }
        });
    }
    let close = dialog.clone();
    save.connect_clicked(move |_| {
        let Some(remote) = remotes.get(selector.selected() as usize) else {
            return;
        };
        let display = name.text().trim().to_owned();
        let start = directory.text().trim_matches('/').to_owned();
        if display.is_empty() {
            notify(&state, "Enter a display name.");
            return;
        }
        let probe = Location::Remote {
            profile_id: "import".into(),
            path: if start.is_empty() {
                "probe".into()
            } else {
                start.clone()
            },
        };
        if probe.validate().is_err() {
            notify(&state, "Enter a valid starting directory.");
            return;
        }
        let profile = ConnectionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: display,
            protocol: Protocol::Rclone,
            host: String::new(),
            port: 0,
            username: String::new(),
            auth: Auth::Agent,
            default_dir: start,
            timeout_secs: 30,
            ssh_key_file: None,
            known_hosts_file: None,
            ca_cert: None,
            allow_plaintext: false,
            rclone_remote: Some(remote.name.clone()),
            rclone_config: Some(config.clone()),
        };
        let result = store::save(&profile)
            .map(|_| profile)
            .map_err(|e| e.to_string());
        let _ = state.borrow().sender.send(Message::Saved(result));
        close.close();
    });
    dismissible_dialog(&dialog, "Cancel");
    dialog.present();
}

fn connection_dialog(parent: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) {
    let dialog = gtk::Window::builder()
        .title("New connection")
        .transient_for(parent)
        .modal(true)
        .default_width(420)
        .default_height(650)
        .build();
    dialog.add_css_class("transfer-window");
    dialog.add_css_class("transfer-dialog");
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.set_margin_top(16);
    form.set_margin_bottom(16);
    form.set_margin_start(16);
    form.set_margin_end(16);
    let name = field(&form, "Display name", "My server");
    let protocol = gtk::DropDown::from_strings(&[
        "SFTP",
        "FTP",
        "FTPS (explicit)",
        "FTPS (implicit)",
        "WebDAV",
    ]);
    dropdown_field(&form, "Protocol", &protocol);
    let host = field(&form, "Host or WebDAV URL", "example.com");
    let port = field(&form, "Port", "22");
    port.set_text("22");
    let user = field(&form, "Username", "");
    let auth = gtk::DropDown::from_strings(&["SSH agent", "SSH key", "Password"]);
    dropdown_field(&form, "Authentication", &auth);
    let key = field(&form, "SSH key file (if selected)", "");
    let known = field(&form, "SSH known_hosts file", "");
    known.set_text(&format!(
        "{}/.ssh/known_hosts",
        std::env::var("HOME").unwrap_or_default()
    ));
    let trust = gtk::Button::with_label("Trust a new SSH host key…");
    form.append(&trust);
    let ca = field(&form, "Custom TLS CA certificate (optional)", "");
    let password = field(&form, "Password (if selected)", "");
    password.set_visibility(false);
    let directory = field(&form, "Starting directory", "");
    let plaintext = gtk::CheckButton::with_label("Allow plaintext FTP or HTTP WebDAV");
    form.append(&plaintext);
    let save = gtk::Button::with_label("Save connection");
    save.add_css_class("suggested-action");
    form.append(&save);
    dialog.set_child(Some(&gtk::ScrolledWindow::builder().child(&form).build()));
    {
        let port = port.clone();
        let auth = auth.clone();
        protocol.connect_selected_notify(move |choice| match choice.selected() {
            0 => {
                port.set_text("22");
                auth.set_selected(0);
            }
            1 | 2 => {
                port.set_text("21");
                auth.set_selected(2);
            }
            3 => {
                port.set_text("990");
                auth.set_selected(2);
            }
            _ => {
                port.set_text("443");
                auth.set_selected(2);
            }
        });
    }
    {
        let host = host.clone();
        let port = port.clone();
        let known = known.clone();
        let dialog = dialog.clone();
        trust.connect_clicked(move |_| {
            trust_host(
                &dialog,
                host.text().to_string(),
                port.text().to_string(),
                known.clone(),
            )
        });
    }
    let close = dialog.clone();
    save.connect_clicked(move |_| {
        let protocol = match protocol.selected() {
            0 => Protocol::Sftp,
            1 => Protocol::Ftp,
            2 => Protocol::FtpsExplicit,
            3 => Protocol::FtpsImplicit,
            _ => Protocol::Webdav,
        };
        let auth = match auth.selected() {
            0 => Auth::Agent,
            1 => Auth::KeyFile,
            _ => Auth::Password,
        };
        let host_value = host.text().trim().to_string();
        let Ok(port_value) = port.text().parse::<u16>() else {
            notify(&state, "Enter a valid port.");
            return;
        };
        if port_value == 0
            || host_value.starts_with('-')
            || host_value.contains(char::is_whitespace)
        {
            notify(&state, "Enter a valid host and port.");
            return;
        }
        let profile = ConnectionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.text().to_string(),
            protocol,
            host: host_value,
            port: port_value,
            username: user.text().to_string(),
            auth: auth.clone(),
            default_dir: directory.text().to_string(),
            timeout_secs: 30,
            ssh_key_file: if key.text().is_empty() {
                None
            } else {
                Some(PathBuf::from(key.text().as_str()))
            },
            known_hosts_file: if known.text().is_empty() {
                None
            } else {
                Some(PathBuf::from(known.text().as_str()))
            },
            ca_cert: if ca.text().is_empty() {
                None
            } else {
                Some(PathBuf::from(ca.text().as_str()))
            },
            allow_plaintext: plaintext.is_active(),
            rclone_remote: None,
            rclone_config: None,
        };
        if profile.name.trim().is_empty() || profile.host.trim().is_empty() {
            notify(&state, "Enter a name and host.");
            return;
        }
        if profile.protocol == Protocol::Ftp && !profile.allow_plaintext {
            notify(&state, "Confirm plaintext FTP before saving.");
            return;
        }
        if profile.protocol == Protocol::Webdav
            && !profile.host.starts_with("https://")
            && !profile.host.starts_with("http://")
        {
            notify(&state, "Enter a WebDAV URL starting with https:// or http://.");
            return;
        }
        if profile.protocol != Protocol::Webdav && profile.host.contains('/') {
            notify(&state, "Enter a host name without a path.");
            return;
        }
        if profile.host.contains('@') || profile.host.contains('?') || profile.host.contains('#') {
            notify(&state, "Put credentials in the password field, not the host URL.");
            return;
        }
        if profile.protocol == Protocol::Webdav
            && profile.host.starts_with("http://")
            && !profile.allow_plaintext
        {
            notify(&state, "Confirm plaintext HTTP WebDAV before saving.");
            return;
        }
        let secret = password.text().to_string();
        if auth == Auth::Password && (secret.is_empty() || secret.contains(['\n', '\r'])) {
            notify(&state, "Enter a password without line breaks.");
            return;
        }
        let (runtime, sender) = {
            let s = state.borrow();
            (s.runtime.clone(), s.sender.clone())
        };
        runtime.spawn(async move {
            let result = async {
                if auth == Auth::Password {
                    store::save_password(&profile.id, &secret)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                store::save(&profile).map_err(|e| e.to_string())?;
                Ok(profile)
            }
            .await;
            let _ = sender.send(Message::Saved(result));
        });
        close.close();
    });
    dismissible_dialog(&dialog, "Cancel");
    dialog.present();
}
fn trust_host(parent: &gtk::Window, host: String, port: String, known: gtk::Entry) {
    if host.trim().is_empty() {
        return;
    }
    let parent = parent.clone();
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || -> Result<(String, String), String> {
            use std::{
                io::Write,
                process::{Command, Stdio},
            };
            let output = Command::new("ssh-keyscan")
                .args(["-T", "5", "-p", &port, "-t", "ed25519,rsa,ecdsa", &host])
                .output()
                .map_err(|e| e.to_string())?;
            let keys = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
            if keys
                .lines()
                .all(|line| line.starts_with('#') || line.trim().is_empty())
            {
                return Err("No SSH host key was returned".into());
            }
            let mut fingerprint = Command::new("ssh-keygen")
                .args(["-lf", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            fingerprint
                .stdin
                .take()
                .ok_or("Cannot inspect key")?
                .write_all(keys.as_bytes())
                .map_err(|e| e.to_string())?;
            let shown = fingerprint.wait_with_output().map_err(|e| e.to_string())?;
            Ok((keys, String::from_utf8_lossy(&shown.stdout).to_string()))
        })
        .await;
        match result {
            Ok(Ok((keys, fingerprints))) => {
                let prompt = gtk::AlertDialog::builder()
                    .message("Trust this SSH host key?")
                    .detail(fingerprints)
                    .buttons(["Cancel", "Trust"])
                    .cancel_button(0)
                    .default_button(0)
                    .build();
                if prompt.choose_future(Some(&parent)).await == Ok(1) {
                    let path = nyx_core::paths::data_dir().join("ssh/known_hosts");
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    use std::io::Write;
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
                        if file.write_all(keys.as_bytes()).is_ok() {
                            known.set_text(&path.to_string_lossy());
                        }
                    }
                }
            }
            _ => gtk::AlertDialog::builder()
                .message("Could not inspect the SSH host key")
                .buttons(["Close"])
                .build()
                .show(Some(&parent)),
        }
    });
}
fn field(parent: &gtk::Box, label: &str, placeholder: &str) -> gtk::Entry {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.add_css_class("dim-label");
    group.append(&title);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    group.append(&entry);
    parent.append(&group);
    entry
}

fn dropdown_field(parent: &gtk::Box, label: &str, dropdown: &gtk::DropDown) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.add_css_class("dim-label");
    group.append(&title);
    dropdown.set_hexpand(true);
    group.append(dropdown);
    parent.append(&group);
}
