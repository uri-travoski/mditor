mod render;
mod settings;
mod update;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use sv::prelude::*;

const APP_ID: &str = "dev.mditor.Mditor";
const MAX_WIDTH: i32 = 760;

const FONT_MIN: i32 = -3;
const FONT_MAX: i32 = 12;

thread_local! {
    static FONT_OFFSET: Cell<i32> = const { Cell::new(0) };
    /// User-chosen default size in pt; None follows the system font.
    static FONT_BASE: Cell<Option<f64>> = const { Cell::new(None) };
    static FONT_CSS: gtk::CssProvider = gtk::CssProvider::new();
    static WINDOWS: RefCell<Vec<Rc<Window>>> = const { RefCell::new(Vec::new()) };
}

struct Window {
    win: gtk::ApplicationWindow,
    notebook: gtk::Notebook,
    view_btn: gtk::ToggleButton,
    edit_btn: gtk::ToggleButton,
    docs: RefCell<Vec<Rc<Doc>>>,
    syncing: Cell<bool>,
}

struct Doc {
    win: Weak<Window>,
    page: gtk::Stack,
    buffer: sv::Buffer,
    edit: sv::View,
    preview: gtk::TextView,
    tags: render::Tags,
    file: RefCell<Option<gio::File>>,
    stale: Cell<bool>,
    rendered: RefCell<render::Rendered>,
    label: gtk::Label,
}

fn centered(sw: &gtk::ScrolledWindow, view: &impl IsA<gtk::TextView>) {
    let view = view.as_ref().clone();
    sw.add_css_class("mditor");
    sw.hadjustment().connect_notify_local(Some("page-size"), move |a, _| {
        let m = ((a.page_size() as i32 - MAX_WIDTH) / 2).max(24);
        if view.margin_start() != m {
            view.set_margin_start(m);
            view.set_margin_end(m);
        }
    });
}

// ---------------------------------------------------------------- Window

impl Window {
    fn new(app: &gtk::Application) -> Rc<Window> {
        let notebook = gtk::Notebook::builder().scrollable(true).show_border(false).build();

        let view_btn = gtk::ToggleButton::with_label("View");
        let edit_btn = gtk::ToggleButton::with_label("Edit");
        edit_btn.set_group(Some(&view_btn));
        let toggles = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toggles.add_css_class("linked");
        toggles.append(&view_btn);
        toggles.append(&edit_btn);

        let open_btn = gtk::Button::from_icon_name("document-open-symbolic");
        open_btn.set_action_name(Some("win.open"));
        open_btn.set_tooltip_text(Some("Open (Ctrl+O)"));
        let save_btn = gtk::Button::from_icon_name("document-save-symbolic");
        save_btn.set_action_name(Some("win.save"));
        save_btn.set_tooltip_text(Some("Save (Ctrl+S)"));
        let new_btn = gtk::Button::from_icon_name("tab-new-symbolic");
        new_btn.set_action_name(Some("win.new-tab"));
        new_btn.set_tooltip_text(Some("New tab (Ctrl+T)"));
        let app_icon = gtk::Image::from_icon_name(APP_ID);
        app_icon.set_pixel_size(24);
        app_icon.set_margin_end(4);

        let menu = gio::Menu::new();
        let zoom = gio::Menu::new();
        zoom.append(Some("Increase Font Size"), Some("app.zoom-in"));
        zoom.append(Some("Decrease Font Size"), Some("app.zoom-out"));
        zoom.append(Some("Reset Font Size"), Some("app.zoom-reset"));
        zoom.append(Some("Default Font Size…"), Some("app.font-default"));
        menu.append_section(None, &zoom);
        menu.append(Some("Check for Updates…"), Some("app.check-updates"));
        menu.append(Some("Check for Updates on Startup"), Some("app.auto-update"));
        menu.append(Some("About mditor"), Some("app.about"));
        let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).build();

        // A plain toolbar inside the window (no custom titlebar) so the system
        // draws the title bar and its minimize/maximize/close buttons.
        let start = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for w in [app_icon.upcast_ref::<gtk::Widget>(), open_btn.upcast_ref(), save_btn.upcast_ref(), new_btn.upcast_ref()] {
            start.append(w);
        }
        let bar = gtk::CenterBox::new();
        bar.add_css_class("toolbar");
        bar.set_start_widget(Some(&start));
        bar.set_center_widget(Some(&toggles));
        bar.set_end_widget(Some(&menu_btn));
        bar.set_margin_start(6);
        bar.set_margin_end(6);
        bar.set_margin_top(6);
        bar.set_margin_bottom(6);

        notebook.set_vexpand(true);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&bar);
        content.append(&notebook);

        let win = gtk::ApplicationWindow::builder()
            .application(app)
            .default_width(900)
            .default_height(700)
            .child(&content)
            .build();

        let w = Rc::new(Window {
            win,
            notebook,
            view_btn,
            edit_btn,
            docs: RefCell::new(vec![]),
            syncing: Cell::new(false),
        });
        w.wire();
        WINDOWS.with(|ws| ws.borrow_mut().push(w.clone()));
        w
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let add = |name: &str, f: fn(&Rc<Window>)| {
            let a = gio::SimpleAction::new(name, None);
            let weak = weak.clone();
            a.connect_activate(move |_, _| {
                if let Some(w) = weak.upgrade() {
                    f(&w);
                }
            });
            self.win.add_action(&a);
        };
        add("toggle", |w| {
            if let Some(d) = w.cur() {
                d.set_mode(!d.is_view());
            }
        });
        add("save", |w| {
            if let Some(d) = w.cur() {
                d.save(None)
            }
        });
        add("save-as", |w| {
            if let Some(d) = w.cur() {
                d.save_as(None)
            }
        });
        add("open", |w| w.open_dialog());
        add("new-tab", |w| {
            w.add_doc(None);
        });
        add("close-tab", |w| {
            if let Some(d) = w.cur() {
                w.close_doc(&d)
            }
        });
        add("next-tab", |w| w.notebook.next_page());
        add("prev-tab", |w| w.notebook.prev_page());

        for (btn, view) in [(&self.view_btn, true), (&self.edit_btn, false)] {
            let weak = weak.clone();
            btn.connect_toggled(move |b| {
                let Some(w) = weak.upgrade() else { return };
                if b.is_active() && !w.syncing.get() {
                    if let Some(d) = w.cur() {
                        d.set_mode(view);
                    }
                }
            });
        }

        self.notebook.connect_switch_page({
            let weak = weak.clone();
            move |_, page, _| {
                let Some(w) = weak.upgrade() else { return };
                let doc = w.docs.borrow().iter().find(|d| d.page.upcast_ref::<gtk::Widget>() == page).cloned();
                if let Some(d) = doc {
                    w.sync(&d);
                }
            }
        });

        if let Some(s) = gtk::Settings::default() {
            for prop in ["gtk-theme-name", "gtk-application-prefer-dark-theme"] {
                let weak = weak.clone();
                s.connect_notify_local(Some(prop), move |_, _| {
                    let weak = weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(w) = weak.upgrade() {
                            for d in w.docs.borrow().iter() {
                                d.apply_scheme(&w.win);
                            }
                        }
                    });
                });
            }
        }

        self.win.connect_close_request({
            let weak = weak.clone();
            move |_| {
                let Some(w) = weak.upgrade() else { return glib::Propagation::Proceed };
                let dirty = w.docs.borrow().iter().find(|d| d.buffer.is_modified()).cloned();
                let Some(d) = dirty else { return glib::Propagation::Proceed };
                let w2 = w.clone();
                w.ask_save(&d, Rc::new(move || w2.win.close()));
                glib::Propagation::Stop
            }
        });

        self.win.connect_destroy(|win| {
            WINDOWS.with(|ws| ws.borrow_mut().retain(|w| w.win.upcast_ref::<gtk::Window>() != win.upcast_ref::<gtk::Window>()));
        });

        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        drop.connect_drop({
            let weak = weak.clone();
            move |_, v, _, _| {
                let (Some(w), Ok(list)) = (weak.upgrade(), v.get::<gdk::FileList>()) else { return false };
                for f in list.files() {
                    w.add_doc(Some(f));
                }
                true
            }
        });
        self.win.add_controller(drop);

        // Ctrl+Tab must beat the text views' focus-cycling bindings.
        let keys = gtk::ShortcutController::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        for (trigger, action) in [("<Ctrl>Tab|<Ctrl>Page_Down", "win.next-tab"), ("<Ctrl><Shift>ISO_Left_Tab|<Ctrl>Page_Up", "win.prev-tab")] {
            if let Some(t) = gtk::ShortcutTrigger::parse_string(trigger) {
                keys.add_shortcut(gtk::Shortcut::new(Some(t), Some(gtk::NamedAction::new(action))));
            }
        }
        self.win.add_controller(keys);
    }

    fn cur(&self) -> Option<Rc<Doc>> {
        let page = self.notebook.nth_page(self.notebook.current_page())?;
        self.docs.borrow().iter().find(|d| d.page.upcast_ref::<gtk::Widget>() == &page).cloned()
    }

    /// Reflect `doc`'s mode and name in the header.
    fn sync(&self, doc: &Rc<Doc>) {
        self.syncing.set(true);
        let v = doc.is_view();
        self.view_btn.set_active(v);
        self.edit_btn.set_active(!v);
        self.syncing.set(false);
        let m = if doc.buffer.is_modified() { "• " } else { "" };
        let n = doc.display_name();
        self.win.set_title(Some(&format!("{m}{n} — mditor")));
    }

    fn refresh(&self) {
        if let Some(d) = self.cur() {
            self.sync(&d);
        }
    }

    fn add_doc(self: &Rc<Self>, file: Option<gio::File>) -> Rc<Doc> {
        if let Some(f) = &file {
            let existing = self.docs.borrow().iter().find(|d| d.file.borrow().as_ref().is_some_and(|g| g.equal(f))).cloned();
            if let Some(d) = existing {
                self.notebook.set_current_page(self.notebook.page_num(&d.page));
                return d;
            }
            if let Some(d) = self.cur().filter(|d| d.is_pristine()) {
                d.load(f.clone());
                return d;
            }
        }
        let doc = Doc::new(self);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.set_focus_on_click(false);
        let tab = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        tab.append(&doc.label);
        tab.append(&close);
        close.connect_clicked({
            let (w, d) = (Rc::downgrade(self), doc.clone());
            move |_| {
                if let Some(w) = w.upgrade() {
                    w.close_doc(&d)
                }
            }
        });
        self.docs.borrow_mut().push(doc.clone());
        let n = self.notebook.append_page(&doc.page, Some(&tab));
        self.notebook.set_tab_reorderable(&doc.page, true);
        self.notebook.set_current_page(Some(n));
        self.notebook.set_show_tabs(self.docs.borrow().len() > 1);
        doc.apply_scheme(&self.win);
        match file {
            Some(f) => doc.load(f),
            None => {
                doc.update_title();
                doc.set_mode(false);
            }
        }
        doc
    }

    fn ask_save(self: &Rc<Self>, doc: &Rc<Doc>, then: Rc<dyn Fn()>) {
        self.notebook.set_current_page(self.notebook.page_num(&doc.page));
        let dlg = gtk::AlertDialog::builder()
            .message(format!("Save changes to “{}”?", doc.display_name()))
            .detail("Your changes will be lost if you don't save them.")
            .buttons(["Cancel", "Discard", "Save"])
            .cancel_button(0)
            .default_button(2)
            .build();
        let d = doc.clone();
        dlg.choose(Some(&self.win), gio::Cancellable::NONE, move |r| match r {
            Ok(1) => {
                d.buffer.set_modified(false);
                then();
            }
            Ok(2) => d.save(Some(Box::new(move || then()))),
            _ => {}
        });
    }

    fn close_doc(self: &Rc<Self>, doc: &Rc<Doc>) {
        let (w, d) = (self.clone(), doc.clone());
        let remove: Rc<dyn Fn()> = Rc::new(move || w.remove_doc(&d));
        if doc.buffer.is_modified() {
            self.ask_save(doc, remove);
        } else {
            remove();
        }
    }

    fn remove_doc(&self, doc: &Rc<Doc>) {
        if let Some(n) = self.notebook.page_num(&doc.page) {
            self.notebook.remove_page(Some(n));
        }
        self.docs.borrow_mut().retain(|d| !Rc::ptr_eq(d, doc));
        let n = self.docs.borrow().len();
        self.notebook.set_show_tabs(n > 1);
        if n == 0 {
            self.win.close();
        } else {
            self.refresh();
        }
    }

    fn open_dialog(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Markdown"));
        for p in ["*.md", "*.markdown", "*.mdown", "*.txt"] {
            filter.add_pattern(p);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dlg = gtk::FileDialog::builder().title("Open").filters(&filters).build();
        let w = self.clone();
        dlg.open_multiple(Some(&self.win), gio::Cancellable::NONE, move |r| {
            let Ok(list) = r else { return };
            for i in 0..list.n_items() {
                if let Some(f) = list.item(i).and_downcast::<gio::File>() {
                    w.add_doc(Some(f));
                }
            }
        });
    }

    fn error(&self, msg: &str) {
        gtk::AlertDialog::builder().message(msg).build().show(Some(&self.win));
    }
}

// ------------------------------------------------------------------- Doc

impl Doc {
    fn new(window: &Rc<Window>) -> Rc<Doc> {
        let buffer = sv::Buffer::new(None);
        if let Some(l) = sv::LanguageManager::default().language("markdown") {
            buffer.set_language(Some(&l));
        }
        buffer.set_highlight_matching_brackets(false);

        let edit = sv::View::with_buffer(&buffer);
        edit.set_wrap_mode(gtk::WrapMode::WordChar);
        edit.set_monospace(true);
        edit.set_auto_indent(true);
        edit.set_top_margin(16);
        edit.set_bottom_margin(16);
        edit.set_margin_start(24);
        edit.set_margin_end(24);
        let edit_sw = gtk::ScrolledWindow::builder().child(&edit).build();
        centered(&edit_sw, &edit);

        let preview = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(16)
            .bottom_margin(32)
            .margin_start(24)
            .margin_end(24)
            .build();
        let preview_sw = gtk::ScrolledWindow::builder().child(&preview).build();
        centered(&preview_sw, &preview);
        let tags = render::Tags::new(&preview.buffer());

        let page = gtk::Stack::new();
        page.add_named(&edit_sw, Some("edit"));
        page.add_named(&preview_sw, Some("view"));

        let label = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .width_chars(8)
            .max_width_chars(24)
            .build();

        let d = Rc::new(Doc {
            win: Rc::downgrade(window),
            page,
            buffer,
            edit,
            preview,
            tags,
            file: RefCell::new(None),
            stale: Cell::new(true),
            rendered: RefCell::new(Default::default()),
            label,
        });
        d.wire();
        d
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.buffer.connect_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(d) = weak.upgrade() {
                    d.stale.set(true);
                }
            }
        });
        self.buffer.connect_modified_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(d) = weak.upgrade() {
                    d.update_title();
                }
            }
        });

        let click = gtk::GestureClick::new();
        click.connect_released({
            let weak = weak.clone();
            move |_, _, x, y| {
                let Some(d) = weak.upgrade() else { return };
                if d.preview.buffer().has_selection() {
                    return;
                }
                if let Some(url) = d.link_at(x, y) {
                    d.follow(&url);
                }
            }
        });
        self.preview.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let weak = weak.clone();
            move |_, x, y| {
                let Some(d) = weak.upgrade() else { return };
                let c = if d.link_at(x, y).is_some() { "pointer" } else { "text" };
                d.preview.set_cursor_from_name(Some(c));
            }
        });
        self.preview.add_controller(motion);
    }

    fn follow(&self, url: &str) {
        let local = !url.contains(':') && !url.starts_with('#');
        if !local {
            let _ = gio::AppInfo::launch_default_for_uri(url, gio::AppLaunchContext::NONE);
            return;
        }
        let rel = url.split('#').next().unwrap_or("");
        let (Some(base), Some(w)) = (self.base_dir(), self.win.upgrade()) else { return };
        let p = base.join(rel);
        if !rel.is_empty() && p.is_file() {
            w.add_doc(Some(gio::File::for_path(p)));
        }
    }

    fn link_at(&self, x: f64, y: f64) -> Option<String> {
        let (bx, by) = self.preview.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let it = self.preview.iter_at_location(bx, by)?;
        self.rendered.borrow().link_at(it.offset()).map(str::to_string)
    }

    fn apply_scheme(&self, win: &gtk::ApplicationWindow) {
        let fg = win.color();
        let dark = fg.red() + fg.green() + fg.blue() > 1.5;
        let ids: &[&str] = if dark { &["Adwaita-dark", "oblivion"] } else { &["Adwaita", "classic"] };
        let m = sv::StyleSchemeManager::default();
        if let Some(s) = ids.iter().find_map(|i| m.scheme(i)) {
            self.buffer.set_style_scheme(Some(&s));
        }
        self.tags.refresh_theme(&self.preview);
    }

    fn display_name(&self) -> String {
        self.file
            .borrow()
            .as_ref()
            .and_then(|f| f.basename())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    fn update_title(&self) {
        let m = if self.buffer.is_modified() { "• " } else { "" };
        self.label.set_text(&format!("{m}{}", self.display_name()));
        if let Some(w) = self.win.upgrade() {
            w.refresh();
        }
    }

    fn base_dir(&self) -> Option<PathBuf> {
        self.file.borrow().as_ref()?.parent()?.path()
    }

    fn is_pristine(&self) -> bool {
        self.file.borrow().is_none() && !self.buffer.is_modified() && self.buffer.char_count() == 0
    }

    fn is_view(&self) -> bool {
        self.page.visible_child_name().as_deref() == Some("view")
    }

    fn render_preview(&self) {
        let t0 = std::time::Instant::now();
        let text = self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), false);
        let r = render::render(&self.preview, &self.tags, &text, self.base_dir().as_deref());
        *self.rendered.borrow_mut() = r;
        if std::env::var_os("MDITOR_TRACE").is_some() {
            eprintln!("render {} bytes: {:?}", text.len(), t0.elapsed());
        }
        self.stale.set(false);
    }

    fn scroll_to_offset(view: &gtk::TextView, off: i32) {
        if off == 0 {
            if let Some(adj) = view.vadjustment() {
                adj.set_value(0.0);
            }
            return;
        }
        let buf = view.buffer();
        let it = buf.iter_at_offset(off);
        let mark = match buf.mark("sync") {
            Some(m) => {
                buf.move_mark(&m, &it);
                m
            }
            None => buf.create_mark(Some("sync"), &it, false),
        };
        view.scroll_to_mark(&mark, 0.0, true, 0.0, 0.0);
    }

    fn set_mode(&self, view: bool) {
        let showing_view = self.is_view();
        if view && !showing_view {
            let rect = self.edit.visible_rect();
            let (top, _) = self.edit.line_at_y(rect.y());
            let line = top.line() as usize;
            if self.stale.get() {
                self.render_preview();
            }
            self.page.set_visible_child_name("view");
            let off = self.rendered.borrow().offset_for_line(line);
            Self::scroll_to_offset(&self.preview, off);
            self.preview.grab_focus();
        } else if !view && showing_view {
            let rect = self.preview.visible_rect();
            let (top, _) = self.preview.line_at_y(rect.y());
            let line = self.rendered.borrow().line_for_offset(top.offset());
            self.page.set_visible_child_name("edit");
            let it = self.buffer.iter_at_line(line as i32).unwrap_or_else(|| self.buffer.start_iter());
            Self::scroll_to_offset(self.edit.upcast_ref(), it.offset());
            self.buffer.place_cursor(&it);
            self.edit.grab_focus();
        }
        if let Some(w) = self.win.upgrade() {
            w.refresh();
        }
    }

    fn load(self: &Rc<Self>, file: gio::File) {
        let Some(path) = file.path() else {
            self.error("Only local files are supported.");
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.error(&format!("Could not open {}: {e}", path.display()));
                return;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&text);
        self.buffer.end_irreversible_action();
        self.buffer.set_modified(false);
        *self.file.borrow_mut() = Some(file);
        self.stale.set(true);
        self.buffer.place_cursor(&self.buffer.start_iter());
        self.update_title();
        self.set_mode(true);
    }

    fn save_as(self: &Rc<Self>, then: Option<Box<dyn FnOnce()>>) {
        let Some(w) = self.win.upgrade() else { return };
        let name = self.display_name();
        let dlg = gtk::FileDialog::builder()
            .title("Save As")
            .initial_name(if name.contains('.') { name } else { format!("{name}.md") })
            .build();
        let d = self.clone();
        dlg.save(Some(&w.win), gio::Cancellable::NONE, move |r| {
            let Ok(f) = r else { return };
            *d.file.borrow_mut() = Some(f);
            d.stale.set(true);
            d.write(then);
        });
    }

    fn save(self: &Rc<Self>, then: Option<Box<dyn FnOnce()>>) {
        if self.file.borrow().is_none() {
            self.save_as(then);
        } else {
            self.write(then);
        }
    }

    fn write(self: &Rc<Self>, then: Option<Box<dyn FnOnce()>>) {
        let Some(path) = self.file.borrow().as_ref().and_then(|f| f.path()) else { return };
        let text = self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), false);
        let mut tmp = path.clone().into_os_string();
        tmp.push(".mditor-tmp");
        let res = std::fs::write(&tmp, text.as_str()).and_then(|_| std::fs::rename(&tmp, &path));
        match res {
            Ok(()) => {
                self.buffer.set_modified(false);
                self.update_title();
                if let Some(t) = then {
                    t();
                }
            }
            Err(e) => self.error(&format!("Could not save {}: {e}", path.display())),
        }
    }

    fn error(&self, msg: &str) {
        if let Some(w) = self.win.upgrade() {
            w.error(msg);
        }
    }
}

// ------------------------------------------------------------------- app

fn target_window(app: &gtk::Application) -> Rc<Window> {
    let active = app.active_window();
    WINDOWS
        .with(|ws| {
            let ws = ws.borrow();
            ws.iter().find(|w| Some(w.win.upcast_ref::<gtk::Window>()) == active.as_ref()).or(ws.last()).cloned()
        })
        .unwrap_or_else(|| Window::new(app))
}

fn main() -> glib::ExitCode {
    // WM_CLASS (X11) / app_id (Wayland) must equal the .desktop file's name so
    // the taskbar and window switcher show the right icon.
    glib::set_prgname(Some(APP_ID));
    // On X11 let the window manager draw the title bar and its buttons.
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("GTK_CSD").is_none() {
        std::env::set_var("GTK_CSD", "0");
    }
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|app| {
        gtk::Window::set_default_icon_name(APP_ID);
        let css = gtk::CssProvider::new();
        css.load_from_data("scrolledwindow.mditor { background-color: @theme_base_color; }");
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        for (a, k) in [
            ("win.toggle", "<Ctrl>e"),
            ("win.save", "<Ctrl>s"),
            ("win.save-as", "<Ctrl><Shift>s"),
            ("win.open", "<Ctrl>o"),
            ("win.new-tab", "<Ctrl>t"),
            ("win.close-tab", "<Ctrl>w"),
            ("app.new-window", "<Ctrl>n"),
        ] {
            app.set_accels_for_action(a, &[k]);
        }
        app.set_accels_for_action("app.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]);
        app.set_accels_for_action("app.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]);
        app.set_accels_for_action("app.zoom-reset", &["<Ctrl>0", "<Ctrl>KP_0"]);
        let new = gio::SimpleAction::new("new-window", None);
        new.connect_activate(glib::clone!(
            #[weak]
            app,
            move |_, _| {
                let w = Window::new(&app);
                w.add_doc(None);
                w.win.present();
            }
        ));
        app.add_action(&new);

        let auto = gio::SimpleAction::new_stateful("auto-update", None, &update::auto_check_enabled().to_variant());
        auto.connect_change_state(|a, v| {
            if let Some(v) = v {
                a.set_state(v);
                update::set_auto_check(v.get::<bool>().unwrap_or(true));
            }
        });
        app.add_action(&auto);
        let check = gio::SimpleAction::new("check-updates", None);
        check.connect_activate(glib::clone!(
            #[weak]
            app,
            move |_, _| check_for_updates(&app, true)
        ));
        app.add_action(&check);
        let offset = settings::get("font_offset").and_then(|v| v.parse().ok()).unwrap_or(0).clamp(FONT_MIN, FONT_MAX);
        if let Some(d) = gdk::Display::default() {
            FONT_CSS.with(|c| gtk::style_context_add_provider_for_display(&d, c, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION));
        }
        FONT_BASE.with(|b| b.set(settings::get("font_base").and_then(|v| v.parse().ok()).filter(|v: &f64| (6.0..=40.0).contains(v))));
        set_font_offset(offset);
        let fd = gio::SimpleAction::new("font-default", None);
        fd.connect_activate(glib::clone!(
            #[weak]
            app,
            move |_, _| show_font_dialog(&app)
        ));
        app.add_action(&fd);
        for (name, delta) in [("zoom-in", Some(1)), ("zoom-out", Some(-1)), ("zoom-reset", None)] {
            let a = gio::SimpleAction::new(name, None);
            a.connect_activate(move |_, _| {
                let cur = FONT_OFFSET.with(Cell::get);
                set_font_offset(delta.map_or(0, |d| cur + d));
            });
            app.add_action(&a);
        }
        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(glib::clone!(
            #[weak]
            app,
            move |_, _| show_about(&app)
        ));
        app.add_action(&about);
    });
    app.connect_activate(|app| {
        let w = target_window(app);
        if w.docs.borrow().is_empty() {
            w.add_doc(None);
        }
        w.win.present();
        startup_update_check(app);
    });
    app.connect_open(|app, files, _| {
        let w = target_window(app);
        for f in files {
            w.add_doc(Some(f.clone()));
        }
        w.win.present();
        startup_update_check(app);
    });
    app.run()
}

fn startup_update_check(app: &gtk::Application) {
    thread_local! { static DONE: Cell<bool> = const { Cell::new(false) }; }
    if DONE.with(|d| d.replace(true)) || !update::auto_check_enabled() {
        return;
    }
    let app = app.clone();
    glib::spawn_future_local(async move {
        glib::timeout_future_seconds(3).await;
        check_for_updates(&app, false);
    });
}

/// `manual` checks also report "up to date" and errors; automatic ones stay silent.
fn check_for_updates(app: &gtk::Application, manual: bool) {
    let app = app.clone();
    glib::spawn_future_local(async move {
        let outcome = gio::spawn_blocking(update::check).await.unwrap_or_else(|_| update::Outcome::Failed("check failed".into()));
        if std::env::var_os("MDITOR_TRACE").is_some() {
            eprintln!("update check done: {}", match &outcome { update::Outcome::UpToDate => "up to date".to_string(), update::Outcome::Available { version, .. } => format!("available {version}"), update::Outcome::Failed(e) => format!("failed: {e}") });
        }
        let win = target_window(&app).win.clone();
        let dlg = gtk::AlertDialog::builder();
        match outcome {
            update::Outcome::Available { version, url } => {
                let dlg = dlg
                    .message(format!("mditor {version} is available"))
                    .detail(format!("You have {}.", update::CURRENT))
                    .buttons(["Later", "Download"])
                    .cancel_button(0)
                    .default_button(1)
                    .build();
                dlg.choose(Some(&win), gio::Cancellable::NONE, move |r| {
                    if r == Ok(1) {
                        let _ = gio::AppInfo::launch_default_for_uri(&url, gio::AppLaunchContext::NONE);
                    }
                });
            }
            update::Outcome::UpToDate if manual => {
                dlg.message("mditor is up to date").detail(format!("Version {}", update::CURRENT)).build().show(Some(&win));
            }
            update::Outcome::Failed(e) if manual => {
                dlg.message("Could not check for updates").detail(e).build().show(Some(&win));
            }
            _ => {}
        }
    });
}

fn show_about(app: &gtk::Application) {
    gtk::AboutDialog::builder()
        .transient_for(&target_window(app).win)
        .modal(true)
        .program_name("mditor")
        .version(update::CURRENT)
        .comments("Fast, minimal Markdown viewer and editor")
        .logo_icon_name(APP_ID)
        .website(update::REPO_URL)
        .website_label("github.com/uri-travoski/mditor")
        .copyright("© 2026 mditor contributors · GPL-3.0-or-later")
        .build()
        .present();
}

fn system_font_size() -> f64 {
    gtk::Settings::default()
        .and_then(|s| s.gtk_font_name())
        .and_then(|f| f.rsplit(' ').next().and_then(|n| n.parse::<f64>().ok()))
        .unwrap_or(11.0)
}

/// Text size = default size (user's, else the system UI font) + zoom `offset` points,
/// for the editor and preview.
fn set_font_offset(offset: i32) {
    let offset = offset.clamp(FONT_MIN, FONT_MAX);
    FONT_OFFSET.with(|o| o.set(offset));
    settings::set("font_offset", &offset.to_string());
    apply_font();
}

fn apply_font() {
    let base = FONT_BASE.with(Cell::get).unwrap_or_else(system_font_size);
    let size = (base + FONT_OFFSET.with(Cell::get) as f64).max(6.0);
    FONT_CSS.with(|c| c.load_from_data(&format!("textview, textview label {{ font-size: {size}pt; }}")));
}

fn show_font_dialog(app: &gtk::Application) {
    let current = FONT_BASE.with(Cell::get).unwrap_or_else(system_font_size);
    let spin = gtk::SpinButton::with_range(6.0, 40.0, 1.0);
    spin.set_value(current.round());
    spin.connect_value_changed(|s| {
        let v = s.value();
        FONT_BASE.with(|b| b.set(Some(v)));
        settings::set("font_base", &v.to_string());
        apply_font();
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&gtk::Label::new(Some("Default size (pt)")));
    row.append(&spin);
    let system = gtk::Button::with_label("Use System Size");
    system.connect_clicked(glib::clone!(
        #[weak]
        spin,
        move |_| {
            FONT_BASE.with(|b| b.set(None));
            settings::set("font_base", "system");
            apply_font();
            spin.set_value(system_font_size().round());
        }
    ));
    let close = gtk::Button::with_label("Close");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&system);
    buttons.append(&close);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.append(&row);
    body.append(&buttons);
    let dlg = gtk::Window::builder()
        .title("Default Font Size")
        .modal(true)
        .resizable(false)
        .transient_for(&target_window(app).win)
        .child(&body)
        .build();
    close.connect_clicked(glib::clone!(
        #[weak]
        dlg,
        move |_| dlg.close()
    ));
    dlg.present();
}
