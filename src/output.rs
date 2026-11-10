//! Écriture des formats de sortie : TXT, MD, HTML, EPUB, DOCX, PDF.

use crate::doc::{Block, Book, Kind};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Txt,
    Md,
    Html,
    Epub,
    Docx,
    Pdf,
}

impl Format {
    pub const ALL: [Format; 6] = [Format::Epub, Format::Pdf, Format::Docx, Format::Html, Format::Md, Format::Txt];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Txt => "txt",
            Format::Md => "md",
            Format::Html => "html",
            Format::Epub => "epub",
            Format::Docx => "docx",
            Format::Pdf => "pdf",
        }
    }
}

pub struct Options<'a> {
    pub lang: &'a str,
    pub bilingual: bool,
    pub pdf_font: Option<&'a Path>,
}

pub fn write(book: &Book, path: &Path, fmt: Format, o: &Options) -> Result<(), String> {
    let tmp = path.with_extension(format!("{}.part", fmt.ext()));
    match fmt {
        Format::Txt => std::fs::write(&tmp, to_txt(book, o)).map_err(|e| e.to_string())?,
        Format::Md => std::fs::write(&tmp, to_md(book, o)).map_err(|e| e.to_string())?,
        Format::Html => std::fs::write(&tmp, to_html(book, o)).map_err(|e| e.to_string())?,
        Format::Epub => to_epub(book, &tmp, o)?,
        Format::Docx => to_docx(book, &tmp, o)?,
        Format::Pdf => to_pdf(book, &tmp, o)?,
    }
    // Écriture atomique : jamais de fichier à moitié écrit.
    std::fs::rename(&tmp, path).map_err(|e| format!("Impossible d'écrire {} : {e}", path.display()))
}

/// Texte original à afficher en mode bilingue (seulement s'il diffère).
fn src_of<'a>(b: &'a Block, o: &Options) -> Option<&'a str> {
    (o.bilingual && b.src != b.text).then_some(b.src.as_str())
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // caractères interdits en XML 1.0
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------- TXT / MD

fn to_txt(book: &Book, o: &Options) -> String {
    let mut s = format!("{}\n\n\n", book.title);
    for ch in &book.chapters {
        for b in &ch.blocks {
            let prefix = match b.kind {
                Kind::Item => "- ",
                Kind::Quote => "    ",
                _ => "",
            };
            if matches!(b.kind, Kind::Heading(_)) {
                s.push('\n');
            }
            let _ = writeln!(s, "{prefix}{}", b.text);
            if let Some(src) = src_of(b, o) {
                let _ = writeln!(s, "{prefix}[{src}]");
            }
            s.push('\n');
        }
        s.push_str("\n\n");
    }
    s
}

fn to_md(book: &Book, o: &Options) -> String {
    let mut s = format!("# {}\n\n", book.title);
    for ch in &book.chapters {
        for b in &ch.blocks {
            let prefix = match b.kind {
                Kind::Heading(l) => "#".repeat((l as usize + 1).min(6)) + " ",
                Kind::Item => "- ".into(),
                Kind::Quote => "> ".into(),
                Kind::Para => String::new(),
            };
            let _ = writeln!(s, "{prefix}{}\n", b.text);
            if let Some(src) = src_of(b, o) {
                let _ = writeln!(s, "> *{src}*\n");
            }
        }
    }
    s
}

// ---------------------------------------------------------------- HTML / XHTML

const CSS: &str = "body{font-family:Georgia,'Times New Roman',serif;line-height:1.6;max-width:40em;margin:2em auto;padding:0 1em;}\
h1,h2,h3{line-height:1.25;margin:1.6em 0 .6em;}\
p{margin:0 0 .8em;text-align:justify;}\
blockquote{margin:1em 2em;font-style:italic;}\
.src{color:#777;font-size:.9em;font-style:italic;}\
.chapter{page-break-before:always;break-before:page;}\
@media (prefers-color-scheme:dark){body{background:#1b1b1b;color:#ddd;}.src{color:#999;}}";

/// `shift` décale les niveaux de titre (HTML : le h1 est réservé au titre du livre).
fn blocks_html(blocks: &[Block], o: &Options, shift: u8, out: &mut String) {
    let mut in_list = false;
    for b in blocks {
        let is_item = b.kind == Kind::Item;
        if is_item && !in_list {
            out.push_str("<ul>\n");
        } else if !is_item && in_list {
            out.push_str("</ul>\n");
        }
        in_list = is_item;
        let t = esc(&b.text);
        let _ = match b.kind {
            Kind::Heading(l) => {
                let l = (l + shift).min(6);
                writeln!(out, "<h{l}>{t}</h{l}>")
            }
            Kind::Para => writeln!(out, "<p>{t}</p>"),
            Kind::Quote => writeln!(out, "<blockquote><p>{t}</p></blockquote>"),
            Kind::Item => writeln!(out, "<li>{t}</li>"),
        };
        if let Some(src) = src_of(b, o) {
            let tag = if is_item { "li" } else { "p" };
            let _ = writeln!(out, "<{tag} class=\"src\">{}</{tag}>", esc(src));
        }
    }
    if in_list {
        out.push_str("</ul>\n");
    }
}

fn dir_attr(o: &Options) -> &'static str {
    if crate::langs::is_rtl(o.lang) { " dir=\"rtl\"" } else { "" }
}

fn to_html(book: &Book, o: &Options) -> String {
    let lang = crate::langs::file_code(o.lang);
    let mut s = format!(
        "<!DOCTYPE html>\n<html lang=\"{lang}\"{}>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>{}</title>\n<style>{CSS}</style>\n</head>\n<body>\n<h1>{}</h1>\n",
        dir_attr(o),
        esc(&book.title),
        esc(&book.title)
    );
    for ch in &book.chapters {
        s.push_str("<section class=\"chapter\">\n");
        blocks_html(&ch.blocks, o, 1, &mut s);
        s.push_str("</section>\n");
    }
    s.push_str("</body>\n</html>\n");
    s
}

// ---------------------------------------------------------------- ZIP

struct Zip(zip::ZipWriter<std::fs::File>);

impl Zip {
    fn create(path: &Path) -> Result<Self, String> {
        let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        Ok(Self(zip::ZipWriter::new(f)))
    }
    fn add(&mut self, name: &str, data: &[u8], stored: bool) -> Result<(), String> {
        use zip::write::SimpleFileOptions;
        let method = if stored { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
        self.0.start_file(name, SimpleFileOptions::default().compression_method(method)).map_err(|e| e.to_string())?;
        self.0.write_all(data).map_err(|e| e.to_string())
    }
    fn finish(self) -> Result<(), String> {
        self.0.finish().map(|_| ()).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------- EPUB 3

fn utc_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    // Algorithme "civil from days" (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn pseudo_uuid(seed: &str) -> String {
    let h1 = crate::translate::fnv(seed);
    let h2 = crate::translate::fnv(&format!("{seed}{}", utc_now()));
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        h1 >> 32,
        (h1 >> 16) & 0xffff,
        h1 & 0xfff,
        h2 >> 52,
        h2 & 0xffff_ffff_ffff
    )
}

fn to_epub(book: &Book, path: &Path, o: &Options) -> Result<(), String> {
    let lang = crate::langs::file_code(o.lang);
    let title = esc(&book.title);
    let dir = dir_attr(o);
    let mut z = Zip::create(path)?;
    z.add("mimetype", b"application/epub+zip", true)?;
    z.add(
        "META-INF/container.xml",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
        false,
    )?;
    z.add("OEBPS/style.css", CSS.as_bytes(), false)?;

    let page = |head: &str, body: &str| {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{lang}\" lang=\"{lang}\"{dir}>\n<head><meta charset=\"utf-8\"/><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"/></head>\n<body>\n{body}</body>\n</html>\n",
            esc(head)
        )
    };

    // Page de titre + chapitres
    z.add("OEBPS/title.xhtml", page(&book.title, &format!("<h1 style=\"margin-top:30%;text-align:center\">{title}</h1>\n")).as_bytes(), false)?;
    let mut manifest = String::from("<item id=\"title\" href=\"title.xhtml\" media-type=\"application/xhtml+xml\"/>\n");
    let mut spine = String::from("<itemref idref=\"title\"/>\n");
    let mut nav = String::new();
    let mut ncx = String::new();
    for (i, ch) in book.chapters.iter().enumerate() {
        let id = format!("c{:04}", i + 1);
        let name = ch.title().map(str::to_string).unwrap_or_else(|| format!("{}", i + 1));
        let mut body = String::new();
        blocks_html(&ch.blocks, o, 0, &mut body);
        z.add(&format!("OEBPS/{id}.xhtml"), page(&name, &body).as_bytes(), false)?;
        let _ = writeln!(manifest, "<item id=\"{id}\" href=\"{id}.xhtml\" media-type=\"application/xhtml+xml\"/>");
        let _ = writeln!(spine, "<itemref idref=\"{id}\"/>");
        let _ = writeln!(nav, "<li><a href=\"{id}.xhtml\">{}</a></li>", esc(&name));
        let _ = writeln!(
            ncx,
            "<navPoint id=\"n{id}\" playOrder=\"{}\"><navLabel><text>{}</text></navLabel><content src=\"{id}.xhtml\"/></navPoint>",
            i + 1,
            esc(&name)
        );
    }
    z.add(
        "OEBPS/nav.xhtml",
        page("Table des matières", &format!("<nav epub:type=\"toc\" id=\"toc\"><h1>{title}</h1>\n<ol>\n{nav}</ol></nav>\n")).as_bytes(),
        false,
    )?;
    let uid = pseudo_uuid(&book.title);
    z.add(
        "OEBPS/toc.ncx",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n<head><meta name=\"dtb:uid\" content=\"urn:uuid:{uid}\"/></head>\n<docTitle><text>{title}</text></docTitle>\n<navMap>\n{ncx}</navMap>\n</ncx>\n"
        )
        .as_bytes(),
        false,
    )?;
    z.add(
        "OEBPS/content.opf",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\" xml:lang=\"{lang}\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"uid\">urn:uuid:{uid}</dc:identifier>\n<dc:title>{title}</dc:title>\n<dc:language>{lang}</dc:language>\n<meta property=\"dcterms:modified\">{}</meta>\n</metadata>\n<manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n{manifest}</manifest>\n<spine toc=\"ncx\"{}>\n{spine}</spine>\n</package>\n",
            utc_now(),
            if crate::langs::is_rtl(o.lang) { " page-progression-direction=\"rtl\"" } else { "" }
        )
        .as_bytes(),
        false,
    )?;
    z.finish()
}

// ---------------------------------------------------------------- DOCX

fn to_docx(book: &Book, path: &Path, o: &Options) -> Result<(), String> {
    let rtl = crate::langs::is_rtl(o.lang);
    let lang = crate::langs::file_code(o.lang);
    let para = |out: &mut String, style: &str, text: &str, page_break: bool| {
        let _ = write!(out, "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/>");
        if page_break {
            out.push_str("<w:pageBreakBefore/>");
        }
        if rtl {
            out.push_str("<w:bidi/>");
        }
        out.push_str("</w:pPr><w:r>");
        if rtl {
            out.push_str("<w:rPr><w:rtl/></w:rPr>");
        }
        let _ = write!(out, "<w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>", esc(text));
    };
    let mut body = String::new();
    para(&mut body, "Title", &book.title, false);
    for ch in &book.chapters {
        for (i, b) in ch.blocks.iter().enumerate() {
            let style = match b.kind {
                Kind::Heading(l) => format!("Heading{}", l.min(3)),
                Kind::Quote => "Quote".into(),
                Kind::Item => "ListItem".into(),
                Kind::Para => "Normal".into(),
            };
            let text = if b.kind == Kind::Item { format!("\u{2022} {}", b.text) } else { b.text.clone() };
            para(&mut body, &style, &text, i == 0);
            if let Some(src) = src_of(b, o) {
                para(&mut body, "Source", src, false);
            }
        }
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1417\" w:right=\"1417\" w:bottom=\"1417\" w:left=\"1417\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"
    );
    let style = |id: &str, name: &str, ppr: &str, rpr: &str| {
        format!("<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr>{ppr}</w:pPr><w:rPr>{rpr}</w:rPr></w:style>")
    };
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Georgia\" w:hAnsi=\"Georgia\" w:cs=\"Arial\" w:eastAsia=\"SimSun\"/><w:sz w:val=\"24\"/><w:lang w:val=\"{lang}\" w:bidi=\"{lang}\" w:eastAsia=\"{lang}\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"300\" w:lineRule=\"auto\"/><w:jc w:val=\"both\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>{}{}{}{}{}{}{}</w:styles>",
        style("Title", "Title", "<w:jc w:val=\"center\"/><w:spacing w:before=\"2400\" w:after=\"600\"/>", "<w:b/><w:sz w:val=\"56\"/>"),
        style("Heading1", "heading 1", "<w:keepNext/><w:spacing w:before=\"480\" w:after=\"240\"/><w:jc w:val=\"left\"/><w:outlineLvl w:val=\"0\"/>", "<w:b/><w:sz w:val=\"36\"/>"),
        style("Heading2", "heading 2", "<w:keepNext/><w:spacing w:before=\"360\" w:after=\"160\"/><w:jc w:val=\"left\"/><w:outlineLvl w:val=\"1\"/>", "<w:b/><w:sz w:val=\"30\"/>"),
        style("Heading3", "heading 3", "<w:keepNext/><w:spacing w:before=\"240\" w:after=\"120\"/><w:jc w:val=\"left\"/><w:outlineLvl w:val=\"2\"/>", "<w:b/><w:sz w:val=\"26\"/>"),
        style("Quote", "Quote", "<w:ind w:left=\"720\" w:right=\"720\"/>", "<w:i/>"),
        style("ListItem", "List Item", "<w:ind w:left=\"360\"/><w:spacing w:after=\"60\"/>", ""),
        style("Source", "Original", "", "<w:i/><w:color w:val=\"808080\"/><w:sz w:val=\"21\"/>"),
    );
    let mut z = Zip::create(path)?;
    z.add(
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#,
        false,
    )?;
    z.add(
        "_rels/.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#,
        false,
    )?;
    z.add(
        "word/_rels/document.xml.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        false,
    )?;
    z.add(
        "docProps/core.xml",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:language>{lang}</dc:language></cp:coreProperties>",
            esc(&book.title)
        )
        .as_bytes(),
        false,
    )?;
    z.add("word/document.xml", document.as_bytes(), false)?;
    z.add("word/styles.xml", styles.as_bytes(), false)?;
    z.finish()
}

// ---------------------------------------------------------------- PDF

const CRIMSON_R: &[u8] = include_bytes!("../polices/CrimsonText-Regular.ttf");
const CRIMSON_B: &[u8] = include_bytes!("../polices/CrimsonText-Bold.ttf");
const CRIMSON_I: &[u8] = include_bytes!("../polices/CrimsonText-Italic.ttf");

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Caractères hors de ce que couvre Crimson Text (latin étendu + ponctuation).
fn needs_wide_font(c: char) -> bool {
    let u = c as u32;
    u > 0x024F && !(0x2000..=0x206F).contains(&u) && !(0x20A0..=0x20CF).contains(&u) && !(0x2100..=0x214F).contains(&u)
}

fn system_font(cjk: bool) -> Option<Vec<u8>> {
    let win = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let candidates: Vec<String> = if cjk {
        ["simhei.ttf", "Deng.ttf", "msyh.ttf", "malgun.ttf", "meiryo.ttf"]
            .iter()
            .map(|f| format!("{win}\\Fonts\\{f}"))
            .chain(["/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf".into(), "/Library/Fonts/Arial Unicode.ttf".into()])
            .collect()
    } else {
        ["arial.ttf", "segoeui.ttf", "times.ttf", "tahoma.ttf"]
            .iter()
            .map(|f| format!("{win}\\Fonts\\{f}"))
            .chain(["/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf".into(), "/Library/Fonts/Arial Unicode.ttf".into()])
            .collect()
    };
    candidates.iter().find_map(|p| std::fs::read(p).ok())
}

fn font_family(o: &Options, book: &Book) -> Result<genpdf::fonts::FontFamily<genpdf::fonts::FontData>, String> {
    use genpdf::fonts::{FontData, FontFamily};
    let fd = |b: Vec<u8>| FontData::new(b, None).map_err(|e| format!("Police invalide : {e}"));
    let one = |b: Vec<u8>| -> Result<FontFamily<FontData>, String> {
        let f = fd(b)?;
        Ok(FontFamily { regular: f.clone(), bold: f.clone(), italic: f.clone(), bold_italic: f })
    };
    if let Some(p) = o.pdf_font {
        return one(std::fs::read(p).map_err(|e| format!("Police PDF : {e}"))?);
    }
    let mut cjk = false;
    let mut wide = false;
    for t in book.all_text().chain(book.chapters.iter().flat_map(|c| c.blocks.iter().map(|b| b.src.as_str()))) {
        for c in t.chars() {
            cjk |= is_cjk(c);
            wide |= needs_wide_font(c);
        }
        if cjk {
            break;
        }
    }
    if wide || cjk {
        return match system_font(cjk) {
            Some(b) => one(b),
            None => Err("Aucune police adaptée à cette écriture : choisis une police .ttf dans les réglages, ou sors en EPUB/DOCX/HTML.".into()),
        };
    }
    Ok(FontFamily { regular: fd(CRIMSON_R.to_vec())?, bold: fd(CRIMSON_B.to_vec())?, italic: fd(CRIMSON_I.to_vec())?, bold_italic: fd(CRIMSON_B.to_vec())? })
}

/// genpdf ne coupe qu'aux espaces : on insère des points de coupure dans le texte CJK.
fn wrap_cjk(s: &str) -> String {
    if !s.chars().any(is_cjk) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() * 2);
    let mut run = 0;
    for c in s.chars() {
        out.push(c);
        run = if c == ' ' { 0 } else { run + 1 };
        if is_cjk(c) && run >= 12 {
            out.push(' ');
            run = 0;
        }
    }
    out
}

fn to_pdf(book: &Book, path: &Path, o: &Options) -> Result<(), String> {
    use genpdf::{elements, style::Style, Alignment, Element as _};
    let mut doc = genpdf::Document::new(font_family(o, book)?);
    doc.set_title(book.title.clone());
    doc.set_font_size(11);
    doc.set_line_spacing(1.35);
    let mut deco = genpdf::SimplePageDecorator::new();
    deco.set_margins(genpdf::Margins::trbl(22, 20, 22, 20));
    deco.set_header(|page| {
        let mut p = elements::Paragraph::new(if page > 1 { format!("{page}") } else { String::new() });
        p.set_alignment(Alignment::Center);
        p.styled(Style::new().with_font_size(8))
    });
    doc.set_page_decorator(deco);

    doc.push(elements::Break::new(10));
    doc.push(elements::Paragraph::new(genpdf::style::StyledString::new(wrap_cjk(&book.title), Style::new().bold().with_font_size(26))).aligned(Alignment::Center));
    for ch in &book.chapters {
        doc.push(elements::PageBreak::new());
        for b in &ch.blocks {
            let text = wrap_cjk(&b.text);
            let (style, gap) = match b.kind {
                Kind::Heading(1) => (Style::new().bold().with_font_size(20), 1.0),
                Kind::Heading(2) => (Style::new().bold().with_font_size(16), 0.8),
                Kind::Heading(_) => (Style::new().bold().with_font_size(13), 0.6),
                Kind::Quote => (Style::new().italic(), 0.4),
                _ => (Style::new(), 0.4),
            };
            let text = if b.kind == Kind::Item { format!("\u{2022} {text}") } else { text };
            let p = elements::Paragraph::new(genpdf::style::StyledString::new(text, style));
            let margins = match b.kind {
                Kind::Quote => genpdf::Margins::trbl(0, 12, 0, 12),
                Kind::Item => genpdf::Margins::trbl(0, 0, 0, 6),
                _ => genpdf::Margins::all(0),
            };
            doc.push(p.padded(margins));
            if let Some(src) = src_of(b, o) {
                let s = Style::new().italic().with_font_size(9).with_color(genpdf::style::Color::Greyscale(110));
                doc.push(elements::Paragraph::new(genpdf::style::StyledString::new(wrap_cjk(src), s)).padded(margins));
            }
            doc.push(elements::Break::new(gap));
        }
    }
    doc.render_to_file(path).map_err(|e| format!("Erreur PDF : {e}"))
}
