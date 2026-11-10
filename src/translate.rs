//! Moteurs de traduction, découpage en lots, cache et exécution parallèle.

use crate::doc::{has_letters, Book};
use crate::langs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Engine {
    /// Toute API compatible OpenAI : LM Studio, Ollama, llama.cpp, OpenAI, OpenRouter...
    Llm,
    DeepL,
    /// Aucun appel réseau : préfixe le texte. Pour tester les formats.
    Test,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub engine: Engine,
    pub url: String,
    pub model: String,
    pub api_key: String,
    pub deepl_key: String,
    pub src: String,
    pub tgt: String,
    pub parallel: usize,
    pub batch_chars: usize,
    pub instructions: String,
    pub glossary: Vec<(String, String)>,
    pub timeout_s: u64,
}

pub struct Progress {
    pub done_chars: usize,
    pub total_chars: usize,
    pub done_segs: usize,
    pub total_segs: usize,
}

pub struct Stats {
    pub failed: usize,
    /// Dernière erreur passagère rencontrée (délai dépassé, réponse incomplète...).
    pub last_error: Option<String>,
}

/// Hachage FNV-1a 64 bits (stable d'une version de Rust à l'autre, contrairement à DefaultHasher).
pub fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

pub fn load_glossary(path: &str) -> Result<Vec<(String, String)>, String> {
    if path.trim().is_empty() {
        return Ok(Vec::new());
    }
    let s = std::fs::read_to_string(path).map_err(|e| format!("Glossaire : {e}"))?;
    let map: HashMap<String, String> = serde_json::from_str(&s).map_err(|e| format!("Glossaire JSON invalide : {e}"))?;
    let mut v: Vec<_> = map.into_iter().filter(|(k, _)| !k.trim().is_empty()).collect();
    v.sort();
    Ok(v)
}

// ---------------------------------------------------------------- cache

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("traducteur_livre")
}

struct Cache {
    path: PathBuf,
    map: HashMap<String, String>,
    last_save: Instant,
    dirty: bool,
}

impl Cache {
    fn open(cfg: &Config) -> Self {
        let model = match cfg.engine {
            Engine::Llm => cfg.model.as_str(),
            Engine::DeepL => "deepl",
            Engine::Test => "test",
        };
        let gl: String = cfg.glossary.iter().map(|(a, b)| format!("{a}={b};")).collect();
        let key = format!("{model}|{}|{}|{}|{gl}", cfg.src, cfg.tgt, cfg.instructions.trim());
        let path = cache_dir().join(format!("{:016x}.json", fnv(&key)));
        let map = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Self { path, map, last_save: Instant::now(), dirty: false }
    }

    fn insert(&mut self, k: String, v: String) {
        self.map.insert(k, v);
        self.dirty = true;
        if self.last_save.elapsed() > Duration::from_secs(5) {
            self.save();
        }
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let _ = std::fs::create_dir_all(self.path.parent().unwrap());
        let tmp = self.path.with_extension("tmp");
        if let Ok(s) = serde_json::to_string(&self.map) {
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
        self.dirty = false;
        self.last_save = Instant::now();
    }
}

// ---------------------------------------------------------------- moteurs

enum Fail {
    /// Erreur passagère : on réessaie.
    Retry(String),
    /// Erreur de configuration (clé, modèle, URL) : on arrête tout.
    Fatal(String),
}

pub struct Translator {
    cfg: Config,
    client: reqwest::blocking::Client,
    last_error: std::sync::Mutex<Option<String>>,
}

fn short(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 300 { s.chars().take(300).collect::<String>() + "..." } else { s.to_string() }
}

fn http_fail(status: reqwest::StatusCode, body: &str) -> Fail {
    let msg = format!("HTTP {status} : {}", short(body));
    if status.as_u16() == 429 || status.is_server_error() { Fail::Retry(msg) } else { Fail::Fatal(msg) }
}

fn llm_base(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    u.strip_suffix("/chat/completions").unwrap_or(u).to_string()
}

pub fn list_models(url: &str, key: &str) -> Result<Vec<String>, String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    let mut req = client.get(format!("{}/models", llm_base(url)));
    if !key.trim().is_empty() {
        req = req.bearer_auth(key.trim());
    }
    let resp = req.send().map_err(|e| format!("Serveur injoignable : {e}"))?;
    let status = resp.status();
    let body = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status} : {}", short(&body)));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let mut ids: Vec<String> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(String::from))
        .filter(|id| !id.to_lowercase().contains("embed"))
        .collect();
    ids.sort();
    Ok(ids)
}

/// Retire le raisonnement des modèles "thinking" (<think>...</think>).
fn strip_think(s: &str) -> &str {
    match s.rfind("</think>") {
        Some(i) => &s[i + "</think>".len()..],
        None => s,
    }
}

/// Découpe "[[1]] a [[2]] b" en {1: "a", 2: "b"}.
fn parse_markers(s: &str) -> HashMap<usize, String> {
    let mut out = HashMap::new();
    let mut cur: Option<(usize, usize)> = None; // (numéro, début du texte)
    let b = s.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'[' && b[i + 1] == b'[' {
            let digits = b[i + 2..].iter().take_while(|c| c.is_ascii_digit()).count();
            let end = i + 2 + digits;
            if digits > 0 && b.get(end) == Some(&b']') && b.get(end + 1) == Some(&b']') {
                if let Some((n, start)) = cur {
                    out.insert(n, s[start..i].trim().to_string());
                }
                let n = s[i + 2..end].parse().unwrap_or(0);
                cur = Some((n, end + 2));
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
    if let Some((n, start)) = cur {
        out.insert(n, s[start..].trim().to_string());
    }
    out
}

fn plausible(src: &str, tr: &str) -> bool {
    let (ls, lt) = (src.chars().count(), tr.chars().count());
    if lt == 0 {
        return false;
    }
    if ls < 20 {
        return true;
    }
    // Le modèle a renvoyé le texte tel quel
    if ls > 40 && src.trim() == tr.trim() {
        return false;
    }
    let r = lt as f64 / ls as f64;
    (0.15..=6.0).contains(&r)
}

impl Translator {
    pub fn new(cfg: Config) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_s.max(10)))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { cfg, client, last_error: Default::default() })
    }

    fn batch_limits(&self) -> (usize, usize) {
        match self.cfg.engine {
            Engine::Llm => (self.cfg.batch_chars.max(200), 40),
            Engine::DeepL => (25_000, 50),
            Engine::Test => (100_000, 500),
        }
    }

    fn system_prompt(&self, texts: &[&str]) -> String {
        let tgt = langs::prompt_name(&self.cfg.tgt);
        let from = if self.cfg.src == langs::AUTO { String::new() } else { format!("from {} ", langs::prompt_name(&self.cfg.src)) };
        let mut p = format!(
            "You are an expert literary translator. Translate the user's text {from}into {tgt}.\n\
Rules:\n\
- Translate faithfully and completely into natural, fluent {tgt}. Keep the author's tone, style, register and paragraphing. Never summarize, censor, explain or add anything.\n\
- The text is split into segments marked [[1]], [[2]], ... Reply with exactly the same markers, in the same order, one translated segment per marker. Never merge, split or skip segments.\n\
- Keep proper names and numbers. Headings stay short.\n\
- Output only the translation, with no notes or comments."
        );
        let ins = self.cfg.instructions.trim();
        if !ins.is_empty() {
            p.push_str(&format!("\n\nAbout this book / extra instructions:\n{ins}"));
        }
        let joined = texts.join("\n").to_lowercase();
        let terms: Vec<String> =
            self.cfg.glossary.iter().filter(|(k, _)| joined.contains(&k.to_lowercase())).map(|(k, v)| format!("- {k} -> {v}")).collect();
        if !terms.is_empty() {
            p.push_str("\n\nMandatory glossary:\n");
            p.push_str(&terms.join("\n"));
        }
        p
    }

    fn call_llm(&self, texts: &[&str]) -> Result<Vec<Option<String>>, Fail> {
        let mut user: String = texts.iter().enumerate().map(|(i, t)| format!("[[{}]] {t}\n\n", i + 1)).collect();
        if self.cfg.model.to_lowercase().contains("qwen3") {
            user.push_str("/no_think");
        }
        let body = json!({
            "model": self.cfg.model,
            "messages": [
                {"role": "system", "content": self.system_prompt(texts)},
                {"role": "user", "content": user},
            ],
            "temperature": 0.2,
            "stream": false,
        });
        let mut req = self.client.post(format!("{}/chat/completions", llm_base(&self.cfg.url))).json(&body);
        if !self.cfg.api_key.trim().is_empty() {
            req = req.bearer_auth(self.cfg.api_key.trim());
        }
        let resp = req.send().map_err(|e| {
            if e.is_connect() { Fail::Fatal(format!("Serveur injoignable ({}) : lance LM Studio / Ollama", self.cfg.url)) } else { Fail::Retry(e.to_string()) }
        })?;
        let status = resp.status();
        let text = resp.text().map_err(|e| Fail::Retry(e.to_string()))?;
        if !status.is_success() {
            return Err(http_fail(status, &text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| Fail::Retry(format!("Réponse illisible : {e}")))?;
        let content = v["choices"][0]["message"]["content"].as_str().ok_or_else(|| Fail::Retry(format!("Réponse inattendue : {}", short(&text))))?;
        let content = strip_think(content).trim();
        let mut found = parse_markers(content);
        if texts.len() == 1 && found.is_empty() {
            found.insert(1, content.to_string());
        }
        Ok((1..=texts.len()).map(|i| found.remove(&i)).collect())
    }

    fn call_deepl(&self, texts: &[&str]) -> Result<Vec<Option<String>>, Fail> {
        let key = self.cfg.deepl_key.trim();
        if key.is_empty() {
            return Err(Fail::Fatal("Clé DeepL manquante (Réglages)".into()));
        }
        let host = if key.ends_with(":fx") { "https://api-free.deepl.com" } else { "https://api.deepl.com" };
        let mut body = json!({"text": texts, "target_lang": langs::deepl_target(&self.cfg.tgt)});
        if let Some(s) = langs::deepl_source(&self.cfg.src) {
            body["source_lang"] = json!(s);
        }
        let resp = self
            .client
            .post(format!("{host}/v2/translate"))
            .header("Authorization", format!("DeepL-Auth-Key {key}"))
            .json(&body)
            .send()
            .map_err(|e| Fail::Retry(e.to_string()))?;
        let status = resp.status();
        let text = resp.text().map_err(|e| Fail::Retry(e.to_string()))?;
        if !status.is_success() {
            return Err(http_fail(status, &text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| Fail::Retry(e.to_string()))?;
        let arr = v["translations"].as_array().cloned().unwrap_or_default();
        Ok((0..texts.len()).map(|i| arr.get(i).and_then(|t| t["text"].as_str()).map(String::from)).collect())
    }

    fn call(&self, texts: &[&str]) -> Result<Vec<Option<String>>, Fail> {
        match self.cfg.engine {
            Engine::Llm => self.call_llm(texts),
            Engine::DeepL => self.call_deepl(texts),
            Engine::Test => Ok(texts.iter().map(|t| Some(format!("[{}] {t}", self.cfg.tgt))).collect()),
        }
    }

    /// Traduit un lot avec réessais, puis repli segment par segment.
    fn translate_batch(&self, texts: &[&str], cancel: &AtomicBool) -> Result<Vec<Option<String>>, String> {
        let mut best: Vec<Option<String>> = vec![None; texts.len()];
        for attempt in 1..=3u64 {
            if cancel.load(Ordering::Relaxed) {
                return Ok(best);
            }
            match self.call(texts) {
                Ok(res) => {
                    for (i, r) in res.into_iter().enumerate() {
                        if let Some(r) = r.filter(|r| plausible(texts[i], r)) {
                            best[i] = Some(r);
                        }
                    }
                    if best.iter().all(Option::is_some) {
                        return Ok(best);
                    }
                    *self.last_error.lock().unwrap() = Some("réponse incomplète ou non traduite du modèle".into());
                }
                Err(Fail::Fatal(e)) => return Err(e),
                Err(Fail::Retry(e)) => {
                    *self.last_error.lock().unwrap() = Some(e);
                    std::thread::sleep(Duration::from_secs(2 * attempt));
                }
            }
            if texts.len() > 1 && attempt == 2 {
                break; // le lot ne passe pas : on bascule sur le détail
            }
        }
        if texts.len() > 1 {
            for i in 0..texts.len() {
                if best[i].is_none() && !cancel.load(Ordering::Relaxed) {
                    best[i] = self.translate_batch(&texts[i..=i], cancel)?.pop().flatten();
                }
            }
        }
        Ok(best)
    }

    /// Traduit tout le livre en place.
    pub fn translate_book(&self, book: &mut Book, cancel: &AtomicBool, mut on_progress: impl FnMut(Progress)) -> Result<Stats, String> {
        let mut cache = Cache::open(&self.cfg);

        // Segments uniques, dans l'ordre du livre
        let mut seen = HashSet::new();
        let mut pending: Vec<String> = Vec::new();
        for t in book.all_text() {
            if has_letters(t) && seen.insert(t) && !cache.map.contains_key(t) {
                pending.push(t.to_string());
            }
        }
        *self.last_error.lock().unwrap() = None;

        // Lots
        let (max_chars, max_items) = self.batch_limits();
        let mut batches: Vec<Vec<&str>> = Vec::new();
        let mut size = 0;
        for t in &pending {
            let full = batches.last().is_none_or(|b| b.len() >= max_items || size + t.len() > max_chars);
            if full {
                batches.push(Vec::new());
                size = 0;
            }
            batches.last_mut().unwrap().push(t);
            size += t.len();
        }

        let total_chars: usize = pending.iter().map(|t| t.len()).sum();
        let total_segs = pending.len();
        let (mut done_chars, mut done_segs, mut failed) = (0, 0, 0);
        on_progress(Progress { done_chars, total_chars, done_segs, total_segs });

        let next = AtomicUsize::new(0);
        let mut fatal: Option<String> = None;
        let workers = self.cfg.parallel.clamp(1, 16).min(batches.len().max(1));
        std::thread::scope(|s| {
            let (tx, rx) = mpsc::channel();
            for _ in 0..workers {
                let tx = tx.clone();
                let (next, batches) = (&next, &batches);
                s.spawn(move || loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= batches.len() || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let r = self.translate_batch(&batches[i], cancel);
                    if tx.send((i, r)).is_err() {
                        break;
                    }
                });
            }
            drop(tx);
            for (i, r) in rx {
                match r {
                    Ok(res) => {
                        for (src, tr) in batches[i].iter().zip(res) {
                            done_chars += src.len();
                            done_segs += 1;
                            match tr {
                                Some(tr) => cache.insert(src.to_string(), tr),
                                None => failed += 1,
                            }
                        }
                        on_progress(Progress { done_chars, total_chars, done_segs, total_segs });
                    }
                    Err(e) => {
                        fatal.get_or_insert(e);
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
        });
        cache.save();
        if let Some(e) = fatal {
            return Err(e);
        }

        // Application : texte non traduit = laissé tel quel
        let map = &cache.map;
        if let Some(t) = map.get(&book.title) {
            book.title = t.clone();
        }
        for b in book.blocks_mut() {
            if let Some(t) = map.get(&b.text) {
                b.text = t.clone();
            }
        }
        let last_error = if failed > 0 { self.last_error.lock().unwrap().take() } else { None };
        Ok(Stats { failed, last_error })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers() {
        let m = parse_markers("[[1]] Bonjour.\n\n[[2]] Le [x] monde\n[[3]]fin");
        assert_eq!(m[&1], "Bonjour.");
        assert_eq!(m[&2], "Le [x] monde");
        assert_eq!(m[&3], "fin");
    }

    #[test]
    fn think_removed() {
        assert_eq!(strip_think("<think>hmm</think>\n[[1]] a").trim(), "[[1]] a");
    }

    #[test]
    fn base_url() {
        assert_eq!(llm_base("http://localhost:1234/v1/"), "http://localhost:1234/v1");
        assert_eq!(llm_base("http://x/v1/chat/completions"), "http://x/v1");
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::doc::{Block, Kind};

    /// cargo test live -- --ignored --nocapture   (nécessite LM Studio sur :1234)
    #[test]
    #[ignore]
    fn live_llm() {
        let url = "http://localhost:1234/v1";
        let model = list_models(url, "").unwrap().into_iter().find(|m| m.contains("instruct")).expect("modèle");
        let cfg = Config {
            engine: Engine::Llm, url: url.into(), model: model.clone(), api_key: String::new(), deepl_key: String::new(),
            src: crate::langs::AUTO.into(), tgt: "fr".into(), parallel: 2, batch_chars: 3000,
            instructions: format!("live {}", std::process::id()), glossary: vec![("Windmill".into(), "moulin à vent".into())], timeout_s: 300,
        };
        let mut book = Book::from_blocks("The Lost Garden".into(), vec![
            Block::new(Kind::Heading(1), "Chapter 1: The Windmill".into()),
            Block::new(Kind::Para, "The old windmill creaked in the wind, and Anna wondered whether anyone still lived there.".into()),
            Block::new(Kind::Para, "She pushed the door open. Inside, dust danced in the pale light.".into()),
            Block::new(Kind::Heading(1), "Kapitel 2: Die Rückkehr".into()),
            Block::new(Kind::Para, "Am nächsten Morgen kehrte sie mit ihrem Bruder zurück.".into()),
        ]);
        let t = Instant::now();
        let s = Translator::new(cfg).unwrap().translate_book(&mut book, &AtomicBool::new(false), |_| {}).unwrap();
        println!("{model} en {:.1}s, échecs : {}", t.elapsed().as_secs_f32(), s.failed);
        println!("{}", book.title);
        for b in book.blocks_mut() { println!("{:?} {}", b.kind, b.text); }
        assert_eq!(s.failed, 0);
    }
}
