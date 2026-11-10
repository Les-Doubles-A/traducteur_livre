//! Interface graphique (egui).

use crate::job::{self, Job, Msg};
use crate::langs::{self, AUTO, LANGS};
use crate::output::Format;
use crate::translate::{self, Config, Engine};
use eframe::egui::{self, Color32, RichText};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    engine: Engine,
    url: String,
    model: String,
    api_key: String,
    deepl_key: String,
    src: String,
    tgt: String,
    format: Format,
    bilingual: bool,
    out_dir: String,
    parallel: usize,
    batch_chars: usize,
    timeout_s: u64,
    instructions: String,
    glossary: String,
    pdf_font: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: Engine::Llm,
            url: "http://localhost:1234/v1".into(),
            model: String::new(),
            api_key: String::new(),
            deepl_key: String::new(),
            src: AUTO.into(),
            tgt: "fr".into(),
            format: Format::Epub,
            bilingual: false,
            out_dir: String::new(),
            parallel: 2,
            batch_chars: 3000,
            timeout_s: 600,
            instructions: String::new(),
            glossary: std::fs::canonicalize("glossaire.json").map(|p| p.display().to_string().trim_start_matches(r"\\?\").to_string()).unwrap_or_default(),
            pdf_font: String::new(),
        }
    }
}

struct Running {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    status: String,
    overall: f32,
    segs: (usize, usize),
    file: (usize, usize),
    start: Instant,
}

enum LogLine {
    Ok(PathBuf, usize),
    Err(String),
}

pub struct App {
    s: Settings,
    files: Vec<PathBuf>,
    run: Option<Running>,
    log: Vec<LogLine>,
    models: Vec<String>,
    models_rx: Option<Receiver<Result<Vec<String>, String>>>,
    models_err: Option<String>,
    show_settings: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let s: Settings = cc.storage.and_then(|st| eframe::get_value(st, eframe::APP_KEY)).unwrap_or_default();
        let show_settings = s.engine == Engine::Llm && s.model.is_empty();
        let mut app = Self { s, files: Vec::new(), run: None, log: Vec::new(), models: Vec::new(), models_rx: None, models_err: None, show_settings };
        if app.s.engine == Engine::Llm {
            app.refresh_models(&cc.egui_ctx);
        }
        app
    }

    fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            let ok = p.extension().and_then(|e| e.to_str()).is_some_and(|e| crate::input::EXTENSIONS.contains(&e.to_lowercase().as_str()));
            if ok && !self.files.contains(&p) {
                self.files.push(p);
            }
        }
    }

    fn refresh_models(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let (url, key, ctx) = (self.s.url.clone(), self.s.api_key.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(translate::list_models(&url, &key));
            ctx.request_repaint();
        });
        self.models_rx = Some(rx);
        self.models_err = None;
    }

    fn start(&mut self, ctx: &egui::Context) {
        let glossary = match translate::load_glossary(&self.s.glossary) {
            Ok(g) => g,
            Err(e) => {
                self.log.push(LogLine::Err(e));
                return;
            }
        };
        let cfg = Config {
            engine: self.s.engine,
            url: self.s.url.clone(),
            model: self.s.model.clone(),
            api_key: self.s.api_key.clone(),
            deepl_key: self.s.deepl_key.clone(),
            src: self.s.src.clone(),
            tgt: self.s.tgt.clone(),
            parallel: self.s.parallel,
            batch_chars: self.s.batch_chars,
            instructions: self.s.instructions.clone(),
            glossary,
            timeout_s: self.s.timeout_s,
        };
        let non_empty = |s: &str| (!s.trim().is_empty()).then(|| PathBuf::from(s.trim()));
        let j = Job {
            files: self.files.clone(),
            out_dir: non_empty(&self.s.out_dir),
            format: self.s.format,
            bilingual: self.s.bilingual,
            pdf_font: non_empty(&self.s.pdf_font),
            cfg,
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (c, ctx2) = (cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            job::run(j, c, |m| {
                let _ = tx.send(m);
                ctx2.request_repaint();
            })
        });
        self.log.clear();
        let n = self.files.len();
        self.run = Some(Running { rx, cancel, status: "Démarrage".into(), overall: 0.0, segs: (0, 0), file: (0, n), start: Instant::now() });
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.models_rx {
            if let Ok(r) = rx.try_recv() {
                match r {
                    Ok(m) => {
                        if self.s.model.is_empty() || !m.contains(&self.s.model) {
                            if let Some(best) = m.iter().max_by_key(|id| model_score(id)) {
                                self.s.model = best.clone();
                            }
                        }
                        self.models = m;
                    }
                    Err(e) => self.models_err = Some(e),
                }
                self.models_rx = None;
            }
        }
        let Some(run) = &mut self.run else { return };
        let mut finished = false;
        while let Ok(m) = run.rx.try_recv() {
            match m {
                Msg::Status(s) => run.status = s,
                Msg::Progress { file, files, frac, segs } => {
                    run.overall = (file as f32 + frac) / files.max(1) as f32;
                    run.segs = segs;
                    run.file = (file, files);
                }
                Msg::Done { out, failed } => self.log.push(LogLine::Ok(out, failed)),
                Msg::Error(e) => self.log.push(LogLine::Err(e)),
                Msg::Finished => finished = true,
            }
        }
        if finished {
            self.run = None;
        }
    }

    // ------------------------------------------------------------ vues

    fn lang_combo(ui: &mut egui::Ui, id: &str, code: &mut String, allow_auto: bool) {
        let custom = *code != AUTO && !LANGS.iter().any(|l| l.0 == code);
        egui::ComboBox::from_id_salt(id)
            .width(170.0)
            .selected_text(if custom { "Autre...".to_string() } else { langs::label(code) })
            .height(400.0)
            .show_ui(ui, |ui| {
                if allow_auto {
                    ui.selectable_value(code, AUTO.to_string(), langs::label(AUTO));
                }
                for (c, _, label) in LANGS {
                    ui.selectable_value(code, c.to_string(), *label);
                }
                if ui.selectable_label(custom, "Autre...").clicked() && !custom {
                    code.clear();
                }
            });
        if custom {
            ui.add(egui::TextEdit::singleline(code).hint_text("ex. Breton").desired_width(110.0));
        }
    }

    fn files_ui(&mut self, ui: &mut egui::Ui) {
        let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
        let stroke = if hovering { ui.visuals().selection.stroke } else { ui.visuals().widgets.noninteractive.bg_stroke };
        egui::Frame::group(ui.style()).stroke(stroke).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.files.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new("Glissez vos livres ici").size(17.0));
                    ui.label(RichText::new("EPUB, PDF, DOCX, ODT, FB2, HTML, MD, TXT").weak().small());
                    ui.add_space(6.0);
                    if ui.button("Choisir des fichiers...").clicked() {
                        self.pick_files();
                    }
                    ui.add_space(14.0);
                });
                return;
            }
            let mut remove = None;
            egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                for (i, f) in self.files.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let enabled = self.run.is_none();
                        if ui.add_enabled(enabled, egui::Button::new("x").small()).on_hover_text("Retirer").clicked() {
                            remove = Some(i);
                        }
                        ui.label(f.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()).on_hover_text(f.display().to_string());
                    });
                }
            });
            if let Some(i) = remove {
                self.files.remove(i);
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.run.is_none(), |ui| {
                    if ui.small_button("Ajouter...").clicked() {
                        self.pick_files();
                    }
                    if ui.small_button("Tout retirer").clicked() {
                        self.files.clear();
                    }
                });
            });
        });
    }

    fn pick_files(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Livres", crate::input::EXTENSIONS).pick_files() {
            self.add_files(p);
        }
    }

    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let idle = self.run.is_none();
        self.files_ui(ui);
        ui.add_space(10.0);

        ui.add_enabled_ui(idle, |ui| {
            egui::Grid::new("opts").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label("Langues");
                ui.horizontal(|ui| {
                    Self::lang_combo(ui, "src", &mut self.s.src, true);
                    let can_swap = self.s.src != AUTO;
                    if ui.add_enabled(can_swap, egui::Button::new("<->")).on_hover_text("Inverser").clicked() {
                        std::mem::swap(&mut self.s.src, &mut self.s.tgt);
                    }
                    Self::lang_combo(ui, "tgt", &mut self.s.tgt, false);
                });
                ui.end_row();

                ui.label("Format");
                ui.horizontal_wrapped(|ui| {
                    for f in Format::ALL {
                        ui.selectable_value(&mut self.s.format, f, f.ext().to_uppercase());
                    }
                    ui.separator();
                    ui.checkbox(&mut self.s.bilingual, "Bilingue").on_hover_text("Ajoute le texte original sous chaque paragraphe");
                });
                ui.end_row();

                ui.label("Dossier");
                ui.horizontal(|ui| {
                    let label = if self.s.out_dir.is_empty() { "À côté du livre".to_string() } else { self.s.out_dir.clone() };
                    ui.label(RichText::new(label).weak());
                    if ui.small_button("Changer...").clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.s.out_dir = d.display().to_string();
                        }
                    }
                    if !self.s.out_dir.is_empty() && ui.small_button("x").on_hover_text("À côté du livre").clicked() {
                        self.s.out_dir.clear();
                    }
                });
                ui.end_row();
            });
        });
        ui.add_space(12.0);

        let ready = !self.files.is_empty() && !self.s.tgt.trim().is_empty() && (self.s.engine != Engine::Llm || !self.s.model.trim().is_empty());
        let ctx = ui.ctx().clone();
        if let Some(run) = &self.run {
            let btn = egui::Button::new(RichText::new("Annuler").size(16.0)).min_size(egui::vec2(ui.available_width(), 36.0));
            if ui.add(btn).clicked() {
                run.cancel.store(true, Ordering::Relaxed);
            }
        } else {
            let btn = egui::Button::new(RichText::new("Traduire").size(16.0).strong()).min_size(egui::vec2(ui.available_width(), 36.0));
            let r = ui.add_enabled(ready, btn);
            let r = if self.s.engine == Engine::Llm && self.s.model.trim().is_empty() { r.on_disabled_hover_text("Choisissez un modèle dans les réglages") } else { r };
            if r.clicked() {
                self.start(&ctx);
            }
        }

        if let Some(run) = &self.run {
            ui.add_space(8.0);
            let el = run.start.elapsed().as_secs_f32();
            let eta = if run.overall > 0.02 { fmt_dur(el / run.overall - el) } else { "...".into() };
            ui.add(egui::ProgressBar::new(run.overall).show_percentage().animate(true));
            let files = if run.file.1 > 1 { format!("Livre {}/{}  -  ", run.file.0 + 1, run.file.1) } else { String::new() };
            let cancelled = if run.cancel.load(Ordering::Relaxed) { "  -  arrêt en cours" } else { "" };
            ui.label(RichText::new(format!("{files}{}  -  {}/{} segments  -  reste {eta}{cancelled}", run.status, run.segs.0, run.segs.1)).weak());
        }

        ui.add_space(8.0);
        for l in &self.log {
            match l {
                LogLine::Ok(p, failed) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Terminé").color(Color32::from_rgb(80, 170, 90)).strong());
                        ui.label(p.file_name().map(|s| s.to_string_lossy()).unwrap_or_default());
                        if ui.small_button("Ouvrir").clicked() {
                            open(p, false);
                        }
                        if ui.small_button("Dossier").clicked() {
                            open(p, true);
                        }
                    });
                    if *failed > 0 {
                        ui.label(RichText::new(format!("{failed} passage(s) laissé(s) en version originale. Relancez pour réessayer (le reste est en cache).")).color(Color32::from_rgb(210, 150, 40)));
                    }
                }
                LogLine::Err(e) => {
                    ui.label(RichText::new(e).color(Color32::from_rgb(220, 80, 70)));
                }
            }
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Réglages");
        ui.add_space(6.0);
        ui.add_enabled_ui(self.run.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.s.engine, Engine::Llm, "IA (OpenAI compatible)");
                ui.selectable_value(&mut self.s.engine, Engine::DeepL, "DeepL");
                ui.selectable_value(&mut self.s.engine, Engine::Test, "Test");
            });
            ui.add_space(6.0);
            match self.s.engine {
                Engine::Llm => {
                    ui.label("Adresse du serveur");
                    let r = ui.add(egui::TextEdit::singleline(&mut self.s.url).desired_width(f32::INFINITY));
                    ui.label(RichText::new("LM Studio :1234/v1  -  Ollama :11434/v1  -  ou https://api.openai.com/v1").weak().small());
                    ui.horizontal(|ui| {
                        if ui.small_button("LM Studio").clicked() {
                            self.s.url = "http://localhost:1234/v1".into();
                            self.refresh_models(&ctx);
                        }
                        if ui.small_button("Ollama").clicked() {
                            self.s.url = "http://localhost:11434/v1".into();
                            self.refresh_models(&ctx);
                        }
                    });
                    ui.add_space(4.0);
                    ui.label("Modèle");
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("model")
                            .width(ui.available_width() - 80.0)
                            .selected_text(if self.s.model.is_empty() { "-" } else { self.s.model.as_str() })
                            .show_ui(ui, |ui| {
                                for m in &self.models {
                                    ui.selectable_value(&mut self.s.model, m.clone(), m);
                                }
                            });
                        let busy = self.models_rx.is_some();
                        if ui.add_enabled(!busy, egui::Button::new("Actualiser")).on_hover_text("Recharger la liste des modèles").clicked() || r.lost_focus() {
                            self.refresh_models(&ctx);
                        }
                    });
                    ui.add(egui::TextEdit::singleline(&mut self.s.model).hint_text("ou nom du modèle").desired_width(f32::INFINITY));
                    if let Some(e) = &self.models_err {
                        ui.label(RichText::new(e).color(Color32::from_rgb(220, 80, 70)).small());
                    }
                    ui.label("Clé API (facultative en local)");
                    ui.add(egui::TextEdit::singleline(&mut self.s.api_key).password(true).desired_width(f32::INFINITY));
                }
                Engine::DeepL => {
                    ui.label("Clé API DeepL");
                    ui.add(egui::TextEdit::singleline(&mut self.s.deepl_key).password(true).desired_width(f32::INFINITY));
                    ui.label(RichText::new("Les clés gratuites finissent par :fx").weak().small());
                }
                Engine::Test => {
                    ui.label(RichText::new("Aucune traduction : préfixe le texte. Sert à vérifier les formats.").weak());
                }
            }

            ui.separator();
            if self.s.engine == Engine::Llm {
                ui.label("Consignes (facultatif)");
                ui.add(
                    egui::TextEdit::multiline(&mut self.s.instructions)
                        .hint_text("Genre, ton, public, tutoiement...")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                file_row(ui, "Glossaire JSON", &mut self.s.glossary, "json");
                ui.label(RichText::new("{\"terme source\": \"traduction imposée\"}").weak().small());
                ui.add_space(4.0);
                ui.add(egui::Slider::new(&mut self.s.parallel, 1..=8).text("requêtes en parallèle"));
                ui.add(egui::Slider::new(&mut self.s.batch_chars, 500..=8000).step_by(250.0).text("caractères par lot"));
                ui.add(egui::Slider::new(&mut self.s.timeout_s, 30..=1800).text("délai max (s)"));
            }
            file_row(ui, "Police PDF (.ttf)", &mut self.s.pdf_font, "ttf");
            ui.label(RichText::new("Vide = automatique (Crimson Text, ou police système pour les autres alphabets)").weak().small());
            ui.add_space(8.0);
            if ui.button("Vider le cache de traduction").clicked() {
                let _ = std::fs::remove_dir_all(translate::cache_dir());
            }
        });
    }
}

fn file_row(ui: &mut egui::Ui, label: &str, value: &mut String, ext: &str) {
    ui.label(label);
    ui.horizontal(|ui| {
        let name = Path::new(value.as_str()).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "aucun".into());
        ui.label(RichText::new(name).weak()).on_hover_text(value.as_str());
        if ui.small_button("Choisir...").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter(ext, &[ext]).pick_file() {
                *value = p.display().to_string();
            }
        }
        if !value.is_empty() && ui.small_button("x").clicked() {
            value.clear();
        }
    });
}

/// Choix automatique du modèle : privilégie les modèles de conversation généralistes.
fn model_score(id: &str) -> i32 {
    let l = id.to_lowercase();
    let mut s = 0;
    for (k, v) in [("instruct", 3), ("chat", 2), ("qwen", 2), ("mistral", 2), ("llama", 1), ("gemma", 2), ("gpt", 2)] {
        if l.contains(k) {
            s += v;
        }
    }
    for k in ["vl", "vision", "coder", "title", "guard", "whisper", "tts", "rerank", "270m", "135m", "0.5b"] {
        if l.contains(k) {
            s -= 4;
        }
    }
    s
}

fn fmt_dur(s: f32) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 { format!("{}h{:02}", s / 3600, s % 3600 / 60) } else if s >= 60 { format!("{} min", s.div_ceil(60)) } else { format!("{s} s") }
}

/// Ouvre un fichier avec l'application par défaut, ou son dossier.
fn open(p: &Path, folder: bool) {
    #[cfg(windows)]
    let _ = if folder {
        std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn()
    } else {
        std::process::Command::new("explorer").arg(p).spawn()
    };
    #[cfg(target_os = "macos")]
    let _ = if folder { std::process::Command::new("open").arg("-R").arg(p).spawn() } else { std::process::Command::new("open").arg(p).spawn() };
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(if folder { p.parent().unwrap_or(p) } else { p }).spawn();
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() && self.run.is_none() {
            self.add_files(dropped);
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Traducteur de livres");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.toggle_value(&mut self.show_settings, "Réglages");
                    let engine = match self.s.engine {
                        Engine::Llm => if self.s.model.is_empty() { "aucun modèle".to_string() } else { self.s.model.clone() },
                        Engine::DeepL => "DeepL".into(),
                        Engine::Test => "mode test".into(),
                    };
                    ui.label(RichText::new(engine).weak().small());
                });
            });
            ui.add_space(4.0);
        });
        if self.show_settings {
            egui::Panel::right("settings").resizable(false).exact_size(330.0).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_ui(ui));
            });
        }
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.main_ui(ui));
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.s);
    }
}
