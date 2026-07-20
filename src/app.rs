//! Application state and the eframe update loop that stitches the panels
//! together: menu bar, tree, basket canvas, search, and all file operations.

use crate::canvas::{self, CanvasAction, Env};
use crate::images::TextureCache;
use crate::model::{CardKind, ChecklistItem, Document, NodeId};
use crate::tree::{self, TreeAction};
use egui_commonmark::CommonMarkCache;
use std::collections::HashMap;
use std::path::PathBuf;

const MIN_CARD: egui::Vec2 = egui::Vec2::new(140.0, 90.0);

/// eframe storage key holding the path of the document open at last exit.
const LAST_DOC_KEY: &str = "last_doc_path";

pub struct TrellisApp {
    doc: Document,
    selected: Option<NodeId>,
    /// Per-node canvas pan offset, so each basket remembers its scroll.
    pans: HashMap<NodeId, egui::Vec2>,
    md_cache: CommonMarkCache,
    tex_cache: TextureCache,
    renaming: Option<(NodeId, String)>,

    /// Path of the document on disk, if any. `None` = never saved.
    doc_path: Option<PathBuf>,
    /// Fallback autosave location used when the document is untitled.
    autosave_path: PathBuf,
    dirty: bool,
    status: String,

    search_open: bool,
    search_query: String,
    show_about: bool,
    dark: bool,
    zoom: f32,
}

impl TrellisApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        setup_fonts(&cc.egui_ctx);
        let autosave_path = default_autosave_path();

        // Reopen the document from the last session if possible; otherwise fall
        // back to the autosave slot, then to a fresh welcome document.
        let last_path = cc
            .storage
            .and_then(|s| s.get_string(LAST_DOC_KEY))
            .map(PathBuf::from);
        let mut doc_path: Option<PathBuf> = None;
        let mut doc: Option<Document> = None;
        if let Some(p) = &last_path {
            if let Ok(d) = std::fs::read_to_string(p)
                .map_err(|_| ())
                .and_then(|s| ron::from_str::<Document>(&s).map_err(|_| ()))
            {
                doc = Some(d);
                doc_path = Some(p.clone());
            }
        }
        let doc = doc
            .or_else(|| {
                std::fs::read_to_string(&autosave_path)
                    .ok()
                    .and_then(|s| ron::from_str::<Document>(&s).ok())
            })
            .unwrap_or_default();
        let selected = doc.roots.first().copied();

        cc.egui_ctx.style_mut(|s| {
            s.visuals.window_rounding = 8.0.into();
        });

        Self {
            doc,
            selected,
            pans: HashMap::new(),
            md_cache: CommonMarkCache::default(),
            tex_cache: TextureCache::default(),
            renaming: None,
            doc_path,
            autosave_path,
            dirty: false,
            status: "Ready".to_string(),
            search_open: false,
            search_query: String::new(),
            show_about: false,
            dark: true,
            zoom: 1.0,
        }
    }

    // --- persistence --------------------------------------------------------

    fn target_path(&self) -> PathBuf {
        self.doc_path.clone().unwrap_or_else(|| self.autosave_path.clone())
    }

    fn write_to(&mut self, path: PathBuf) {
        match ron::ser::to_string_pretty(&self.doc, ron::ser::PrettyConfig::default()) {
            Ok(s) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::write(&path, s) {
                    Ok(_) => {
                        self.dirty = false;
                        self.status = format!("Saved → {}", path.display());
                    }
                    Err(e) => self.status = format!("Save failed: {e}"),
                }
            }
            Err(e) => self.status = format!("Serialize failed: {e}"),
        }
    }

    fn save(&mut self) {
        let path = self.target_path();
        self.write_to(path);
    }

    fn save_as(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Trellis document", &["ron"])
            .set_file_name("untitled.ron")
            .save_file()
        {
            self.doc_path = Some(path.clone());
            self.write_to(path);
        }
    }

    fn confirm_discard(&self) -> bool {
        if !self.dirty {
            return true;
        }
        matches!(
            rfd::MessageDialog::new()
                .set_title("Unsaved changes")
                .set_description("Discard the current document?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show(),
            rfd::MessageDialogResult::Yes
        )
    }

    fn new_document(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        self.doc = Document::default();
        self.selected = self.doc.roots.first().copied();
        self.pans.clear();
        self.doc_path = None;
        self.dirty = false;
        self.status = "New document".to_string();
    }

    fn open_document(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Trellis document", &["ron"])
            .pick_file()
        {
            match std::fs::read_to_string(&path).map(|s| ron::from_str::<Document>(&s)) {
                Ok(Ok(doc)) => {
                    self.doc = doc;
                    self.selected = self.doc.roots.first().copied();
                    self.pans.clear();
                    self.doc_path = Some(path.clone());
                    self.dirty = false;
                    self.status = format!("Opened {}", path.display());
                }
                Ok(Err(e)) => self.status = format!("Parse error: {e}"),
                Err(e) => self.status = format!("Read error: {e}"),
            }
        }
    }

    fn import(&mut self, html: bool) {
        let (label, exts): (&str, &[&str]) = if html {
            ("HTML", &["html", "htm"])
        } else {
            ("Markdown", &["md", "markdown", "txt"])
        };
        if let Some(path) = rfd::FileDialog::new().add_filter(label, exts).pick_file() {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let title = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Imported".to_string());
                    let id = self.doc.import_as_node(title, &content, html);
                    self.selected = Some(id);
                    self.dirty = true;
                    self.status = format!("Imported {} as a node", label);
                }
                Err(e) => self.status = format!("Read error: {e}"),
            }
        }
    }

    fn export_html(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("HTML", &["html"])
            .set_file_name("trellis-export.html")
            .save_file()
        {
            match std::fs::write(&path, self.doc.export_html()) {
                Ok(_) => self.status = format!("Exported HTML → {}", path.display()),
                Err(e) => self.status = format!("Export failed: {e}"),
            }
        }
    }

    fn export_json(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_file_name("trellis-export.json")
            .save_file()
        {
            match self.doc.export_json() {
                Ok(s) => match std::fs::write(&path, s) {
                    Ok(_) => self.status = format!("Exported JSON → {}", path.display()),
                    Err(e) => self.status = format!("Export failed: {e}"),
                },
                Err(e) => self.status = format!("Serialize failed: {e}"),
            }
        }
    }

    // --- action application -------------------------------------------------

    fn apply_tree(&mut self, actions: Vec<TreeAction>) {
        // Selection isn't persisted, so a pure Select must not dirty the doc.
        if actions.iter().any(|a| !matches!(a, TreeAction::Select(_))) {
            self.dirty = true;
        }
        for a in actions {
            match a {
                TreeAction::Select(id) => self.selected = Some(id),
                TreeAction::AddRoot => {
                    let id = self.doc.add_node(None, "Untitled".to_string());
                    self.selected = Some(id);
                    self.renaming = Some((id, "Untitled".to_string()));
                }
                TreeAction::AddChild(parent) => {
                    let id = self.doc.add_node(Some(parent), "Untitled".to_string());
                    if let Some(n) = self.doc.nodes.get_mut(&parent) {
                        n.expanded = true;
                    }
                    self.selected = Some(id);
                    self.renaming = Some((id, "Untitled".to_string()));
                }
                TreeAction::AddSibling(sib) => {
                    let id = self.doc.add_sibling(sib, "Untitled".to_string());
                    self.selected = Some(id);
                    self.renaming = Some((id, "Untitled".to_string()));
                }
                TreeAction::Remove(id) => {
                    self.doc.remove_node(id);
                    if self.selected == Some(id) {
                        self.selected = self.doc.roots.first().copied();
                    }
                }
                TreeAction::Rename(id, title) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.title = title;
                    }
                }
                TreeAction::ToggleExpand(id) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.expanded = !n.expanded;
                    }
                }
                TreeAction::MoveUp(id) => self.doc.move_sibling(id, true),
                TreeAction::MoveDown(id) => self.doc.move_sibling(id, false),
                TreeAction::Indent(id) => self.doc.indent(id),
                TreeAction::Outdent(id) => self.doc.outdent(id),
                TreeAction::SetColor(id, col) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.color = col;
                    }
                }
            }
        }
    }

    fn apply_canvas(&mut self, node: NodeId, actions: Vec<CanvasAction>) {
        // ResetView only nudges the (unsaved) pan, so it must not dirty the doc.
        if actions.iter().any(|a| !matches!(a, CanvasAction::ResetView)) {
            self.dirty = true;
        }
        for a in actions {
            match a {
                CanvasAction::AddCard(kind, pos) => {
                    self.doc.add_card(node, pos, kind);
                }
                CanvasAction::MoveCard(cid, delta) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.pos += delta;
                    }
                }
                CanvasAction::ResizeCard(cid, delta) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.size = (c.size + delta).max(MIN_CARD);
                    }
                }
                CanvasAction::RaiseCard(cid) => self.doc.raise_card(node, cid),
                CanvasAction::SetTitle(cid, t) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.title = t;
                    }
                }
                CanvasAction::SetBody(cid, b) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.body = b;
                    }
                }
                CanvasAction::SetLang(cid, lang) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Code { lang: l } = &mut c.kind {
                            *l = lang;
                        }
                    }
                }
                CanvasAction::SetColor(cid, col) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.color = col;
                    }
                }
                CanvasAction::SetEditing(cid, ed) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.editing = ed;
                    }
                }
                CanvasAction::Duplicate(cid) => {
                    self.doc.duplicate_card(node, cid);
                }
                CanvasAction::Remove(cid) => {
                    self.doc.remove_card(node, cid);
                    self.tex_cache.forget(cid);
                }
                CanvasAction::ChecklistToggle(cid, i) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            if let Some(it) = items.get_mut(i) {
                                it.done = !it.done;
                            }
                        }
                    }
                }
                CanvasAction::ChecklistSetText(cid, i, text) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            if let Some(it) = items.get_mut(i) {
                                it.text = text;
                            }
                        }
                    }
                }
                CanvasAction::ChecklistAdd(cid) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            items.push(ChecklistItem { done: false, text: String::new() });
                        }
                    }
                }
                CanvasAction::ChecklistRemove(cid, i) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            if i < items.len() {
                                items.remove(i);
                            }
                        }
                    }
                }
                CanvasAction::LoadImage(cid) => self.load_image_into(node, cid),
                CanvasAction::ResetView => {
                    self.pans.insert(node, egui::Vec2::ZERO);
                }
            }
        }
    }

    fn load_image_into(&mut self, node: NodeId, card: crate::model::CardId) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "webp"])
            .pick_file()
        {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if let Some(c) = self.doc.card_mut(node, card) {
                        c.kind = CardKind::Image { data: bytes, name };
                    }
                    self.tex_cache.forget(card);
                    self.status = "Loaded image".to_string();
                }
                Err(e) => self.status = format!("Image read error: {e}"),
            }
        }
    }

    // --- panels -------------------------------------------------------------

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New").clicked() {
                        self.new_document();
                        ui.close_menu();
                    }
                    if ui.button("Open…").clicked() {
                        self.open_document();
                        ui.close_menu();
                    }
                    if ui.button("Save").clicked() {
                        self.save();
                        ui.close_menu();
                    }
                    if ui.button("Save As…").clicked() {
                        self.save_as();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Import Markdown…").clicked() {
                        self.import(false);
                        ui.close_menu();
                    }
                    if ui.button("Import HTML…").clicked() {
                        self.import(true);
                        ui.close_menu();
                    }
                    if ui.button("Export HTML…").clicked() {
                        self.export_html();
                        ui.close_menu();
                    }
                    if ui.button("Export JSON…").clicked() {
                        self.export_json();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Add root node").clicked() {
                        self.apply_tree(vec![TreeAction::AddRoot]);
                        ui.close_menu();
                    }
                    if let Some(sel) = self.selected {
                        if ui.button("Add child to selected").clicked() {
                            self.apply_tree(vec![TreeAction::AddChild(sel)]);
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button(if self.dark { "Light theme" } else { "Dark theme" }).clicked() {
                        self.dark = !self.dark;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Zoom in").clicked() {
                        self.zoom = (self.zoom + 0.1).min(3.0);
                    }
                    if ui.button("Zoom out").clicked() {
                        self.zoom = (self.zoom - 0.1).max(0.5);
                    }
                    if ui.button("Reset zoom").clicked() {
                        self.zoom = 1.0;
                    }
                    ui.separator();
                    if ui.button("Find… (Ctrl+F)").clicked() {
                        self.search_open = !self.search_open;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About Trellis").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn search_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("search")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Search");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").clicked() {
                            self.search_open = false;
                        }
                    });
                });
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("find text…")
                        .desired_width(f32::INFINITY),
                );
                resp.request_focus();
                ui.separator();
                let hits = self.doc.search(&self.search_query);
                if self.search_query.is_empty() {
                    ui.weak("Type to search titles and card contents.");
                } else {
                    ui.weak(format!("{} match(es)", hits.len()));
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for hit in hits {
                        let frame = egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(6.0, 4.0));
                        frame.show(ui, |ui| {
                            if ui
                                .add(egui::Label::new(egui::RichText::new(&hit.node_title).strong())
                                    .sense(egui::Sense::click()))
                                .clicked()
                            {
                                self.selected = Some(hit.node);
                            }
                            ui.small(hit.snippet);
                        });
                        ui.separator();
                    }
                });
            });
    }
}

impl eframe::App for TrellisApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Theme + zoom.
        ctx.set_visuals(if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        ctx.set_zoom_factor(self.zoom);

        // Keyboard shortcuts.
        ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(egui::Key::S) {
                // handled after borrow ends via flag
            }
        });
        let cmd = ctx.input(|i| i.modifiers.command);
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::S)) {
            self.save();
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::F)) {
            self.search_open = !self.search_open;
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::N)) {
            self.new_document();
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) {
            self.zoom = (self.zoom + 0.1).min(3.0);
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::Minus)) {
            self.zoom = (self.zoom - 0.1).max(0.5);
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::Num0)) {
            self.zoom = 1.0;
        }

        self.menu_bar(ctx);

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    self.save();
                }
                ui.separator();
                let title = self
                    .doc_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "untitled".to_string());
                let mark = if self.dirty { "●" } else { "" };
                ui.label(format!("{mark} {title}"));
                ui.separator();
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                });
            });
        });

        if self.search_open {
            self.search_panel(ctx);
        }

        egui::SidePanel::left("tree")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                let actions = tree::ui(ui, &self.doc, self.selected, &mut self.renaming);
                self.apply_tree(actions);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(sel) = self.selected {
                if self.doc.nodes.contains_key(&sel) {
                    let mut pan = self.pans.get(&sel).copied().unwrap_or_default();
                    let node = self.doc.nodes.get(&sel).unwrap();
                    let mut env = Env {
                        md: &mut self.md_cache,
                        tex: &mut self.tex_cache,
                    };
                    let actions = canvas::ui(ui, node, &mut pan, &mut env);
                    self.pans.insert(sel, pan);
                    self.apply_canvas(sel, actions);
                } else {
                    self.selected = None;
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("No node selected. Add one on the left to start a basket.");
                });
            }
        });

        if self.show_about {
            egui::Window::new("About Trellis")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.heading("Trellis");
                    ui.label("A hierarchical, spatial note-taking app.");
                    ui.add_space(4.0);
                    ui.label("A tree of nodes, where every node is a free-form basket of cards.");
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        self.show_about = false;
                    }
                });
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Remember which file to reopen next launch (untitled docs live in the
        // autosave slot and need no key).
        if let Some(p) = &self.doc_path {
            storage.set_string(LAST_DOC_KEY, p.display().to_string());
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Best-effort autosave to the working file (or the autosave slot).
        let path = self.target_path();
        self.write_to(path);
    }
}

/// Install DejaVu as the primary UI font. It carries the arrows, bullets,
/// dashes and box-drawing that egui's default fonts lack, so UI glyphs and the
/// wide Unicode common in dev/sysadmin notes render instead of showing tofu.
/// The egui defaults stay as fallback (emoji, Cyrillic, …).
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/DejaVuSans.ttf")),
    );
    fonts.font_data.insert(
        "dejavu-mono".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/DejaVuSansMono.ttf")),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "dejavu".to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "dejavu-mono".to_owned());
    ctx.set_fonts(fonts);
}

fn default_autosave_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "Trellis", "Trellis")
        .map(|d| d.data_dir().join("autosave.ron"))
        .unwrap_or_else(|| PathBuf::from("trellis-autosave.ron"))
}
