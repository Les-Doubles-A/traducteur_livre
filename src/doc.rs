//! Modèle de document commun à tous les formats.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Heading(u8),
    Para,
    Quote,
    Item,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub kind: Kind,
    /// Texte courant (traduit après le passage du traducteur).
    pub text: String,
    /// Texte original, conservé pour la sortie bilingue.
    pub src: String,
}

impl Block {
    pub fn new(kind: Kind, text: String) -> Self {
        Self { kind, src: text.clone(), text }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Chapter {
    pub blocks: Vec<Block>,
}

impl Chapter {
    pub fn title(&self) -> Option<&str> {
        self.blocks.iter().find(|b| matches!(b.kind, Kind::Heading(_))).map(|b| b.text.as_str())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Book {
    pub title: String,
    pub chapters: Vec<Chapter>,
}

impl Book {
    pub fn new(title: String, chapters: Vec<Chapter>) -> Self {
        let chapters: Vec<Chapter> = chapters.into_iter().filter(|c| !c.blocks.is_empty()).collect();
        Self { title, chapters }
    }

    /// Découpe une liste plate de blocs en chapitres : chaque titre du niveau
    /// le plus haut présent ouvre un nouveau chapitre.
    pub fn from_blocks(mut title: String, mut blocks: Vec<Block>) -> Self {
        let level = |b: &Block| if let Kind::Heading(l) = b.kind { Some(l) } else { None };
        let mut top = blocks.iter().filter_map(level).min();
        // Un titre unique de plus haut niveau en tête = titre du livre, pas un chapitre.
        if let (Some(t), Some(first)) = (top, blocks.first()) {
            if level(first) == Some(t) && blocks.iter().filter(|b| level(b) == Some(t)).count() == 1 && blocks.len() > 1 {
                title = blocks.remove(0).text;
                top = blocks.iter().filter_map(level).min();
            }
        }
        let mut chapters = vec![Chapter::default()];
        for b in blocks {
            let opens = matches!((b.kind, top), (Kind::Heading(l), Some(t)) if l == t);
            if opens && !chapters.last().unwrap().blocks.is_empty() {
                chapters.push(Chapter::default());
            }
            chapters.last_mut().unwrap().blocks.push(b);
        }
        Self::new(title, chapters)
    }

    pub fn blocks_mut(&mut self) -> impl Iterator<Item = &mut Block> {
        self.chapters.iter_mut().flat_map(|c| c.blocks.iter_mut())
    }

    pub fn all_text(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str())
            .chain(self.chapters.iter().flat_map(|c| c.blocks.iter().map(|b| b.text.as_str())))
    }
}

/// Réduit tous les blancs à une seule espace.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

pub fn has_letters(s: &str) -> bool {
    s.chars().any(char::is_alphabetic)
}
