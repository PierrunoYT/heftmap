//! egui front end: folder tree table, treemap, file types and largest files.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Button, FontId, Key, Layout, Rect, RichText, Sense, TextEdit, Ui,
    vec2,
};
use egui_extras::{Column, TableBuilder, TableRow};

use crate::format;
use crate::scan::{self, Progress};
use crate::tree::{NodeId, ROOT, SortKey, SubtreeStats, Tree};
use crate::treemap::Treemap;

const ROW_HEIGHT: f32 = 20.0;
const INDENT: f32 = 14.0;
const LARGEST_FILES: usize = 250;
const PAGE_ROWS: usize = 20;

pub fn run(initial: Option<PathBuf>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Heftmap")
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size([720.0, 420.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("embedded icon is a valid PNG"),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Heftmap",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(&cc.egui_ctx, initial)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SideTab {
    Treemap,
    Types,
    Largest,
}

enum Action {
    /// Select from the tree table: the side panel follows the selection.
    Select(NodeId),
    /// Select from the side panel: reveal the node in the tree, keep the side panel scope.
    Reveal(NodeId),
    /// Make a directory the side panel scope (and select it).
    Focus(NodeId),
    Toggle(NodeId),
    Sort(SortKey),
    OpenInFileManager(NodeId),
    CopyPath(NodeId),
}

struct ScanJob {
    root: PathBuf,
    progress: Arc<Progress>,
    result: mpsc::Receiver<io::Result<Option<Tree>>>,
    started: Instant,
}

struct App {
    path_input: String,
    drives: Vec<String>,
    show_files: bool,
    side_tab: SideTab,
    job: Option<ScanJob>,
    /// Kept while a rescan runs, so cancelling restores it.
    view: Option<View>,
    message: Option<String>,
    /// Not elevated on Windows: unreadable folders may become readable after a restart.
    can_elevate: bool,
}

impl App {
    fn new(ctx: &egui::Context, initial: Option<PathBuf>) -> Self {
        ctx.all_styles_mut(|style| style.interaction.selectable_labels = false);
        let mut app = Self {
            path_input: String::new(),
            drives: drives(),
            show_files: true,
            side_tab: SideTab::Treemap,
            job: None,
            view: None,
            message: None,
            can_elevate: crate::elevation::can_relaunch_elevated(),
        };
        if let Some(path) = initial {
            app.path_input = path.display().to_string();
            app.start_scan(ctx);
        }
        app
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        let input = self.path_input.trim().trim_matches('"');
        if input.is_empty() {
            return;
        }
        if let Some(old) = self.job.take() {
            old.progress.cancel();
        }
        let root = crate::normalize_path(Path::new(input));
        self.path_input = root.display().to_string();

        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let worker = {
            let (root, progress, ctx) = (root.clone(), progress.clone(), ctx.clone());
            move || {
                let _ = tx.send(scan::scan(&root, &progress));
                ctx.request_repaint();
            }
        };
        match std::thread::Builder::new().name("scan".into()).spawn(worker) {
            Ok(_) => {
                self.message = None;
                self.job = Some(ScanJob { root, progress, result: rx, started: Instant::now() });
            }
            Err(e) => self.message = Some(format!("Cannot start scan: {e}")),
        }
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        let outcome = match job.result.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(io::Error::other("scan thread panicked")),
        };
        let root = self.job.take().map(|j| j.root).unwrap_or_default();
        match outcome {
            Ok(Some(tree)) => self.view = Some(View::new(tree)),
            Ok(None) => self.message = Some("Scan cancelled.".into()),
            Err(e) => self.message = Some(format!("Cannot scan {}: {e}", root.display())),
        }
    }

    fn apply(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        let Some(view) = &mut self.view else { return };
        for action in actions {
            match action {
                Action::Select(id) => view.select(id, true, false),
                Action::Reveal(id) => view.select(id, false, true),
                Action::Focus(id) => view.select(id, true, true),
                Action::Toggle(id) => view.toggle(id),
                Action::Sort(key) => view.set_sort(key),
                Action::OpenInFileManager(id) => {
                    if let Err(e) = open_in_file_manager(&view.tree.path(id)) {
                        self.message = Some(format!("Cannot open file manager: {e}"));
                    }
                }
                Action::CopyPath(id) => ctx.copy_text(view.tree.path(id).display().to_string()),
            }
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let mut start = false;
        ui.horizontal(|ui| {
            ui.label("Folder");
            let edit = ui.add(
                TextEdit::singleline(&mut self.path_input)
                    .hint_text(if cfg!(windows) { r"e.g. C:\ or D:\Projects" } else { "e.g. /home" })
                    .desired_width((ui.available_width() - 330.0).max(160.0)),
            );
            start |= edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));

            if ui.button("Browse…").clicked() {
                let mut dialog = rfd::FileDialog::new();
                let current = Path::new(self.path_input.trim());
                if current.is_dir() {
                    dialog = dialog.set_directory(current);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.path_input = dir.display().to_string();
                    start = true;
                }
            }
            ui.menu_button("Drives", |ui| {
                for drive in &self.drives {
                    if ui.button(drive).clicked() {
                        self.path_input = drive.clone();
                        start = true;
                        ui.close();
                    }
                }
            });

            match &self.job {
                Some(job) => {
                    let label = if job.progress.is_cancelled() { "Cancelling…" } else { "Cancel" };
                    if ui.button(label).clicked() {
                        job.progress.cancel();
                    }
                }
                None => {
                    let label = if self.view.is_some() { "Rescan" } else { "Scan" };
                    let enabled = !self.path_input.trim().is_empty();
                    start |= ui.add_enabled(enabled, Button::new(RichText::new(label).strong())).clicked();
                }
            }

            if ui.checkbox(&mut self.show_files, "Show files").changed()
                && let Some(view) = &mut self.view
            {
                view.rows_dirty = true;
                view.scroll_to_selected = true;
            }
        });
        if start {
            self.start_scan(ui.ctx());
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let mut restart = false;
        ui.horizontal(|ui| {
            if let Some(job) = &self.job {
                let p = &job.progress;
                ui.spinner();
                ui.label(format!(
                    "Scanning… {} files · {} folders · {}",
                    format::count(p.files.load(std::sync::atomic::Ordering::Relaxed)),
                    format::count(p.dirs.load(std::sync::atomic::Ordering::Relaxed)),
                    format::size(p.bytes.load(std::sync::atomic::Ordering::Relaxed)),
                ));
            } else if let Some(view) = &self.view {
                let tree = &view.tree;
                let root = tree.node(ROOT);
                ui.label(format!(
                    "{} ({} on disk) · {} files · {} folders · scanned in {:.1?}",
                    format::size(root.size),
                    format::size(root.allocated),
                    format::count(root.files),
                    format::count(root.dirs),
                    tree.elapsed,
                ));
                if tree.errors > 0 {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        match tree.errors {
                            1 => "⚠ 1 entry could not be read".to_owned(),
                            n => format!("⚠ {} entries could not be read", format::count(n)),
                        },
                    )
                    .on_hover_text("Usually protected system folders.");
                    if self.can_elevate {
                        restart = ui
                            .button("🛡 Restart as administrator")
                            .on_hover_text("Rescan this folder with administrator rights")
                            .clicked();
                    }
                }
            }
            if let Some(msg) = &self.message {
                ui.separator();
                ui.colored_label(ui.visuals().error_fg_color, msg);
            }
        });
        if restart {
            self.restart_elevated(ui.ctx());
        }
    }

    /// Relaunches through UAC, rescanning the current folder, and closes this instance.
    fn restart_elevated(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            let root = self.view.as_ref().map(|v| v.tree.root_path.clone());
            let args: Vec<&std::ffi::OsStr> = root.iter().map(|p| p.as_os_str()).collect();
            match crate::elevation::relaunch_elevated(&args) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.message = Some(format!("Not restarted: {e}")),
            }
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_scan(&ctx);
        let mut actions = Vec::new();

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));

        if let Some(job) = &self.job {
            egui::CentralPanel::default().show(ui, |ui| scan_progress(ui, job));
            return;
        }
        let Some(view) = &mut self.view else {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Choose a folder or drive, then press Scan.").size(18.0).weak());
                });
            });
            return;
        };

        let side_tab = &mut self.side_tab;
        egui::Panel::right("side")
            .resizable(true)
            .default_size(440.0)
            .size_range(260.0..=1400.0)
            .show(ui, |ui| side_panel(ui, view, side_tab, &mut actions));
        egui::CentralPanel::default().show(ui, |ui| {
            view.handle_keys(ui, self.show_files, &mut actions);
            tree_table(ui, view, self.show_files, &mut actions);
        });

        self.apply(&ctx, actions);
    }
}

// ---------------------------------------------------------------------------------------------
// View state

struct View {
    tree: Tree,
    expanded: Vec<bool>,
    /// Visible tree rows: node and indentation depth.
    rows: Vec<(NodeId, u16)>,
    rows_dirty: bool,
    sort: SortKey,
    descending: bool,
    /// Bumped whenever the sort order changes; children are re-sorted lazily on expansion.
    sort_epoch: u32,
    sorted_epoch: Vec<u32>,
    selected: NodeId,
    scroll_to_selected: bool,
    /// Directory shown by the side panel.
    focus: NodeId,
    stats: Option<(NodeId, SubtreeStats)>,
    treemap: Option<Treemap>,
}

impl View {
    fn new(tree: Tree) -> Self {
        let n = tree.len();
        let mut expanded = vec![false; n];
        expanded[ROOT as usize] = true;
        Self {
            tree,
            expanded,
            rows: Vec::new(),
            rows_dirty: true,
            sort: SortKey::Allocated,
            descending: true,
            sort_epoch: 1,
            sorted_epoch: vec![0; n],
            selected: ROOT,
            scroll_to_selected: false,
            focus: ROOT,
            stats: None,
            treemap: None,
        }
    }

    fn rebuild_rows(&mut self, show_files: bool) {
        self.rows.clear();
        let mut stack = vec![(ROOT, 0u16)];
        while let Some((id, depth)) = stack.pop() {
            self.rows.push((id, depth));
            if !self.expanded[id as usize] {
                continue;
            }
            if self.sorted_epoch[id as usize] != self.sort_epoch {
                self.tree.sort_children(id, self.sort, self.descending);
                self.sorted_epoch[id as usize] = self.sort_epoch;
            }
            let children = self.tree.children(id);
            stack.extend(
                children
                    .iter()
                    .rev()
                    .filter(|&&c| show_files || self.tree.node(c).is_dir)
                    .map(|&c| (c, depth + 1)),
            );
        }
        self.rows_dirty = false;
    }

    fn select(&mut self, id: NodeId, follow_focus: bool, reveal: bool) {
        self.selected = id;
        if follow_focus {
            self.focus = self.tree.dir_of(id);
        }
        if reveal {
            for a in self.tree.ancestors_inclusive(id).skip(1) {
                if !self.expanded[a as usize] {
                    self.expanded[a as usize] = true;
                    self.rows_dirty = true;
                }
            }
            self.scroll_to_selected = true;
        }
    }

    fn toggle(&mut self, id: NodeId) {
        let e = &mut self.expanded[id as usize];
        *e = !*e;
        self.rows_dirty = true;
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.sort == key {
            self.descending = !self.descending;
        } else {
            self.sort = key;
            self.descending = key != SortKey::Name;
        }
        self.sort_epoch += 1;
        self.rows_dirty = true;
        self.scroll_to_selected = true;
    }

    fn has_visible_children(&self, id: NodeId, show_files: bool) -> bool {
        let node = self.tree.node(id);
        node.dirs > 0 || (show_files && !self.tree.children(id).is_empty())
    }

    /// Arrow keys, Home/End and PageUp/PageDown navigate the tree like Explorer.
    fn handle_keys(&mut self, ui: &Ui, show_files: bool, actions: &mut Vec<Action>) {
        if ui.memory(|m| m.focused().is_some()) {
            return;
        }
        let keys = ui.input(|i| {
            [Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight, Key::Home, Key::End, Key::PageUp, Key::PageDown]
                .map(|k| i.key_pressed(k))
        });
        let [up, down, left, right, home, end, page_up, page_down] = keys;
        if !keys.contains(&true) {
            return;
        }
        if self.rows_dirty {
            self.rebuild_rows(show_files);
        }
        let position = |id| self.rows.iter().position(|&(r, _)| r == id);
        let Some(pos) = position(self.selected).or_else(|| position(self.tree.dir_of(self.selected))) else {
            return;
        };
        let last = self.rows.len() - 1;
        let id = self.rows[pos].0;
        let target = if up {
            pos.saturating_sub(1)
        } else if down {
            (pos + 1).min(last)
        } else if page_up {
            pos.saturating_sub(PAGE_ROWS)
        } else if page_down {
            (pos + PAGE_ROWS).min(last)
        } else if home {
            0
        } else if end {
            last
        } else if right && self.has_visible_children(id, show_files) {
            if self.expanded[id as usize] {
                (pos + 1).min(last)
            } else {
                actions.push(Action::Toggle(id));
                pos
            }
        } else if left {
            if self.expanded[id as usize] && id != ROOT {
                actions.push(Action::Toggle(id));
                pos
            } else {
                self.tree.node(id).parent().and_then(position).unwrap_or(pos)
            }
        } else {
            pos
        };
        if target != pos || id != self.selected {
            actions.push(Action::Select(self.rows[target].0));
            self.scroll_to_selected = true;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Central panel: tree table

fn tree_table(ui: &mut Ui, view: &mut View, show_files: bool, actions: &mut Vec<Action>) {
    if view.rows_dirty {
        view.rebuild_rows(show_files);
    }
    let scroll_row = if std::mem::take(&mut view.scroll_to_selected) {
        view.rows.iter().position(|&(id, _)| id == view.selected)
    } else {
        None
    };

    let mut table = TableBuilder::new(ui)
        .id_salt("tree")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(300.0).at_least(160.0).clip(true))
        .column(Column::initial(84.0).at_least(60.0))
        .column(Column::initial(84.0).at_least(60.0))
        .column(Column::initial(110.0).at_least(70.0))
        .column(Column::initial(84.0).at_least(50.0))
        .column(Column::initial(70.0).at_least(50.0))
        .column(Column::remainder().at_least(110.0));
    if let Some(row) = scroll_row {
        table = table.scroll_to_row(row, Some(Align::Center));
    }

    let view = &*view;
    let tree = &view.tree;
    table
        .header(22.0, |mut header| {
            let sort = (view.sort, view.descending);
            sort_header(&mut header, "Name", SortKey::Name, sort, false, actions);
            sort_header(&mut header, "Size", SortKey::Size, sort, true, actions);
            sort_header(&mut header, "Allocated", SortKey::Allocated, sort, true, actions);
            header.col(|ui| {
                ui.strong("% of parent");
            });
            sort_header(&mut header, "Files", SortKey::Files, sort, true, actions);
            sort_header(&mut header, "Folders", SortKey::Folders, sort, true, actions);
            sort_header(&mut header, "Last change", SortKey::Modified, sort, false, actions);
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, view.rows.len(), |mut row| {
                let (id, depth) = view.rows[row.index()];
                let node = tree.node(id);
                row.set_selected(id == view.selected);

                row.col(|ui| {
                    ui.add_space(depth as f32 * INDENT);
                    let (_, toggle) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
                    if view.has_visible_children(id, show_files) {
                        let openness = if view.expanded[id as usize] { 1.0 } else { 0.0 };
                        egui::collapsing_header::paint_default_icon(ui, openness, &toggle);
                        if toggle.clicked() {
                            actions.push(Action::Toggle(id));
                        }
                    }
                    let icon = if node.is_dir { "🗁" } else { "🗋" };
                    let mut text = RichText::new(format!("{icon} {}", tree.name(id)));
                    if node.is_dir {
                        text = text.strong();
                    }
                    ui.add(egui::Label::new(text).truncate());
                    if node.error {
                        ui.colored_label(ui.visuals().warn_fg_color, "⚠")
                            .on_hover_text("Some contents could not be read");
                    }
                });
                row.col(|ui| right_label(ui, format::size(node.size)));
                row.col(|ui| right_label(ui, format::size(node.allocated)));
                row.col(|ui| {
                    let parent_allocated = node.parent().map_or(node.allocated, |p| tree.node(p).allocated);
                    percent_bar(ui, format::fraction(node.allocated, parent_allocated));
                });
                row.col(|ui| {
                    if node.is_dir {
                        right_label(ui, format::count(node.files));
                    }
                });
                row.col(|ui| {
                    if node.is_dir {
                        right_label(ui, format::count(node.dirs));
                    }
                });
                row.col(|ui| {
                    if let Some(t) = node.modified() {
                        ui.label(format::date(t));
                    }
                });

                let response = row.response();
                if response.clicked() {
                    actions.push(Action::Select(id));
                }
                if response.double_clicked() && node.is_dir && id != ROOT {
                    actions.push(Action::Toggle(id));
                }
                response.context_menu(|ui| node_menu(ui, tree, id, actions));
            });
        });
}

fn sort_header(
    row: &mut TableRow<'_, '_>,
    label: &str,
    key: SortKey,
    (current, descending): (SortKey, bool),
    align_right: bool,
    actions: &mut Vec<Action>,
) {
    row.col(|ui| {
        let text = match (current == key, descending) {
            (true, true) => format!("{label} ⬇"),
            (true, false) => format!("{label} ⬆"),
            (false, _) => label.to_owned(),
        };
        let layout = if align_right { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
        ui.with_layout(layout, |ui| {
            if ui.add(Button::new(RichText::new(text).strong()).frame(false)).clicked() {
                actions.push(Action::Sort(key));
            }
        });
    });
}

fn node_menu(ui: &mut Ui, tree: &Tree, id: NodeId, actions: &mut Vec<Action>) {
    if ui.button("Open in file manager").clicked() {
        actions.push(Action::OpenInFileManager(id));
        ui.close();
    }
    if ui.button("Copy path").clicked() {
        actions.push(Action::CopyPath(id));
        ui.close();
    }
    if tree.node(id).is_dir && ui.button("Show in side panel").clicked() {
        actions.push(Action::Focus(id));
        ui.close();
    }
}

fn right_label(ui: &mut Ui, text: String) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(text));
}

fn percent_bar(ui: &mut Ui, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let filled = Rect::from_min_size(rect.min, vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()));
    painter.rect_filled(filled, 2.0, visuals.selection.bg_fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("{:.1} %", fraction * 100.0),
        FontId::proportional(11.0),
        visuals.strong_text_color(),
    );
}

fn scan_progress(ui: &mut Ui, job: &ScanJob) {
    use std::sync::atomic::Ordering::Relaxed;
    let p = &job.progress;
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.25);
        ui.add(egui::Spinner::new().size(36.0));
        ui.add_space(12.0);
        ui.heading(format!("Scanning {}", job.root.display()));
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "{}   ·   {} files   ·   {} folders   ·   {:.0?}",
                format::size(p.bytes.load(Relaxed)),
                format::count(p.files.load(Relaxed)),
                format::count(p.dirs.load(Relaxed)),
                job.started.elapsed(),
            ))
            .size(16.0),
        );
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(p.current()).weak()).truncate());
    });
}

// ---------------------------------------------------------------------------------------------
// Side panel: treemap, file types, largest files for the focused directory

fn side_panel(ui: &mut Ui, view: &mut View, tab: &mut SideTab, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.selectable_value(tab, SideTab::Treemap, "Treemap");
        ui.selectable_value(tab, SideTab::Types, "File types");
        ui.selectable_value(tab, SideTab::Largest, "Largest files");
    });
    ui.horizontal(|ui| {
        let parent = view.tree.node(view.focus).parent();
        if ui.add_enabled(parent.is_some(), Button::new("⬆")).on_hover_text("Parent folder").clicked()
            && let Some(p) = parent
        {
            actions.push(Action::Focus(p));
        }
        let focus = view.tree.node(view.focus);
        ui.strong(format::size(focus.allocated)).on_hover_text("Allocated (on disk)");
        ui.add(egui::Label::new(view.tree.path(view.focus).display().to_string()).truncate());
    });
    ui.separator();

    if *tab != SideTab::Treemap && view.stats.as_ref().is_none_or(|(f, _)| *f != view.focus) {
        view.stats = Some((view.focus, view.tree.subtree_stats(view.focus, LARGEST_FILES)));
    }
    match tab {
        SideTab::Treemap => treemap_panel(ui, view, actions),
        SideTab::Types => types_table(ui, view),
        SideTab::Largest => largest_table(ui, view, actions),
    }
}

fn treemap_panel(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click());
    let rect = response.rect;
    let tree = &view.tree;
    if tree.node(view.focus).allocated == 0 {
        painter.text(rect.center(), Align2::CENTER_CENTER, "Empty", FontId::proportional(16.0), ui.visuals().weak_text_color());
        return;
    }
    if view.treemap.as_ref().is_none_or(|m| m.focus != view.focus || m.rect != rect) {
        view.treemap = Some(Treemap::new(tree, view.focus, rect));
    }
    let Some(map) = &view.treemap else { return };

    let hovered = response.hover_pos().and_then(|p| map.hit(p)).map(|c| c.id);
    map.paint(&painter, tree, Some(view.selected), hovered);

    if response.double_clicked() {
        // Zoom one level: the focus child containing the clicked cell.
        let child = hovered.and_then(|h| {
            tree.ancestors_inclusive(h).find(|&a| tree.node(a).parent() == Some(view.focus))
        });
        if let Some(c) = child.filter(|&c| tree.node(c).is_dir) {
            actions.push(Action::Focus(c));
        }
    } else if response.clicked()
        && let Some(id) = hovered
    {
        actions.push(Action::Reveal(id));
    }
    if response.secondary_clicked()
        && let Some(parent) = tree.node(view.focus).parent()
    {
        actions.push(Action::Focus(parent));
    }

    if let Some(id) = hovered {
        let focus_allocated = tree.node(view.focus).allocated;
        response
            .on_hover_ui_at_pointer(|ui| {
                let node = tree.node(id);
                ui.strong(tree.path(id).display().to_string());
                ui.label(format!(
                    "{} on disk  ({:.1} % of view) · {} size",
                    format::size(node.allocated),
                    format::fraction(node.allocated, focus_allocated) * 100.0,
                    format::size(node.size),
                ));
                if node.is_dir {
                    ui.label(format!("{} files · {} folders", format::count(node.files), format::count(node.dirs)));
                }
                ui.weak("Click: select · Double-click: zoom in · Right-click: zoom out");
            });
    }
}

fn types_table(ui: &mut Ui, view: &View) {
    let Some((_, stats)) = &view.stats else { return };
    let total = view.tree.node(view.focus).allocated;
    TableBuilder::new(ui)
        .id_salt("types")
        .striped(true)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(110.0).at_least(60.0).clip(true))
        .column(Column::initial(84.0))
        .column(Column::initial(130.0))
        .column(Column::remainder().at_least(60.0))
        .header(20.0, |mut header| {
            for title in ["Type", "Allocated", "% of folder", "Files"] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, stats.extensions.len(), |mut row| {
                let ext = &stats.extensions[row.index()];
                row.col(|ui| {
                    if ext.ext.is_empty() {
                        ui.weak("(no extension)");
                    } else {
                        ui.label(format!(".{}", ext.ext));
                    }
                });
                row.col(|ui| right_label(ui, format::size(ext.bytes)));
                row.col(|ui| percent_bar(ui, format::fraction(ext.bytes, total)));
                row.col(|ui| right_label(ui, format::count(ext.files)));
            });
        });
}

fn largest_table(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    let Some((_, stats)) = &view.stats else { return };
    let tree = &view.tree;
    let base = tree.path(view.focus);
    TableBuilder::new(ui)
        .id_salt("largest")
        .striped(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(200.0).at_least(80.0).clip(true))
        .column(Column::initial(84.0))
        .column(Column::initial(84.0))
        .column(Column::remainder().at_least(80.0).clip(true))
        .header(20.0, |mut header| {
            for title in ["Name", "Allocated", "Size", "Folder"] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, stats.largest.len(), |mut row| {
                let id = stats.largest[row.index()];
                let node = tree.node(id);
                row.set_selected(id == view.selected);
                row.col(|ui| {
                    ui.add(egui::Label::new(tree.name(id)).truncate());
                });
                row.col(|ui| right_label(ui, format::size(node.allocated)));
                row.col(|ui| right_label(ui, format::size(node.size)));
                row.col(|ui| {
                    let folder = tree.path(tree.dir_of(id));
                    let relative = folder.strip_prefix(&base).unwrap_or(&folder);
                    let shown = if relative.as_os_str().is_empty() { Path::new(".") } else { relative };
                    ui.add(egui::Label::new(RichText::new(shown.display().to_string()).weak()).truncate());
                });
                let response = row.response();
                if response.clicked() {
                    actions.push(Action::Reveal(id));
                }
                response.context_menu(|ui| node_menu(ui, tree, id, actions));
            });
        });
}

// ---------------------------------------------------------------------------------------------
// Platform helpers

fn drives() -> Vec<String> {
    #[cfg(windows)]
    {
        // SAFETY: no arguments; returns a bitmask of existing drive letters without touching the disks.
        let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| format!("{}:\\", (b'A' + i) as char))
            .collect()
    }
    #[cfg(not(windows))]
    {
        let mut roots = vec!["/".to_owned()];
        roots.extend(std::env::var("HOME").ok());
        roots
    }
}

fn open_in_file_manager(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `/select,` must be glued to the quoted path in a single argument.
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg("-R").arg(path).spawn()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    std::process::Command::new("xdg-open")
        .arg(if path.is_dir() { path } else { path.parent().unwrap_or(path) })
        .spawn()?;
    Ok(())
}
