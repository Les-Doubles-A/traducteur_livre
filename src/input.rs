//! Lecture des formats d'entrée : TXT, MD, HTML, EPUB, DOCX, ODT, FB2, PDF.

use crate::doc::{normalize, Block, Book, Chapter, Kind};
use std::io::Read;
use std::path::Path;

pub const EXTENSIONS: &[&str] = &["txt", "md", "markdown", "html", "htm", "xhtml", "epub", "docx", "odt", "fb2", "pdf"];

pub fn read(path: &Path) -> Result<Book, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Livre").to_string();
    let book = match ext.as_str() {
        "txt" => read_txt(&decode(&std::fs::read(path).map_err(io)?), stem),
        "md" | "markdown" => read_md(&decode(&std::fs::read(path).map_err(io)?), stem),
        "html" | "htm" | "xhtml" => {
            let s = decode(&std::fs::read(path).map_err(io)?);
            let (title, blocks) = html_blocks(&s);
            Book::from_blocks(title.unwrap_or(stem), blocks)
        }
        "epub" => read_epub(path, stem)?,
        "docx" => read_docx(path, stem)?,
        "odt" => read_odt(path, stem)?,
        "fb2" => read_fb2(&decode(&std::fs::read(path).map_err(io)?), stem)?,
        "pdf" => read_pdf(path, stem)?,
        _ => return Err(format!("Format non pris en charge : .{ext}")),
    };
    if book.chapters.is_empty() {
        return Err("Aucun texte trouvé dans le fichier (PDF scanné ?)".into());
    }
    Ok(book)
}

fn io(e: std::io::Error) -> String {
    e.to_string()
}

/// UTF-8 (avec ou sans BOM), UTF-16 avec BOM, sinon Windows-1252.
fn decode(bytes: &[u8]) -> String {
    if let Some((enc, _)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc.decode(bytes).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}

// ---------------------------------------------------------------- texte brut

const CHAPTER_WORDS: &[&str] = &[
    "chapter", "chapitre", "capítulo", "capitulo", "capitolo", "kapitel", "hoofdstuk", "rozdział",
    "глава", "part", "partie", "parte", "teil", "book", "livre", "libro", "prologue", "prologo",
    "prólogo", "epilogue", "épilogue", "epilogo", "epílogo", "introduction", "préface", "preface",
];

fn looks_like_heading(line: &str) -> bool {
    let l = line.trim();
    if l.is_empty() || l.chars().count() > 60 {
        return false;
    }
    let lower = l.to_lowercase();
    let first = lower.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
    if CHAPTER_WORDS.contains(&first) {
        return true;
    }
    // 第一章 / 第3章
    if l.starts_with('第') && (l.contains('章') || l.contains('回')) {
        return true;
    }
    // Chiffres romains seuls : "IV", "XII."
    let t = l.trim_end_matches('.');
    !t.is_empty() && t.len() <= 6 && t.chars().all(|c| "IVXLC".contains(c))
}

fn read_txt(s: &str, title: String) -> Book {
    let s = s.replace("\r\n", "\n");
    let lines: Vec<&str> = s.lines().collect();
    let blanks = lines.iter().filter(|l| l.trim().is_empty()).count();
    let mut blocks = Vec::new();
    // Paragraphes séparés par des lignes vides (texte coupé à la ligne) ou une ligne = un paragraphe.
    if blanks * 20 >= lines.len() {
        for para in s.split("\n\n") {
            push_txt_para(&mut blocks, para);
        }
    } else {
        for l in lines {
            push_txt_para(&mut blocks, l);
        }
    }
    Book::from_blocks(title, blocks)
}

fn push_txt_para(blocks: &mut Vec<Block>, para: &str) {
    let lines: Vec<&str> = para.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return;
    }
    if lines.len() == 1 && looks_like_heading(lines[0]) {
        blocks.push(Block::new(Kind::Heading(1), normalize(lines[0])));
    } else {
        blocks.push(Block::new(Kind::Para, join_lines(&lines)));
    }
}

/// Recolle des lignes en recollant les césures ("tra-" + "duction").
fn join_lines(lines: &[&str]) -> String {
    let mut out = String::new();
    for l in lines {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        if out.ends_with('-') && l.chars().next().is_some_and(char::is_lowercase) {
            out.pop();
        } else if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(l);
    }
    normalize(&out)
}

// ---------------------------------------------------------------- Markdown

fn read_md(s: &str, title: String) -> Book {
    let mut blocks = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut cur_kind = Kind::Para;
    let mut in_code = false;
    let flush = |cur: &mut Vec<String>, kind: Kind, blocks: &mut Vec<Block>| {
        if !cur.is_empty() {
            let refs: Vec<&str> = cur.iter().map(String::as_str).collect();
            let t = md_inline(&join_lines(&refs));
            if !t.is_empty() {
                blocks.push(Block::new(kind, t));
            }
            cur.clear();
        }
    };
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if t.is_empty() || t == "---" || t == "***" {
            flush(&mut cur, cur_kind, &mut blocks);
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut cur, cur_kind, &mut blocks);
            let h = md_inline(t[hashes..].trim().trim_end_matches('#').trim());
            if !h.is_empty() {
                blocks.push(Block::new(Kind::Heading(hashes as u8), h));
            }
            continue;
        }
        let item = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")).or_else(|| {
            let d = t.chars().take_while(char::is_ascii_digit).count();
            if d > 0 { t[d..].strip_prefix(". ") } else { None }
        });
        if let Some(rest) = item {
            flush(&mut cur, cur_kind, &mut blocks);
            cur_kind = Kind::Item;
            cur.push(rest.to_string());
            continue;
        }
        if let Some(rest) = t.strip_prefix('>') {
            if cur_kind != Kind::Quote {
                flush(&mut cur, cur_kind, &mut blocks);
            }
            cur_kind = Kind::Quote;
            cur.push(rest.trim().to_string());
            continue;
        }
        if cur_kind == Kind::Quote || (cur_kind == Kind::Item && cur.is_empty()) {
            flush(&mut cur, cur_kind, &mut blocks);
            cur_kind = Kind::Para;
        }
        if cur.is_empty() {
            cur_kind = Kind::Para;
        }
        cur.push(t.to_string());
    }
    flush(&mut cur, cur_kind, &mut blocks);
    Book::from_blocks(title, blocks)
}

/// Retire le balisage Markdown en ligne : images, liens, emphase, code.
fn md_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '!' if c.get(i + 1) == Some(&'[') => {
                // image : on saute ![alt](url)
                if let Some(end) = find_link_end(&c, i + 1) {
                    i = end;
                    continue;
                }
                out.push('!');
            }
            '[' => {
                if let Some(close) = c[i..].iter().position(|&x| x == ']').map(|p| p + i) {
                    if c.get(close + 1) == Some(&'(') {
                        if let Some(end) = c[close..].iter().position(|&x| x == ')').map(|p| p + close + 1) {
                            out.extend(&c[i + 1..close]);
                            i = end;
                            continue;
                        }
                    }
                }
                out.push('[');
            }
            '*' | '`' => {}
            '_' if c.get(i + 1) == Some(&'_') => i += 1,
            ch => out.push(ch),
        }
        i += 1;
    }
    normalize(&out)
}

fn find_link_end(c: &[char], open: usize) -> Option<usize> {
    let close = c[open..].iter().position(|&x| x == ']')? + open;
    if c.get(close + 1) != Some(&'(') {
        return None;
    }
    Some(c[close..].iter().position(|&x| x == ')')? + close + 1)
}

// ---------------------------------------------------------------- HTML

const SKIP_TAGS: &[&str] = &["script", "style", "head", "nav", "svg", "math", "noscript", "template", "img", "rt", "rp"];
const BLOCK_TAGS: &[&str] = &[
    "p", "div", "section", "article", "body", "html", "main", "header", "footer", "aside", "blockquote", "li", "ul",
    "ol", "dl", "dt", "dd", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "figure", "figcaption", "table", "tr", "td",
    "th", "caption", "hr", "center", "address", "tbody", "thead",
];

/// Extrait les blocs d'un document HTML/XHTML. Renvoie aussi le <title>.
pub fn html_blocks(src: &str) -> (Option<String>, Vec<Block>) {
    let html = scraper::Html::parse_document(src);
    let title = scraper::Selector::parse("title")
        .ok()
        .and_then(|sel| html.select(&sel).next())
        .map(|t| normalize(&t.text().collect::<String>()))
        .filter(|t| !t.is_empty());
    let mut blocks = Vec::new();
    let mut cur = String::new();
    walk_html(*html.root_element(), &mut blocks, &mut cur, Kind::Para);
    flush_html(&mut cur, &mut blocks, Kind::Para);
    (title, blocks)
}

fn flush_html(cur: &mut String, blocks: &mut Vec<Block>, kind: Kind) {
    let t = normalize(cur);
    cur.clear();
    if !t.is_empty() {
        blocks.push(Block::new(kind, t));
    }
}

fn walk_html(node: ego_tree::NodeRef<scraper::Node>, blocks: &mut Vec<Block>, cur: &mut String, ctx: Kind) {
    for child in node.children() {
        match child.value() {
            scraper::Node::Text(t) => cur.push_str(t),
            scraper::Node::Element(e) => {
                let name = e.name();
                if SKIP_TAGS.contains(&name) {
                    continue;
                }
                if name == "br" {
                    cur.push(' ');
                    continue;
                }
                if !BLOCK_TAGS.contains(&name) {
                    walk_html(child, blocks, cur, ctx);
                    continue;
                }
                flush_html(cur, blocks, ctx);
                let kind = match name.as_bytes() {
                    [b'h', d @ b'1'..=b'6'] => Kind::Heading(d - b'0'),
                    _ => match name {
                        "li" | "dd" | "dt" => Kind::Item,
                        "blockquote" => Kind::Quote,
                        _ if matches!(ctx, Kind::Quote | Kind::Item) => ctx,
                        _ => Kind::Para,
                    },
                };
                walk_html(child, blocks, cur, kind);
                flush_html(cur, blocks, kind);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- archives ZIP

fn zip_open(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let f = std::fs::File::open(path).map_err(io)?;
    zip::ZipArchive::new(f).map_err(|e| format!("Archive invalide : {e}"))
}

fn zip_read(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<String, String> {
    let mut entry = z.by_name(name).map_err(|_| format!("Fichier manquant dans l'archive : {name}"))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).map_err(io)?;
    Ok(decode(&buf))
}

fn xml_parse(s: &str) -> Result<roxmltree::Document<'_>, String> {
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(s, opts).map_err(|e| format!("XML invalide : {e}"))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Résout `href` relativement au dossier `base` dans l'archive.
fn zip_join(base: &str, href: &str) -> String {
    let href = percent_decode(href.split('#').next().unwrap_or(href));
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for p in href.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

// ---------------------------------------------------------------- EPUB

fn read_epub(path: &Path, stem: String) -> Result<Book, String> {
    let mut z = zip_open(path)?;
    let container = zip_read(&mut z, "META-INF/container.xml")?;
    let opf_path = {
        let d = xml_parse(&container)?;
        d.descendants()
            .find(|n| n.has_tag_name("rootfile"))
            .and_then(|n| n.attribute("full-path"))
            .ok_or("container.xml sans rootfile")?
            .to_string()
    };
    let opf = zip_read(&mut z, &opf_path)?;
    let base = opf_path.rsplit_once('/').map(|(b, _)| b).unwrap_or("").to_string();
    let (title, docs) = {
        let d = xml_parse(&opf)?;
        let title = d
            .descendants()
            .find(|n| n.tag_name().name() == "title")
            .and_then(|n| n.text())
            .map(normalize)
            .filter(|t| !t.is_empty());
        let manifest: std::collections::HashMap<&str, &str> = d
            .descendants()
            .filter(|n| n.tag_name().name() == "item")
            .filter_map(|n| Some((n.attribute("id")?, n.attribute("href")?)))
            .collect();
        let docs: Vec<String> = d
            .descendants()
            .filter(|n| n.tag_name().name() == "itemref")
            .filter_map(|n| manifest.get(n.attribute("idref")?))
            .map(|href| zip_join(&base, href))
            .collect();
        (title, docs)
    };
    let mut chapters = Vec::new();
    for name in docs {
        let Ok(xhtml) = zip_read(&mut z, &name) else { continue };
        let (_, blocks) = html_blocks(&xhtml);
        chapters.push(Chapter { blocks });
    }
    let title = title.unwrap_or(stem);
    // Page de titre seule (un unique titre égal au titre du livre) : inutile à traduire deux fois.
    chapters.retain(|c| !(c.blocks.len() == 1 && c.blocks[0].text == title));
    Ok(Book::new(title, chapters))
}

// ---------------------------------------------------------------- DOCX

fn read_docx(path: &Path, stem: String) -> Result<Book, String> {
    let mut z = zip_open(path)?;
    let xml = zip_read(&mut z, "word/document.xml")?;
    let d = xml_parse(&xml)?;
    let mut blocks = Vec::new();
    for p in d.descendants().filter(|n| n.tag_name().name() == "p") {
        if p.ancestors().skip(1).any(|a| a.tag_name().name() == "p") {
            continue;
        }
        let mut text = String::new();
        for n in p.descendants() {
            match n.tag_name().name() {
                "t" => text.push_str(n.text().unwrap_or("")),
                "tab" | "br" | "cr" => text.push(' '),
                _ => {}
            }
        }
        let text = normalize(&text);
        if text.is_empty() {
            continue;
        }
        let ppr = p.children().find(|n| n.tag_name().name() == "pPr");
        let style = ppr
            .and_then(|pp| pp.children().find(|n| n.tag_name().name() == "pStyle"))
            .and_then(|s| s.attributes().find(|a| a.name() == "val").map(|a| a.value().to_lowercase()))
            .unwrap_or_default();
        let outline = ppr
            .and_then(|pp| pp.children().find(|n| n.tag_name().name() == "outlineLvl"))
            .and_then(|s| s.attributes().find(|a| a.name() == "val")?.value().parse::<u8>().ok());
        let kind = if style == "title" || style == "titre" {
            Kind::Heading(1)
        } else if ["heading", "titre", "berschrift", "título", "titolo", "kop"].iter().any(|k| style.contains(k)) {
            let lvl = style.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(1u8);
            Kind::Heading(lvl.clamp(1, 6))
        } else if let Some(o) = outline.filter(|&o| o < 6) {
            Kind::Heading(o + 1)
        } else if style.contains("list") || style.contains("liste") {
            Kind::Item
        } else if style.contains("quote") || style.contains("citation") {
            Kind::Quote
        } else {
            Kind::Para
        };
        blocks.push(Block::new(kind, text));
    }
    Ok(Book::from_blocks(stem, blocks))
}

// ---------------------------------------------------------------- ODT

fn read_odt(path: &Path, stem: String) -> Result<Book, String> {
    let mut z = zip_open(path)?;
    let xml = zip_read(&mut z, "content.xml")?;
    let d = xml_parse(&xml)?;
    let mut blocks = Vec::new();
    for n in d.descendants().filter(|n| matches!(n.tag_name().name(), "p" | "h")) {
        if n.ancestors().skip(1).any(|a| matches!(a.tag_name().name(), "p" | "h" | "note")) {
            continue;
        }
        let mut text = String::new();
        for c in n.descendants() {
            if c.is_text() {
                if !c.ancestors().any(|a| a.tag_name().name() == "note") {
                    text.push_str(c.text().unwrap_or(""));
                }
            } else if matches!(c.tag_name().name(), "s" | "tab" | "line-break") {
                text.push(' ');
            }
        }
        let text = normalize(&text);
        if text.is_empty() {
            continue;
        }
        let kind = if n.tag_name().name() == "h" {
            let lvl = n.attributes().find(|a| a.name() == "outline-level").and_then(|a| a.value().parse().ok()).unwrap_or(1u8);
            Kind::Heading(lvl.clamp(1, 6))
        } else if n.ancestors().any(|a| a.tag_name().name() == "list-item") {
            Kind::Item
        } else {
            Kind::Para
        };
        blocks.push(Block::new(kind, text));
    }
    Ok(Book::from_blocks(stem, blocks))
}

// ---------------------------------------------------------------- FB2

fn read_fb2(s: &str, stem: String) -> Result<Book, String> {
    let d = xml_parse(s)?;
    let title = d
        .descendants()
        .find(|n| n.tag_name().name() == "book-title")
        .and_then(|n| n.text())
        .map(normalize)
        .filter(|t| !t.is_empty());
    let mut blocks = Vec::new();
    let body = d
        .descendants()
        .find(|n| n.tag_name().name() == "body" && n.attribute("name").is_none())
        .ok_or("FB2 sans <body>")?;
    for n in body.descendants().filter(|n| n.is_element()) {
        let name = n.tag_name().name();
        let in_title = n.ancestors().skip(1).any(|a| a.tag_name().name() == "title");
        let text = || normalize(&n.descendants().filter(|c| c.is_text()).filter_map(|c| c.text()).collect::<String>());
        match name {
            "title" => {
                let depth = n.ancestors().filter(|a| a.tag_name().name() == "section").count().clamp(1, 6) as u8;
                let parts: Vec<String> = n
                    .children()
                    .filter(|c| c.tag_name().name() == "p")
                    .map(|c| normalize(&c.descendants().filter_map(|t| t.text()).collect::<String>()))
                    .filter(|t| !t.is_empty())
                    .collect();
                if !parts.is_empty() {
                    blocks.push(Block::new(Kind::Heading(depth), parts.join(" - ")));
                }
            }
            "p" | "v" | "subtitle" | "text-author" if !in_title => {
                let t = text();
                if t.is_empty() {
                    continue;
                }
                let quoted = n.ancestors().any(|a| matches!(a.tag_name().name(), "cite" | "epigraph" | "poem"));
                let kind = match name {
                    "subtitle" => Kind::Heading(6),
                    _ if quoted => Kind::Quote,
                    _ => Kind::Para,
                };
                blocks.push(Block::new(kind, t));
            }
            _ => {}
        }
    }
    Ok(Book::from_blocks(title.unwrap_or(stem), blocks))
}

// ---------------------------------------------------------------- PDF

fn read_pdf(path: &Path, stem: String) -> Result<Book, String> {
    let path = path.to_path_buf();
    // pdf-extract peut paniquer sur des PDF exotiques : on isole l'appel.
    let pages = std::panic::catch_unwind(move || pdf_extract::extract_text_by_pages(&path))
        .map_err(|_| "PDF illisible (structure non prise en charge)".to_string())?
        .map_err(|e| format!("PDF illisible : {e}"))?;

    let mut lines: Vec<String> = Vec::new();
    for page in &pages {
        for l in page.lines() {
            let t = l.trim();
            // Numéros de page seuls : ignorés
            if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ' ') {
                continue;
            }
            lines.push(t.to_string());
        }
    }
    let mut lens: Vec<usize> = lines.iter().map(|l| l.chars().count()).filter(|&n| n > 20).collect();
    lens.sort_unstable();
    let typical = lens.get(lens.len() * 3 / 4).copied().unwrap_or(70);

    let mut blocks = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let flush = |cur: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if cur.is_empty() {
            return;
        }
        if cur.len() == 1 && looks_like_heading(cur[0]) {
            blocks.push(Block::new(Kind::Heading(1), normalize(cur[0])));
        } else {
            let t = join_lines(cur);
            if !t.is_empty() {
                blocks.push(Block::new(Kind::Para, t));
            }
        }
        cur.clear();
    };
    for l in &lines {
        if l.is_empty() {
            flush(&mut cur, &mut blocks);
            continue;
        }
        if looks_like_heading(l) {
            flush(&mut cur, &mut blocks);
            cur.push(l);
            flush(&mut cur, &mut blocks);
            continue;
        }
        cur.push(l);
        // Ligne courte terminée par une ponctuation forte = fin de paragraphe
        let ends = l.ends_with(['.', '!', '?', '"', '»', '”', ':', '…']);
        if ends && l.chars().count() < typical * 7 / 10 {
            flush(&mut cur, &mut blocks);
        }
    }
    flush(&mut cur, &mut blocks);
    Ok(Book::from_blocks(stem, blocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md_inline_strips_markup() {
        assert_eq!(md_inline("**Gras** et [lien](http://x) ![img](a.png) `code`"), "Gras et lien code");
    }

    #[test]
    fn join_dehyphenates() {
        assert_eq!(join_lines(&["une tra-", "duction"]), "une traduction");
    }

    #[test]
    fn zip_paths() {
        assert_eq!(zip_join("OEBPS/text", "../Text/ch%201.xhtml#a"), "OEBPS/Text/ch 1.xhtml");
    }

    #[test]
    fn html_structure() {
        let (t, b) = html_blocks("<html><head><title>T</title></head><body><h1>Un</h1><p>A <i>b</i></p><blockquote><p>Q</p></blockquote><ul><li>x</li></ul></body></html>");
        assert_eq!(t.as_deref(), Some("T"));
        let k: Vec<_> = b.iter().map(|b| (b.kind, b.text.as_str())).collect();
        assert_eq!(k, vec![(Kind::Heading(1), "Un"), (Kind::Para, "A b"), (Kind::Quote, "Q"), (Kind::Item, "x")]);
    }
}
