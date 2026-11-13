//! One of the two panes: where it is (a place button and the crumbs), its files, and the
//! bar that copies a selection to the other pane.

use super::*;
use nyx_ui::menu;
use transfer_core::StorageBackend;

#[derive(Clone, PartialEq)]
pub(super) enum Listing {
    Loading,
    Ready,
    Failed(String),
}

pub(super) struct Pane {
    pub(super) location: Location,
    pub(super) history: Vec<Location>,
    pub(super) history_position: usize,
    pub(super) entries: Vec<FileEntry>,
    /// What's in each local folder, for the Size column.
    pub(super) item_counts: HashMap<String, usize>,
    pub(super) listing: Listing,
    /// 0 name, 1 size, 2 modified.
    pub(super) sort: u32,
    pub(super) descending: bool,
    pub(super) show_hidden: bool,
    pub(super) root: gtk::Box,
    pub(super) filter: gtk::SearchEntry,
    pub(super) list: gtk::ListBox,
    place: gtk::Button,
    place_icon: gtk::Image,
    place_name: gtk::Label,
    place_proto: gtk::Label,
    back: gtk::Button,
    up: gtk::Button,
    filter_button: gtk::ToggleButton,
    view: gtk::Button,
    breadcrumb: gtk::Box,
    path: gtk::Entry,
    path_stack: gtk::Stack,
    filter_revealer: gtk::Revealer,
    heads: Vec<gtk::Button>,
    marks: Vec<gtk::Image>,
    body: gtk::Stack,
    message: gtk::Box,
    foot: gtk::Stack,
    info: gtk::Label,
    selected: gtk::Label,
    copy_path: gtk::Button,
    copy_to: gtk::Button,
}

/// The left pane opens on the start folder, the right one on Downloads, so the window
/// doesn't show one folder twice.
fn initial_location(index: usize) -> Location {
    let start = settings::start_folder();
    if index == 1
        && let Some(downloads) = glib::user_special_dir(glib::UserDirectory::Downloads)
        && downloads.is_dir()
        && downloads != start
    {
        return Location::Local(downloads);
    }
    Location::Local(start)
}

pub(super) fn make_pane(index: usize) -> Pane {
    let root = widgets::vbox(0);
    root.add_css_class("transfer-pane");
    if index == 1 {
        root.add_css_class("second");
    }
    root.set_hexpand(true);
    root.set_vexpand(true);

    let head = widgets::hbox(2);
    head.add_css_class("pane-head");
    let place = gtk::Button::new();
    place.add_css_class("place-button");
    place.set_tooltip_text(Some("Open another place in this pane"));
    let face = widgets::hbox(8);
    let place_icon = gtk::Image::from_icon_name("transfer-monitor-symbolic");
    place_icon.add_css_class("place-icon");
    face.append(&place_icon);
    let place_name = widgets::label("This computer", "place-name");
    place_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    face.append(&place_name);
    let place_proto = widgets::label("", "place-proto");
    place_proto.set_valign(gtk::Align::Center);
    place_proto.set_visible(false);
    face.append(&place_proto);
    let chevron = gtk::Image::from_icon_name("transfer-chev-d-symbolic");
    chevron.add_css_class("chevron");
    face.append(&chevron);
    place.set_child(Some(&face));
    head.append(&place);
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    head.append(&spacer);
    let back = widgets::icon_button("transfer-chev-l-symbolic", "Back (Alt ←)");
    let up = widgets::icon_button("transfer-arrow-u-symbolic", "Up a folder");
    let filter_button = gtk::ToggleButton::new();
    filter_button.set_icon_name("transfer-filter-symbolic");
    filter_button.add_css_class("flat");
    filter_button.add_css_class("icon-button");
    filter_button.set_valign(gtk::Align::Center);
    filter_button.set_tooltip_text(Some("Filter (Ctrl F)"));
    filter_button.update_property(&[gtk::accessible::Property::Label("Filter")]);
    let view = widgets::icon_button("transfer-sort-symbolic", "Sort, hidden files and more");
    for button in [
        back.upcast_ref::<gtk::Widget>(),
        up.upcast_ref(),
        filter_button.upcast_ref(),
        view.upcast_ref(),
    ] {
        head.append(button);
    }
    root.append(&head);

    let breadcrumb = widgets::hbox(0);
    breadcrumb.add_css_class("pane-crumbs");
    let crumbs_scroll = gtk::ScrolledWindow::builder()
        .child(&breadcrumb)
        .hexpand(true)
        .build();
    crumbs_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
    let path = gtk::Entry::new();
    path.add_css_class("pane-path-entry");
    path.set_hexpand(true);
    let path_stack = gtk::Stack::new();
    path_stack.add_css_class("pane-path");
    path_stack.set_vhomogeneous(false);
    path_stack.add_named(&crumbs_scroll, Some("crumbs"));
    path_stack.add_named(&path, Some("entry"));
    path_stack.set_visible_child_name("crumbs");
    root.append(&path_stack);

    let filter_row = widgets::hbox(4);
    filter_row.add_css_class("pane-filter");
    let filter = gtk::SearchEntry::new();
    filter.set_placeholder_text(Some("Filter this folder"));
    filter.set_hexpand(true);
    filter_row.append(&filter);
    let filter_revealer = gtk::Revealer::new();
    filter_revealer.set_child(Some(&filter_row));
    root.append(&filter_revealer);

    let heads_row = widgets::hbox(10);
    heads_row.add_css_class("file-heads");
    let mut heads = Vec::new();
    let mut marks = Vec::new();
    for (column, title) in ["Name", "Size", "Modified"].into_iter().enumerate() {
        let button = gtk::Button::new();
        button.add_css_class("flat");
        button.add_css_class("file-head");
        button.set_tooltip_text(Some(&format!("Sort by {}", title.to_lowercase())));
        let face = widgets::hbox(4);
        face.append(&gtk::Label::new(Some(title)));
        let mark = gtk::Image::from_icon_name("transfer-chev-u-symbolic");
        mark.add_css_class("sort-mark");
        face.append(&mark);
        button.set_child(Some(&face));
        match column {
            0 => button.set_hexpand(true),
            1 => {
                face.set_halign(gtk::Align::End);
                button.add_css_class("size-column");
            }
            _ => button.add_css_class("modified-column"),
        }
        heads_row.append(&button);
        heads.push(button);
        marks.push(mark);
    }
    root.append(&heads_row);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Multiple);
    list.set_activate_on_single_click(false);
    list.add_css_class("file-list");
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
    let message = widgets::vbox(0);
    message.set_vexpand(true);
    let body = gtk::Stack::new();
    body.set_vexpand(true);
    body.add_named(&scroll, Some("list"));
    body.add_named(&message, Some("message"));
    root.append(&body);

    let info_bar = widgets::hbox(0);
    info_bar.add_css_class("pane-foot");
    let info = widgets::label("", "pane-info");
    info.set_ellipsize(gtk::pango::EllipsizeMode::End);
    info_bar.append(&info);
    let selection_bar = widgets::hbox(6);
    selection_bar.add_css_class("selection-bar");
    let selected = widgets::label("", "selection-text");
    selected.set_hexpand(true);
    selected.set_ellipsize(gtk::pango::EllipsizeMode::End);
    selection_bar.append(&selected);
    let copy_path = gtk::Button::with_label("Copy path");
    copy_path.add_css_class("flat");
    copy_path.set_tooltip_text(Some("Copy the selected items' paths"));
    selection_bar.append(&copy_path);
    let copy_to = gtk::Button::new();
    copy_to.add_css_class("suggested-action");
    copy_to.add_css_class("copy-to");
    selection_bar.append(&copy_to);
    let foot = gtk::Stack::new();
    foot.set_vhomogeneous(false);
    foot.add_named(&info_bar, Some("info"));
    foot.add_named(&selection_bar, Some("selection"));
    root.append(&foot);

    let location = initial_location(index);
    let settings = settings::get();
    Pane {
        location: location.clone(),
        history: vec![location],
        history_position: 0,
        entries: Vec::new(),
        item_counts: HashMap::new(),
        listing: Listing::Loading,
        sort: settings::sort_index(),
        descending: false,
        show_hidden: settings.show_hidden,
        root,
        filter,
        list,
        place,
        place_icon,
        place_name,
        place_proto,
        back,
        up,
        filter_button,
        view,
        breadcrumb,
        path,
        path_stack,
        filter_revealer,
        heads,
        marks,
        body,
        message,
        foot,
        info,
        selected,
        copy_path,
        copy_to,
    }
}

pub(super) fn connect_pane(state: Rc<RefCell<State>>, index: usize) {
    let s = state.borrow();
    let p = &s.panes[index];
    {
        let state = state.clone();
        p.place
            .connect_clicked(move |button| place_menu(&state, index, button));
    }
    {
        let state = state.clone();
        p.back
            .connect_clicked(move |_| go_history(state.clone(), index, -1));
    }
    {
        let state = state.clone();
        p.up.connect_clicked(move |_| {
            let next = state.borrow().panes[index].location.parent();
            navigate(state.clone(), index, next);
        });
    }
    {
        let revealer = p.filter_revealer.clone();
        let entry = p.filter.clone();
        p.filter_button.connect_toggled(move |button| {
            revealer.set_reveal_child(button.is_active());
            if button.is_active() {
                entry.grab_focus();
            } else {
                entry.set_text("");
            }
        });
    }
    {
        let state = state.clone();
        p.filter
            .connect_search_changed(move |_| render_pane(&state, index));
    }
    {
        let button = p.filter_button.clone();
        p.filter
            .connect_stop_search(move |_| button.set_active(false));
    }
    {
        let state = state.clone();
        p.view
            .connect_clicked(move |button| view_menu(&state, index, button));
    }
    for (column, head) in p.heads.iter().enumerate() {
        let state = state.clone();
        head.connect_clicked(move |_| {
            {
                let mut s = state.borrow_mut();
                let pane = &mut s.panes[index];
                if pane.sort == column as u32 {
                    pane.descending = !pane.descending;
                } else {
                    pane.sort = column as u32;
                    pane.descending = false;
                }
            }
            render_pane(&state, index);
        });
    }
    {
        let state = state.clone();
        p.path.connect_activate(move |entry| {
            let text = entry.text().to_string();
            let location = match &state.borrow().panes[index].location {
                Location::Local(_) => Location::Local(PathBuf::from(text)),
                Location::Remote { profile_id, .. } => Location::Remote {
                    profile_id: profile_id.clone(),
                    path: text.trim_matches('/').to_owned(),
                },
            };
            state.borrow().panes[index]
                .path_stack
                .set_visible_child_name("crumbs");
            navigate(state.clone(), index, location);
        });
    }
    {
        let stack = p.path_stack.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                stack.set_visible_child_name("crumbs");
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        p.path.add_controller(keys);
    }
    {
        let stack = p.path_stack.clone();
        p.path.connect_has_focus_notify(move |entry| {
            if !entry.has_focus() {
                stack.set_visible_child_name("crumbs");
            }
        });
    }
    {
        // Double-click beside the crumbs to type a location.
        let state = state.clone();
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, presses, _, _| {
            if presses == 2 {
                focus_path(&state, index);
            }
        });
        p.breadcrumb.add_controller(click);
    }
    {
        let state = state.clone();
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.set_button(0);
        click.connect_pressed(move |_, _, _, _| set_active_pane(&state, index));
        p.root.add_controller(click);
    }
    {
        let state = state.clone();
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(move |_| set_active_pane(&state, index));
        p.root.add_controller(focus);
    }
    {
        let state = state.clone();
        p.list.connect_row_activated(move |_, row| {
            let name = row.widget_name().to_string();
            let (current, is_dir) = {
                let s = state.borrow();
                let pane = &s.panes[index];
                (
                    pane.location.clone(),
                    pane.entries.iter().any(|e| e.name == name && e.is_dir),
                )
            };
            if is_dir {
                match current.child(&name) {
                    Ok(next) => navigate(state.clone(), index, next),
                    Err(e) => notify(&state, &e.to_string()),
                }
            }
        });
    }
    {
        let state = state.clone();
        p.list
            .connect_selected_rows_changed(move |_| update_selection(&state, index));
    }
    {
        let state = state.clone();
        p.copy_to
            .connect_clicked(move |_| queue_selected(state.clone(), index, 1 - index));
    }
    {
        let state = state.clone();
        p.copy_path.connect_clicked(move |_| {
            let text = {
                let s = state.borrow();
                let pane = &s.panes[index];
                selected_names(pane)
                    .iter()
                    .filter_map(|name| pane.location.child(name).ok())
                    .map(|location| location_name(&location, &s.profiles))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
                notify(&state, "Copied the path");
            }
        });
    }
    let list = p.list.clone();
    drop(s);
    install_drop_targets(state, index, &list);
}

/// The pane you're in: its place button and a line at its top in the accent, and the
/// sidebar marks where it is.
pub(super) fn set_active_pane(state: &Rc<RefCell<State>>, index: usize) {
    let changed = {
        let mut s = state.borrow_mut();
        let changed = s.active_pane != index || !s.panes[index].root.has_css_class("focus");
        s.active_pane = index;
        for (i, pane) in s.panes.iter().enumerate() {
            if i == index {
                pane.root.add_css_class("focus");
            } else {
                pane.root.remove_css_class("focus");
            }
        }
        changed
    };
    if changed {
        places::mark(state);
    }
}

pub(super) fn focus_filter(state: &Rc<RefCell<State>>, index: usize) {
    let (button, entry) = {
        let s = state.borrow();
        (
            s.panes[index].filter_button.clone(),
            s.panes[index].filter.clone(),
        )
    };
    button.set_active(true);
    entry.grab_focus();
}

pub(super) fn focus_path(state: &Rc<RefCell<State>>, index: usize) {
    let s = state.borrow();
    let pane = &s.panes[index];
    pane.path_stack.set_visible_child_name("entry");
    pane.path.grab_focus();
    pane.path.select_region(0, -1);
}

pub(super) fn toggle_hidden(state: &Rc<RefCell<State>>, index: usize) {
    {
        let mut s = state.borrow_mut();
        let pane = &mut s.panes[index];
        pane.show_hidden = !pane.show_hidden;
    }
    render_pane(state, index);
}

pub(super) fn refresh(state: Rc<RefCell<State>>, index: usize) {
    let (location, profiles, runtime, sender) = {
        let mut s = state.borrow_mut();
        let pane = &mut s.panes[index];
        pane.listing = Listing::Loading;
        pane.back.set_sensitive(pane.history_position > 0);
        pane.up
            .set_sensitive(pane.location.parent() != pane.location);
        (
            pane.location.clone(),
            s.profiles.clone(),
            s.runtime.clone(),
            s.sender.clone(),
        )
    };
    render_place(&state, index);
    render_breadcrumb(&state, index);
    update_selection(&state, index);
    runtime.spawn(async move {
        let selected: Vec<_> = profiles
            .into_iter()
            .filter(
                |p| matches!(&location, Location::Remote { profile_id, .. } if profile_id == &p.id),
            )
            .collect();
        let result = async {
            let credentials = store::with_credentials(&selected)
                .await
                .map_err(|e| e.to_string())?;
            let entries = transfer_rclone::RcloneBackend::new(credentials)
                .list(&location)
                .await
                .map_err(|e| e.to_string())?;
            let counts = match &location {
                Location::Local(path) => {
                    let path = path.clone();
                    let folders: Vec<String> = entries
                        .iter()
                        .filter(|e| e.is_dir)
                        .map(|e| e.name.clone())
                        .collect();
                    tokio::task::spawn_blocking(move || count_items(&path, &folders))
                        .await
                        .unwrap_or_default()
                }
                Location::Remote { .. } => HashMap::new(),
            };
            Ok((entries, counts))
        }
        .await;
        let _ = sender.send(Message::Listed(index, location, result));
    });
}

/// How many visible items each folder holds.
fn count_items(parent: &std::path::Path, folders: &[String]) -> HashMap<String, usize> {
    folders
        .iter()
        .filter_map(|name| {
            let entries = std::fs::read_dir(parent.join(name)).ok()?;
            let count = entries
                .filter_map(Result::ok)
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count();
            Some((name.clone(), count))
        })
        .collect()
}

pub(super) fn listed(
    state: &Rc<RefCell<State>>,
    index: usize,
    location: Location,
    result: ListResult,
) {
    {
        let mut s = state.borrow_mut();
        let pane = &mut s.panes[index];
        if pane.location != location {
            return;
        }
        match result {
            Ok((entries, counts)) => {
                pane.entries = entries;
                pane.item_counts = counts;
                pane.listing = Listing::Ready;
                pane.path.set_text(match &location {
                    Location::Local(p) => p.to_str().unwrap_or(""),
                    Location::Remote { path, .. } => path,
                });
            }
            Err(e) => {
                pane.entries.clear();
                pane.item_counts.clear();
                pane.listing = Listing::Failed(e);
            }
        }
    }
    render_pane(state, index);
}

fn visible_entries(pane: &Pane) -> Vec<&FileEntry> {
    let filter = pane.filter.text().to_lowercase();
    let mut entries: Vec<_> = pane
        .entries
        .iter()
        .filter(|e| {
            (pane.show_hidden || !e.name.starts_with('.'))
                && e.name.to_lowercase().contains(&filter)
        })
        .collect();
    entries.sort_by(|a, b| {
        let order = match pane.sort {
            1 => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)).then_with(|| {
                pane.item_counts
                    .get(&a.name)
                    .cmp(&pane.item_counts.get(&b.name))
            }),
            2 => a.modified.cmp(&b.modified),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        b.is_dir.cmp(&a.is_dir).then(if pane.descending {
            order.reverse()
        } else {
            order
        })
    });
    entries
}

fn selected_names(pane: &Pane) -> Vec<String> {
    pane.list
        .selected_rows()
        .iter()
        .map(|r| r.widget_name().to_string())
        .collect()
}

pub(super) fn render_pane(state: &Rc<RefCell<State>>, index: usize) {
    {
        let s = state.borrow();
        let narrow = s.narrow;
        let pane = &s.panes[index];
        while let Some(child) = pane.list.first_child() {
            pane.list.remove(&child);
        }
        for (column, mark) in pane.marks.iter().enumerate() {
            mark.set_visible(column as u32 == pane.sort);
            mark.set_icon_name(Some(if pane.descending {
                "transfer-chev-d-symbolic"
            } else {
                "transfer-chev-u-symbolic"
            }));
        }
        pane.heads[2].set_visible(!narrow);
        let entries = visible_entries(pane);
        for entry in &entries {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&entry.name);
            let line = widgets::hbox(10);
            line.add_css_class("file-row");
            let icon = gtk::Image::from_icon_name(file_icon(entry));
            icon.add_css_class("file-icon");
            if entry.is_dir {
                icon.add_css_class("folder");
            }
            line.append(&icon);
            let name = widgets::label(&entry.name, "file-name");
            name.set_hexpand(true);
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            line.append(&name);
            let size_text = if entry.is_dir {
                pane.item_counts
                    .get(&entry.name)
                    .map(|n| items_text(*n))
                    .unwrap_or_default()
            } else {
                entry.size.map(format_size).unwrap_or_default()
            };
            let size = widgets::label(&size_text, "file-size");
            size.add_css_class("size-column");
            size.set_xalign(1.0);
            line.append(&size);
            let modified =
                widgets::label(&format_modified(entry.modified.as_deref()), "file-modified");
            modified.add_css_class("modified-column");
            modified.set_visible(!narrow);
            line.append(&modified);
            row.set_child(Some(&line));
            install_row_actions(state.clone(), index, &row, &entry.name, entry.is_dir);
            pane.list.append(&row);
        }
        while let Some(child) = pane.message.first_child() {
            pane.message.remove(&child);
        }
        let message = match &pane.listing {
            Listing::Failed(error) => {
                let s = state.clone();
                Some(widgets::empty_state(
                    "transfer-alert-symbolic",
                    "Couldn't open this folder",
                    &glib::markup_escape_text(error),
                    Some(("Try again", Box::new(move || refresh(s.clone(), index)))),
                ))
            }
            Listing::Ready if entries.is_empty() && !pane.filter.text().is_empty() => {
                Some(widgets::empty_state(
                    "transfer-filter-symbolic",
                    "Nothing matches the filter",
                    "",
                    None,
                ))
            }
            Listing::Ready if entries.is_empty() => Some(widgets::empty_state(
                "transfer-folder-symbolic",
                "This folder is empty",
                "Drop files here to copy them in.",
                None,
            )),
            _ => None,
        };
        match message {
            Some(message) => {
                pane.message.append(&message);
                pane.body.set_visible_child_name("message");
            }
            None => pane.body.set_visible_child_name("list"),
        }
    }
    update_selection(state, index);
}

/// The pane's foot: how many items and the connection, or, with files selected, what's
/// selected and a button that copies it to the other pane.
fn update_selection(state: &Rc<RefCell<State>>, index: usize) {
    let s = state.borrow();
    let pane = &s.panes[index];
    let other = &s.panes[1 - index];
    let names = selected_names(pane);
    if names.is_empty() {
        pane.foot.set_visible_child_name("info");
        pane.info.remove_css_class("error");
        let text = match &pane.listing {
            Listing::Loading => "Loading…".to_owned(),
            Listing::Ready => {
                let count = items_text(visible_entries(pane).len());
                match &pane.location {
                    Location::Remote { .. } => format!("{count} · Connected"),
                    Location::Local(_) => count,
                }
            }
            Listing::Failed(_) => {
                pane.info.add_css_class("error");
                "Couldn't open this folder".to_owned()
            }
        };
        pane.info.set_text(&text);
        return;
    }
    pane.foot.set_visible_child_name("selection");
    let bytes: u64 = pane
        .entries
        .iter()
        .filter(|e| !e.is_dir && names.contains(&e.name))
        .filter_map(|e| e.size)
        .sum();
    let mut text = format!("<b>{} selected</b>", names.len());
    if bytes > 0 {
        text.push_str(&format!(" · {}", format_size(bytes)));
    }
    pane.selected.set_markup(&text);
    pane.copy_path.set_visible(!s.narrow);
    let destination = place_label(&other.location, &s.profiles);
    let face = widgets::hbox(7);
    face.append(&gtk::Image::from_icon_name("transfer-copy-symbolic"));
    face.append(&gtk::Label::new(Some(&format!("Copy to {destination}"))));
    let arrow = gtk::Image::from_icon_name("transfer-arrow-r-symbolic");
    arrow.add_css_class("copy-arrow");
    if index == 1 {
        arrow.add_css_class("flip");
        face.prepend(&arrow);
    } else {
        face.append(&arrow);
    }
    pane.copy_to.set_child(Some(&face));
    pane.copy_to.set_tooltip_text(Some(&format!(
        "Copy the selection to {}",
        location_name(&other.location, &s.profiles)
    )));
    pane.copy_to.set_sensitive(other.location != pane.location);
}

fn render_place(state: &Rc<RefCell<State>>, index: usize) {
    let s = state.borrow();
    let pane = &s.panes[index];
    pane.place_icon
        .set_icon_name(Some(place_icon(&pane.location, &s.profiles)));
    match &pane.location {
        Location::Local(_) => {
            pane.place_name.set_text("This computer");
            pane.place_proto.set_visible(false);
        }
        Location::Remote { profile_id, .. } => {
            let profile = s.profiles.iter().find(|p| &p.id == profile_id);
            pane.place_name
                .set_text(profile.map(|p| p.name.as_str()).unwrap_or("Remote"));
            match profile {
                Some(p) if p.protocol != Protocol::Rclone => {
                    pane.place_proto.set_text(protocol_short(&p.protocol));
                    pane.place_proto.set_visible(true);
                }
                _ => pane.place_proto.set_visible(false),
            }
        }
    }
}

fn render_breadcrumb(state: &Rc<RefCell<State>>, index: usize) {
    let (bar, location) = {
        let s = state.borrow();
        (
            s.panes[index].breadcrumb.clone(),
            s.panes[index].location.clone(),
        )
    };
    while let Some(child) = bar.first_child() {
        bar.remove(&child);
    }
    let mut crumbs: Vec<(String, Location)> = Vec::new();
    match location {
        Location::Local(path) => {
            let home = glib::home_dir();
            let (mut current, rest) = match path.strip_prefix(&home) {
                Ok(rest) if home.parent().is_some() => {
                    if rest.as_os_str().is_empty() {
                        crumbs.push(("Home".into(), Location::Local(home.clone())));
                    }
                    (home.clone(), rest.to_path_buf())
                }
                _ => {
                    crumbs.push(("/".into(), Location::Local(PathBuf::from("/"))));
                    (PathBuf::from("/"), path.clone())
                }
            };
            for part in rest.components() {
                if let std::path::Component::Normal(name) = part {
                    current.push(name);
                    crumbs.push((
                        name.to_string_lossy().to_string(),
                        Location::Local(current.clone()),
                    ));
                }
            }
        }
        Location::Remote { profile_id, path } => {
            crumbs.push((
                "/".into(),
                Location::Remote {
                    profile_id: profile_id.clone(),
                    path: String::new(),
                },
            ));
            let mut current = String::new();
            for part in path.split('/').filter(|part| !part.is_empty()) {
                if !current.is_empty() {
                    current.push('/');
                }
                current.push_str(part);
                crumbs.push((
                    part.to_owned(),
                    Location::Remote {
                        profile_id: profile_id.clone(),
                        path: current.clone(),
                    },
                ));
            }
        }
    }
    let last = crumbs.len().saturating_sub(1);
    for (position, (name, location)) in crumbs.into_iter().enumerate() {
        if position > 0 {
            let sep = gtk::Image::from_icon_name("transfer-chev-r-symbolic");
            sep.add_css_class("crumb-sep");
            bar.append(&sep);
        }
        let button = gtk::Button::with_label(&name);
        button.add_css_class("flat");
        button.add_css_class("crumb");
        if position == last {
            button.add_css_class("last");
        }
        let s = state.clone();
        button.connect_clicked(move |_| navigate(s.clone(), index, location.clone()));
        bar.append(&button);
    }
}

fn place_menu(state: &Rc<RefCell<State>>, index: usize, anchor: &gtk::Button) {
    let profiles = state.borrow().profiles.clone();
    let mut entries = Vec::new();
    {
        let s = state.clone();
        entries.push(
            menu::item("This computer", move || {
                navigate(s.clone(), index, Location::Local(settings::start_folder()))
            })
            .icon("transfer-monitor-symbolic"),
        );
    }
    for profile in profiles {
        let s = state.clone();
        let location = profile_home(&profile);
        let icon = if profile.protocol == Protocol::Rclone {
            "transfer-cloud-symbolic"
        } else {
            "transfer-server-symbolic"
        };
        entries.push(
            menu::item(&profile.name, move || {
                navigate(s.clone(), index, location.clone())
            })
            .icon(icon)
            .key(protocol_short(&profile.protocol)),
        );
    }
    entries.push(menu::separator());
    {
        let s = state.clone();
        entries.push(menu::item("Open a folder on this computer…", move || {
            choose_local_folder(&s, index)
        }));
    }
    {
        let s = state.clone();
        entries.push(menu::item("Connect to a server…", move || {
            let window = s.borrow().window.clone();
            connection_dialog(&window, s.clone());
        }));
    }
    menu::popup_below(anchor, entries);
}

fn view_menu(state: &Rc<RefCell<State>>, index: usize, anchor: &gtk::Button) {
    let (sort, hidden, can_forward) = {
        let s = state.borrow();
        let pane = &s.panes[index];
        (
            pane.sort,
            pane.show_hidden,
            pane.history_position + 1 < pane.history.len(),
        )
    };
    let mut entries = vec![menu::heading("Sort by")];
    for (column, title) in ["Name", "Size", "Modified"].into_iter().enumerate() {
        let s = state.clone();
        entries.push(menu::check(title, sort == column as u32, move |_| {
            {
                let mut state = s.borrow_mut();
                let pane = &mut state.panes[index];
                pane.sort = column as u32;
                pane.descending = false;
            }
            render_pane(&s, index);
        }));
    }
    entries.push(menu::separator());
    {
        let s = state.clone();
        entries.push(
            menu::check("Show hidden files", hidden, move |_| {
                toggle_hidden(&s, index)
            })
            .key("Ctrl+H"),
        );
    }
    entries.push(menu::separator());
    {
        let s = state.clone();
        entries.push(
            menu::item("Forward", move || go_history(s.clone(), index, 1))
                .key("Alt+→")
                .enabled(can_forward),
        );
    }
    {
        let s = state.clone();
        entries.push(menu::item("Refresh", move || refresh(s.clone(), index)).key("Ctrl+R"));
    }
    {
        let s = state.clone();
        entries.push(menu::item("Type a location…", move || focus_path(&s, index)).key("Ctrl+L"));
    }
    {
        let s = state.clone();
        entries.push(menu::item("Open a folder on this computer…", move || {
            choose_local_folder(&s, index)
        }));
    }
    menu::popup_below(anchor, entries);
}

fn choose_local_folder(state: &Rc<RefCell<State>>, index: usize) {
    let window = state.borrow().window.clone();
    let s = state.clone();
    glib::spawn_future_local(async move {
        let dialog = gtk::FileDialog::new();
        if let Ok(file) = dialog.select_folder_future(Some(&window)).await
            && let Some(path) = file.path()
        {
            navigate(s, index, Location::Local(path));
        }
    });
}

fn install_row_actions(
    state: Rc<RefCell<State>>,
    index: usize,
    row: &gtk::ListBoxRow,
    name: &str,
    is_dir: bool,
) {
    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY)
        .build();
    {
        let state = state.clone();
        let row = row.clone();
        let name = name.to_owned();
        drag.connect_prepare(move |_, _, _| {
            let s = state.borrow();
            let pane = &s.panes[index];
            let names: Vec<String> = if row.is_selected() {
                selected_names(pane)
            } else {
                vec![name.clone()]
            };
            let sources: Vec<Location> = names
                .iter()
                .filter_map(|name| pane.location.child(name).ok())
                .collect();
            if sources.is_empty() {
                return None;
            }
            if sources
                .iter()
                .all(|source| matches!(source, Location::Local(_)))
            {
                let files: Vec<gio::File> = sources
                    .iter()
                    .filter_map(|source| match source {
                        Location::Local(path) => Some(gio::File::for_path(path)),
                        _ => None,
                    })
                    .collect();
                return Some(gtk::gdk::ContentProvider::for_value(
                    &gtk::gdk::FileList::from_array(&files).to_value(),
                ));
            }
            let payload = format!("nyx-transfer:{}", serde_json::to_string(&sources).ok()?);
            Some(gtk::gdk::ContentProvider::for_value(&payload.to_value()))
        });
    }
    row.add_controller(drag);
    let row_ = row.clone();
    let name = name.to_owned();
    menu::on_right_click(row, move |widget, x, y| {
        let (list, source, destination) = {
            let s = state.borrow();
            let pane = &s.panes[index];
            (
                pane.list.clone(),
                pane.location.child(&name).ok(),
                place_label(&s.panes[1 - index].location, &s.profiles),
            )
        };
        if !row_.is_selected() {
            list.unselect_all();
            list.select_row(Some(&row_));
        }
        let mut entries = Vec::new();
        if is_dir && let Some(source) = source.clone() {
            let s = state.clone();
            entries.push(
                menu::item("Open", move || navigate(s.clone(), index, source.clone()))
                    .icon("transfer-folder-symbolic"),
            );
        }
        {
            let s = state.clone();
            entries.push(
                menu::item(&format!("Copy to {destination}"), move || {
                    queue_selected(s.clone(), index, 1 - index)
                })
                .icon("transfer-copy-symbolic"),
            );
        }
        if let Some(source) = source {
            let s = state.clone();
            entries.push(menu::item("Copy path", move || {
                if let Some(display) = gtk::gdk::Display::default() {
                    let value = location_name(&source, &s.borrow().profiles);
                    display.clipboard().set_text(&value);
                }
            }));
        }
        menu::popup_at(widget, x, y, entries);
    });
}

fn drop_destination(state: &Rc<RefCell<State>>, index: usize, y: f64) -> Location {
    let s = state.borrow();
    let pane = &s.panes[index];
    let current = pane.location.clone();
    if let Some(row) = pane.list.row_at_y(y as i32) {
        let name = row.widget_name().to_string();
        if pane
            .entries
            .iter()
            .any(|entry| entry.name == name && entry.is_dir)
        {
            return current.child(&name).unwrap_or(current);
        }
    }
    current
}

fn queue_dropped(state: Rc<RefCell<State>>, sources: Vec<Location>, destination: Location) -> bool {
    let mut queued = false;
    for source in sources {
        let name = match &source {
            Location::Local(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().to_string()),
            Location::Remote { path, .. } => path.rsplit('/').next().map(str::to_owned),
        };
        let Some(name) = name else { continue };
        let Ok(target) = destination.child(&name) else {
            continue;
        };
        if target == source {
            continue;
        }
        enqueue_job(state.clone(), source, target);
        queued = true;
    }
    queued
}

fn install_drop_targets(state: Rc<RefCell<State>>, index: usize, list: &gtk::ListBox) {
    let files = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY,
    );
    {
        let s = state.clone();
        files.connect_drop(move |_, value, _, y| {
            let Ok(files) = value.get::<gtk::gdk::FileList>() else {
                return false;
            };
            let sources: Vec<Location> = files
                .files()
                .iter()
                .filter_map(|file| file.path().map(Location::Local))
                .collect();
            let destination = drop_destination(&s, index, y);
            queue_dropped(s.clone(), sources, destination)
        });
    }
    list.add_controller(files);
    let remote = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::COPY);
    {
        let s = state.clone();
        remote.connect_drop(move |_, value, _, y| {
            let Ok(payload) = value.get::<String>() else {
                return false;
            };
            let Some(json) = payload.strip_prefix("nyx-transfer:") else {
                return false;
            };
            let Ok(sources) = serde_json::from_str::<Vec<Location>>(json) else {
                return false;
            };
            let destination = drop_destination(&s, index, y);
            queue_dropped(s.clone(), sources, destination)
        });
    }
    list.add_controller(remote);
}

fn queue_selected(state: Rc<RefCell<State>>, from: usize, to: usize) {
    let (source, destination, names) = {
        let s = state.borrow();
        (
            s.panes[from].location.clone(),
            s.panes[to].location.clone(),
            selected_names(&s.panes[from]),
        )
    };
    if names.is_empty() {
        notify(&state, "Select a file or folder first.");
        return;
    }
    if source == destination {
        notify(&state, "Both panes show the same folder.");
        return;
    }
    for name in names {
        let (Ok(src), Ok(dst)) = (source.child(&name), destination.child(&name)) else {
            continue;
        };
        enqueue_job(state.clone(), src, dst);
    }
    state.borrow().panes[from].list.unselect_all();
}

/// A place's short name: a server's or remote's name, or a local folder's.
pub(super) fn place_label(location: &Location, profiles: &[ConnectionProfile]) -> String {
    match location {
        Location::Local(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "This computer".into()),
        Location::Remote { profile_id, .. } => profiles
            .iter()
            .find(|p| &p.id == profile_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Remote".into()),
    }
}

pub(super) fn place_icon(location: &Location, profiles: &[ConnectionProfile]) -> &'static str {
    match location {
        Location::Local(_) => "transfer-monitor-symbolic",
        Location::Remote { profile_id, .. } => {
            if profiles
                .iter()
                .any(|p| &p.id == profile_id && p.protocol == Protocol::Rclone)
            {
                "transfer-cloud-symbolic"
            } else {
                "transfer-server-symbolic"
            }
        }
    }
}

pub(super) fn protocol_short(protocol: &Protocol) -> &'static str {
    match protocol {
        Protocol::Sftp => "SFTP",
        Protocol::Ftp => "FTP",
        Protocol::FtpsExplicit | Protocol::FtpsImplicit => "FTPS",
        Protocol::Webdav => "WebDAV",
        Protocol::Rclone => "rclone",
    }
}

fn file_icon(entry: &FileEntry) -> &'static str {
    if entry.is_dir {
        return "transfer-folder-symbolic";
    }
    let extension = entry
        .name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" | "htm" | "css" | "scss" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "rs" | "py"
        | "go" | "c" | "h" | "cpp" | "java" | "rb" | "php" | "sh" | "toml" | "yaml" | "yml"
        | "json" | "lua" => "transfer-code-symbolic",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" | "avif" | "heic"
        | "tif" | "tiff" => "transfer-image-symbolic",
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "opus" | "aac" => "transfer-music-symbolic",
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" => "transfer-film-symbolic",
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" | "zst" => {
            "transfer-archive-symbolic"
        }
        "txt" | "md" | "xml" | "csv" | "log" | "pdf" | "doc" | "docx" | "odt" | "rtf" => {
            "transfer-file-text-symbolic"
        }
        _ => "transfer-file-symbolic",
    }
}

pub(super) fn items_text(count: usize) -> String {
    if count == 1 {
        "1 item".into()
    } else {
        format!("{count} items")
    }
}

/// Sizes the way Files shows them: 64 bytes, 9.4 KB, 212 KB, 1.3 MB.
pub(super) fn format_size(bytes: u64) -> String {
    if bytes == 1 {
        return "1 byte".into();
    }
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64;
    let mut unit = "bytes";
    for next in ["KB", "MB", "GB", "TB"] {
        if value < 1000.0 {
            break;
        }
        value /= 1000.0;
        unit = next;
    }
    let text = if value >= 100.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    };
    format!("{} {unit}", text.trim_end_matches(".0"))
}

/// Today 09:41, Yesterday, Mon, 12 Oct, or 12 Oct 2024 for other years.
fn format_modified(value: Option<&str>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let fallback = || value.chars().take(10).collect::<String>();
    let Some(time) = glib::DateTime::from_iso8601(value, None)
        .ok()
        .and_then(|t| t.to_local().ok())
    else {
        return fallback();
    };
    let Ok(now) = glib::DateTime::now_local() else {
        return fallback();
    };
    let day = |t: &glib::DateTime| {
        glib::DateTime::from_local(t.year(), t.month(), t.day_of_month(), 0, 0, 0.0).ok()
    };
    let days = match (day(&now), day(&time)) {
        (Some(a), Some(b)) => (a.difference(&b).as_seconds() as f64 / 86400.0).round() as i64,
        _ => return fallback(),
    };
    let format = match days {
        0 => "Today %H:%M",
        1 => "Yesterday",
        2..=6 => "%a",
        _ if time.year() == now.year() => "%-d %b",
        _ => "%-d %b %Y",
    };
    time.format(format)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| fallback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_files() {
        assert_eq!(format_size(64), "64 bytes");
        assert_eq!(format_size(9_400), "9.4 KB");
        assert_eq!(format_size(15_000), "15 KB");
        assert_eq!(format_size(212_000), "212 KB");
        assert_eq!(format_size(1_300_000), "1.3 MB");
    }

    #[test]
    fn item_counts() {
        assert_eq!(items_text(1), "1 item");
        assert_eq!(items_text(48), "48 items");
    }
}
