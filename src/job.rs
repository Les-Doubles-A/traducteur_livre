//! Exécution d'une file de fichiers dans un fil d'arrière-plan.

use crate::output::{self, Format};
use crate::translate::{Config, Translator};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct Job {
    pub files: Vec<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub format: Format,
    pub bilingual: bool,
    pub pdf_font: Option<PathBuf>,
    pub cfg: Config,
}

pub enum Msg {
    Status(String),
    Progress { file: usize, files: usize, frac: f32, segs: (usize, usize) },
    Done { out: PathBuf, failed: usize },
    Error(String),
    Finished,
}

pub fn output_path(src: &Path, out_dir: Option<&Path>, lang: &str, fmt: Format) -> PathBuf {
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("livre");
    let dir = out_dir.map(Path::to_path_buf).or_else(|| src.parent().map(Path::to_path_buf)).unwrap_or_default();
    dir.join(format!("{stem}.{}.{}", crate::langs::file_code(lang), fmt.ext()))
}

pub fn run(job: Job, cancel: Arc<AtomicBool>, send: impl Fn(Msg)) {
    let translator = match Translator::new(job.cfg.clone()) {
        Ok(t) => t,
        Err(e) => {
            send(Msg::Error(e));
            send(Msg::Finished);
            return;
        }
    };
    let n = job.files.len();
    for (i, path) in job.files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        send(Msg::Status(format!("Lecture de {name}")));
        let mut book = match crate::input::read(path) {
            Ok(b) => b,
            Err(e) => {
                send(Msg::Error(format!("{name} : {e}")));
                continue;
            }
        };
        send(Msg::Status(format!("Traduction de {name}")));
        let res = translator.translate_book(&mut book, &cancel, |p| {
            let frac = if p.total_chars == 0 { 1.0 } else { p.done_chars as f32 / p.total_chars as f32 };
            send(Msg::Progress { file: i, files: n, frac, segs: (p.done_segs, p.total_segs) });
        });
        let stats = match res {
            Ok(s) => s,
            Err(e) => {
                send(Msg::Error(e));
                break;
            }
        };
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(e) = &stats.last_error {
            send(Msg::Error(format!("{name} : {} passage(s) non traduit(s), dernière erreur : {e}", stats.failed)));
        }
        send(Msg::Status(format!("Écriture du {}", job.format.ext().to_uppercase())));
        let out = output_path(path, job.out_dir.as_deref(), &job.cfg.tgt, job.format);
        let opts = output::Options { lang: &job.cfg.tgt, bilingual: job.bilingual, pdf_font: job.pdf_font.as_deref() };
        match output::write(&book, &out, job.format, &opts) {
            Ok(()) => send(Msg::Done { out, failed: stats.failed }),
            Err(e) => send(Msg::Error(format!("{name} : {e}"))),
        }
    }
    send(Msg::Finished);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Block, Book, Kind};
    use crate::translate::Engine;

    fn cfg(tgt: &str) -> Config {
        Config {
            engine: Engine::Test, url: String::new(), model: String::new(), api_key: String::new(), deepl_key: String::new(),
            src: "en".into(), tgt: tgt.into(), parallel: 3, batch_chars: 3000, instructions: format!("test {}", std::process::id()),
            glossary: Vec::new(), timeout_s: 30,
        }
    }

    #[test]
    fn roundtrip_all_formats() {
        let dir = std::env::temp_dir().join(format!("trad_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let blocks = vec![
            Block::new(Kind::Heading(1), "Chapter One".into()),
            Block::new(Kind::Para, "It was a dark & stormy night <really>.".into()),
            Block::new(Kind::Quote, "A quoted line.".into()),
            Block::new(Kind::Item, "First item".into()),
            Block::new(Kind::Heading(1), "Chapter Two".into()),
            Block::new(Kind::Para, "The end, 42.".into()),
        ];
        let src = Book::from_blocks("My Book".into(), blocks);
        assert_eq!(src.chapters.len(), 2);
        let tr = Translator::new(cfg("fr")).unwrap();
        let cancel = AtomicBool::new(false);
        for fmt in Format::ALL {
            let mut book = src.clone();
            let stats = tr.translate_book(&mut book, &cancel, |_| {}).unwrap();
            assert_eq!(stats.failed, 0);
            let out = dir.join(format!("livre.fr.{}", fmt.ext()));
            let o = output::Options { lang: "fr", bilingual: true, pdf_font: None };
            output::write(&book, &out, fmt, &o).unwrap();
            let back = crate::input::read(&out).unwrap();
            let text: String = back.all_text().collect::<Vec<_>>().join("\n");
            assert!(text.contains("[fr] It was a dark & stormy night <really>."), "{fmt:?}: {text}");
            assert!(text.contains("[fr] The end, 42."), "{fmt:?}");
            if matches!(fmt, Format::Epub | Format::Docx | Format::Html | Format::Md) {
                assert!(back.chapters.len() >= 2, "{fmt:?}: {} chapitres", back.chapters.len());
                let h = back.chapters.iter().flat_map(|c| &c.blocks).find(|b| b.text == "[fr] Chapter Two").expect("titre");
                assert!(matches!(h.kind, Kind::Heading(_)), "{fmt:?}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cyrillic_pdf_uses_system_font() {
        let dir = std::env::temp_dir().join(format!("trad_test_ru_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut book = Book::from_blocks("Книга".into(), vec![Block::new(Kind::Para, "Привет, мир.".into())]);
        let tr = Translator::new(cfg("ru")).unwrap();
        tr.translate_book(&mut book, &AtomicBool::new(false), |_| {}).unwrap();
        let out = dir.join("x.pdf");
        output::write(&book, &out, Format::Pdf, &output::Options { lang: "ru", bilingual: false, pdf_font: None }).unwrap();
        // pdf-extract ne relit pas toutes les polices : on vérifie seulement que le PDF est produit.
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"%PDF") && bytes.len() > 10_000);
        if let Ok(keep) = std::env::var("KEEP_PDF") {
            std::fs::copy(&out, keep).unwrap();
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
