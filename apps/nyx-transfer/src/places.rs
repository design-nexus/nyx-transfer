//! The sidebar: places on this computer, saved servers, rclone remotes and workflows,
//! each group under a heading in its own colour. A place opens in the pane you're in.

use super::*;
use nyx_ui::{menu, sidebar};

struct Place {
    icon: &'static str,
    title: String,
    sub: String,
    path: PathBuf,
}

pub(super) fn build() -> (gtk::Box, gtk::Box) {
    let root = widgets::vbox(0);
    root.add_css_class("settings-navigation");
    root.add_css_class("transfer-sidebar");
    root.set_hexpand(false);
    let nav = widgets::vbox(1);
    nav.set_hexpand(false);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&nav)
        .build();
    root.append(&scroller);
    let foot = widgets::hbox(8);
    foot.add_css_class("nav-foot");
    foot.add_css_class("compact-hide");
    let icon = gtk::Image::from_icon_name("transfer-check-c-symbolic");
    icon.add_css_class("nav-foot-icon");
    icon.set_valign(gtk::Align::Start);
    foot.append(&icon);
    let text = widgets::label(
        "Transfers keep going when you close the window",
        "nav-foot-text",
    );
    text.set_wrap(true);
    text.set_hexpand(true);
    foot.append(&text);
    root.append(&foot);
    (root, nav)
}

/// The folder's parent, as the place's quiet second label ("Projects").
fn parent_name(path: &std::path::Path) -> String {
    path.parent()
        .filter(|parent| *parent != glib::home_dir())
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Folders bookmarked in Files (GTK's bookmarks), local ones only.
fn bookmarks() -> Vec<(PathBuf, Option<String>)> {
    let file = glib::user_config_dir().join("gtk-3.0/bookmarks");
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let (uri, label) = match line.split_once(' ') {
                Some((uri, label)) => (uri, Some(label.trim().to_owned())),
                None => (line.trim(), None),
            };
            let (path, _) = glib::filename_from_uri(uri).ok()?;
            path.is_dir()
                .then_some((path, label.filter(|l| !l.is_empty())))
        })
        .take(12)
        .collect()
}

fn local_places() -> Vec<Place> {
    let home = glib::home_dir();
    let mut places = vec![Place {
        icon: "transfer-home-symbolic",
        title: "Home".into(),
        sub: String::new(),
        path: home.clone(),
    }];
    let marked = bookmarks();
    let start = settings::start_folder();
    // The start folder, unless it sits in a bookmarked folder, which stands for it.
    let mut folders: Vec<(PathBuf, Option<String>)> = Vec::new();
    if !marked.iter().any(|(path, _)| start.starts_with(path)) {
        folders.push((start, None));
    }
    folders.extend(marked);
    for (path, label) in folders {
        if places.iter().any(|p| p.path == path) {
            continue;
        }
        let title = label.unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string())
        });
        places.push(Place {
            icon: "transfer-folder-symbolic",
            title,
            sub: parent_name(&path),
            path,
        });
    }
    for (dir, icon) in [
        (glib::UserDirectory::Downloads, "transfer-download-symbolic"),
        (glib::UserDirectory::Pictures, "transfer-image-symbolic"),
    ] {
        let Some(path) = glib::user_special_dir(dir).filter(|p| p.is_dir() && *p != home) else {
            continue;
        };
        if places.iter().any(|p| p.path == path) {
            continue;
        }
        places.push(Place {
            icon,
            title: path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
            sub: String::new(),
            path,
        });
    }
    places
}

fn nav_item(icon: &str, title: &str, sub: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    button.set_tooltip_text(Some(title));
    button.update_property(&[gtk::accessible::Property::Label(title)]);
    let content = widgets::hbox(12);
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class("nav-icon");
    content.append(&image);
    let label = widgets::label(title, "nav-label");
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class("compact-hide");
    content.append(&label);
    if !sub.is_empty() {
        let sub = widgets::label(sub, "nav-sub");
        sub.add_css_class("compact-hide");
        sub.set_valign(gtk::Align::Center);
        content.append(&sub);
    }
    button.set_child(Some(&content));
    button
}

/// A group's heading in its section's colour, and the icon rail's bar in the same colour.
fn group(nav: &gtk::Box, title: &str, section: &str, first: bool, add: widgets::Action<'_>) {
    let bar = gtk::Separator::new(gtk::Orientation::Horizontal);
    bar.add_css_class("nav-divider");
    bar.add_css_class("compact-show");
    bar.add_css_class(&format!("section-{section}"));
    bar.set_visible(false);
    nav.append(&bar);
    let row = match add {
        Some((tip, run)) => sidebar::group_label_with_add(title, first, tip, run),
        None => sidebar::group_label(title, first),
    };
    sidebar::set_section(&row, section);
    nav.append(&row);
}

/// A quiet row that adds the group's first item.
fn add_item(nav: &gtk::Box, title: &str, run: impl Fn() + 'static) {
    let button = nav_item("transfer-plus-symbolic", title, "");
    button.add_css_class("nav-add");
    button.connect_clicked(move |_| run());
    nav.append(&button);
}

fn open(state: &Rc<RefCell<State>>, location: Location) {
    let index = state.borrow().active_pane;
    navigate(state.clone(), index, location);
}

pub(super) fn render(state: &Rc<RefCell<State>>) {
    let (nav, sidebar_root, profiles, workflows) = {
        let s = state.borrow();
        (
            s.nav.clone(),
            s.sidebar.clone(),
            s.profiles.clone(),
            s.workflows.clone(),
        )
    };
    while let Some(child) = nav.first_child() {
        nav.remove(&child);
    }
    let mut items = Vec::new();

    group(&nav, "This computer", sidebar::ACCENT_SECTION, true, None);
    for place in local_places() {
        let button = nav_item(place.icon, &place.title, &place.sub);
        button.set_tooltip_text(Some(&place.path.display().to_string()));
        let location = Location::Local(place.path);
        let (s, l) = (state.clone(), location.clone());
        button.connect_clicked(move |_| open(&s, l.clone()));
        nav.append(&button);
        items.push((button, location));
    }

    let mut profile_group = |title: &str, section: &str, rclone: bool| {
        let s = state.clone();
        let (tip, add): (&str, Box<dyn Fn()>) = if rclone {
            (
                "Import an rclone remote",
                Box::new(move || request_remotes(s.clone())),
            )
        } else {
            (
                "Connect to a server",
                Box::new(move || {
                    let window = s.borrow().window.clone();
                    connection_dialog(&window, s.clone());
                }),
            )
        };
        group(&nav, title, section, false, Some((tip, add)));
        let mut any = false;
        for profile in profiles
            .iter()
            .filter(|p| (p.protocol == Protocol::Rclone) == rclone)
        {
            any = true;
            let (icon, sub) = if rclone {
                ("transfer-cloud-symbolic", "rclone")
            } else {
                (
                    "transfer-server-symbolic",
                    pane::protocol_short(&profile.protocol),
                )
            };
            let button = nav_item(icon, &profile.name, sub);
            let location = profile_home(profile);
            let (s, l) = (state.clone(), location.clone());
            button.connect_clicked(move |_| open(&s, l.clone()));
            nav.append(&button);
            items.push((button, location));
        }
        if !any {
            let s = state.clone();
            if rclone {
                add_item(&nav, "Import a remote", move || request_remotes(s.clone()));
            } else {
                add_item(&nav, "Connect to a server", move || {
                    let window = s.borrow().window.clone();
                    connection_dialog(&window, s.clone());
                });
            }
        }
    };
    profile_group("Servers", "swatch-green", false);
    profile_group("Cloud, from rclone", "swatch-violet", true);

    let s = state.clone();
    group(
        &nav,
        "Workflows",
        "swatch-amber",
        false,
        Some((
            "New workflow from the two panes",
            Box::new(move || {
                let window = s.borrow().window.clone();
                workflows::editor(&window, s.clone(), None);
            }),
        )),
    );
    for workflow in &workflows {
        let (icon, sub) = match workflow.mode {
            SyncMode::Copy => ("transfer-sync-symbolic", "Copy"),
            SyncMode::Mirror => ("transfer-sync-symbolic", "Mirror"),
            SyncMode::Bidirectional => ("transfer-both-symbolic", "Sync"),
        };
        let button = nav_item(icon, &workflow.name, sub);
        {
            let (s, w) = (state.clone(), workflow.clone());
            button.connect_clicked(move |_| {
                let window = s.borrow().window.clone();
                workflows::editor(&window, s.clone(), Some(w.clone()));
            });
        }
        {
            let (s, w) = (state.clone(), workflow.clone());
            menu::on_right_click(&button, move |widget, x, y| {
                let (s1, w1) = (s.clone(), w.clone());
                let (s2, id) = (s.clone(), w.id.clone());
                menu::popup_at(
                    widget,
                    x,
                    y,
                    vec![
                        menu::item("Edit and run…", move || {
                            let window = s1.borrow().window.clone();
                            workflows::editor(&window, s1.clone(), Some(w1.clone()));
                        }),
                        menu::separator(),
                        menu::item("Delete workflow", move || {
                            workflows::delete(&s2, id.clone())
                        })
                        .danger(),
                    ],
                );
            });
        }
        nav.append(&button);
    }
    if workflows.is_empty() {
        let s = state.clone();
        add_item(&nav, "New from the two panes", move || {
            let window = s.borrow().window.clone();
            workflows::editor(&window, s.clone(), None);
        });
    }

    state.borrow_mut().place_items = items;
    let compact = sidebar_root.has_css_class("compact");
    apply_compact(&sidebar_root, compact);
    mark(state);
}

/// Mark the place the active pane is in: the closest place holding its folder, or the
/// server it's on.
pub(super) fn mark(state: &Rc<RefCell<State>>) {
    let s = state.borrow();
    let current = &s.panes[s.active_pane].location;
    let closest =
        s.place_items
            .iter()
            .enumerate()
            .filter_map(|(i, (_, location))| match (location, current) {
                (Location::Local(place), Location::Local(here)) if here.starts_with(place) => {
                    Some((place.components().count(), i))
                }
                (
                    Location::Remote { profile_id: a, .. },
                    Location::Remote { profile_id: b, .. },
                ) if a == b => Some((0, i)),
                _ => None,
            })
            .max()
            .map(|(_, i)| i);
    for (i, (button, _)) in s.place_items.iter().enumerate() {
        if Some(i) == closest {
            button.add_css_class("active");
        } else {
            button.remove_css_class("active");
        }
    }
}

/// Icons only (`compact`), with a bar in each group's colour, or full rows.
pub(super) fn apply_compact(root: &gtk::Box, compact: bool) {
    if compact {
        root.add_css_class("compact");
    } else {
        root.remove_css_class("compact");
    }
    fn walk(widget: &gtk::Widget, compact: bool) {
        if widget.has_css_class("nav-item")
            && let Some(content) = widget.downcast_ref::<gtk::Button>().and_then(|b| b.child())
        {
            content.set_halign(if compact {
                gtk::Align::Center
            } else {
                gtk::Align::Fill
            });
        }
        if widget.has_css_class("compact-hide") {
            widget.set_visible(!compact);
        }
        if widget.has_css_class("compact-show") {
            widget.set_visible(compact);
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            walk(&c, compact);
            child = c.next_sibling();
        }
    }
    walk(root.upcast_ref(), compact);
}
