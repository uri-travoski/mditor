//! Markdown -> GtkTextBuffer renderer (no WebKit), Obsidian flavoured.
//!
//! The whole document is turned into one string plus a list of tag spans and
//! child anchors, then pushed into the buffer in a single `set_text`.

use crate::obs::{self, Hl};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const MAX_CONTENT_WIDTH: i32 = 950;
const OBJ: &str = "\u{FFFC}";

pub type LinkCb = Rc<dyn Fn(&str)>;

pub struct Tags {
    h: [gtk::TextTag; 6],
    gap: gtk::TextTag,
    tight: gtk::TextTag,
    codeblock: gtk::TextTag,
    bold: gtk::TextTag,
    italic: gtk::TextTag,
    strike: gtk::TextTag,
    code: gtk::TextTag,
    link: gtk::TextTag,
    mark: gtk::TextTag,
    tagpill: gtk::TextTag,
    sup: gtk::TextTag,
    sub: gtk::TextTag,
    small: gtk::TextTag,
    kbd: gtk::TextTag,
    qbar: gtk::TextTag,
    math: gtk::TextTag,
    propkey: gtk::TextTag,
    propbox: gtk::TextTag,
    kw: gtk::TextTag,
    st: gtk::TextTag,
    cm: gtk::TextTag,
    nu: gtk::TextTag,
    ctx: RefCell<HashMap<(u8, u8), gtk::TextTag>>,
    callouts: RefCell<HashMap<&'static str, (gtk::TextTag, gtk::TextTag)>>,
    table: gtk::TextTagTable,
}

fn grey(a: f32) -> gdk::RGBA {
    gdk::RGBA::new(0.5, 0.5, 0.5, a)
}

impl Tags {
    pub fn new(buf: &gtk::TextBuffer) -> Self {
        let table = buf.tag_table();
        let mk = |name: &str| {
            let t = gtk::TextTag::new(Some(name));
            table.add(&t);
            t
        };
        let scales = [2.0f64, 1.6, 1.35, 1.15, 1.0, 0.9];
        let h: [gtk::TextTag; 6] = std::array::from_fn(|i| {
            let t = mk(&format!("h{}", i + 1));
            t.set_property("scale", scales[i]);
            t.set_property("weight", 700i32);
            t.set_property("pixels-above-lines", if i < 2 { 14i32 } else { 8 });
            t.set_property("pixels-below-lines", 8i32);
            t
        });
        let tight = mk("tight");
        tight.set_property("pixels-below-lines", 3i32);
        let gap = mk("gap");
        gap.set_property("pixels-below-lines", 12i32);
        let codeblock = mk("codeblock");
        codeblock.set_property("family", "monospace");
        codeblock.set_property("scale", 0.92f64);
        codeblock.set_property("paragraph-background-rgba", grey(0.15));
        codeblock.set_property("wrap-mode", gtk::WrapMode::Char);
        let bold = mk("bold");
        bold.set_property("weight", 700i32);
        let italic = mk("italic");
        italic.set_property("style", pango::Style::Italic);
        let strike = mk("strike");
        strike.set_property("strikethrough", true);
        let code = mk("code");
        code.set_property("family", "monospace");
        code.set_property("scale", 0.92f64);
        code.set_property("background-rgba", grey(0.2));
        let link = mk("link");
        link.set_property("underline", pango::Underline::Single);
        let mark = mk("mark");
        mark.set_property("background-rgba", gdk::RGBA::new(1.0, 0.82, 0.0, 0.38));
        let tagpill = mk("tagpill");
        let sup = mk("sup");
        sup.set_property("scale", 0.75f64);
        sup.set_property("rise", 4096i32);
        let sub = mk("sub");
        sub.set_property("scale", 0.75f64);
        sub.set_property("rise", -2048i32);
        let small = mk("small");
        small.set_property("scale", 0.85f64);
        let kbd = mk("kbd");
        kbd.set_property("family", "monospace");
        kbd.set_property("scale", 0.88f64);
        kbd.set_property("background-rgba", grey(0.25));
        let qbar = mk("qbar");
        qbar.set_property("foreground-rgba", grey(0.6));
        qbar.set_property("weight", 900i32);
        let math = mk("math");
        math.set_property("style", pango::Style::Italic);
        math.set_property("family", "serif");
        let propkey = mk("propkey");
        propkey.set_property("weight", 700i32);
        propkey.set_property("foreground-rgba", grey(0.85));
        let propbox = mk("propbox");
        propbox.set_property("paragraph-background-rgba", grey(0.1));
        propbox.set_property("scale", 0.92f64);
        let kw = mk("hl-kw");
        let st = mk("hl-str");
        let cm = mk("hl-com");
        cm.set_property("style", pango::Style::Italic);
        let nu = mk("hl-num");
        Tags {
            h, gap, tight, codeblock, bold, italic, strike, code, link, mark, tagpill, sup, sub, small, kbd, qbar, math,
            propkey, propbox, kw, st, cm, nu,
            ctx: Default::default(),
            callouts: Default::default(),
            table,
        }
    }

    /// Re-read theme colours (call when the system theme changes).
    pub fn refresh_theme(&self, w: &impl IsA<gtk::Widget>) {
        let c = w
            .style_context()
            .lookup_color("accent_color")
            .unwrap_or_else(|| gdk::RGBA::new(0.21, 0.52, 0.89, 1.0));
        self.link.set_property("foreground-rgba", c);
        self.tagpill.set_property("foreground-rgba", c);
        self.tagpill.set_property("background-rgba", gdk::RGBA::new(c.red(), c.green(), c.blue(), 0.16));
        let fg = w.color();
        let dark = fg.red() + fg.green() + fg.blue() > 1.5;
        let rgb = |l: (f32, f32, f32), d: (f32, f32, f32)| {
            let (r, g, b) = if dark { d } else { l };
            gdk::RGBA::new(r, g, b, 1.0)
        };
        self.kw.set_property("foreground-rgba", rgb((0.60, 0.20, 0.75), (0.78, 0.55, 0.90)));
        self.st.set_property("foreground-rgba", rgb((0.16, 0.50, 0.20), (0.60, 0.80, 0.45)));
        self.cm.set_property("foreground-rgba", rgb((0.45, 0.45, 0.45), (0.50, 0.55, 0.60)));
        self.nu.set_property("foreground-rgba", rgb((0.78, 0.38, 0.0), (0.90, 0.65, 0.40)));
    }

    /// Margin tag for a (quote depth, list depth) context, created lazily.
    fn ctx(&self, q: u8, l: u8) -> gtk::TextTag {
        self.ctx
            .borrow_mut()
            .entry((q, l))
            .or_insert_with(|| {
                let t = gtk::TextTag::new(Some(&format!("ctx-{q}-{l}")));
                self.table.add(&t);
                let left = q as i32 * 14 + l as i32 * 22;
                if left > 0 {
                    t.set_property("left-margin", left);
                }
                if l > 0 {
                    t.set_property("indent", -18i32);
                }
                t
            })
            .clone()
    }

    fn callout(&self, kind: &'static str, rgb: (f32, f32, f32)) -> (gtk::TextTag, gtk::TextTag) {
        self.callouts
            .borrow_mut()
            .entry(kind)
            .or_insert_with(|| {
                let bg = gtk::TextTag::new(Some(&format!("co-bg-{kind}")));
                bg.set_property("paragraph-background-rgba", gdk::RGBA::new(rgb.0, rgb.1, rgb.2, 0.11));
                let title = gtk::TextTag::new(Some(&format!("co-title-{kind}")));
                title.set_property("foreground-rgba", gdk::RGBA::new(rgb.0, rgb.1, rgb.2, 1.0));
                title.set_property("weight", 700i32);
                self.table.add(&bg);
                self.table.add(&title);
                (bg, title)
            })
            .clone()
    }
}

// ------------------------------------------------------------ results

pub struct Fold {
    pub start: i32,
    pub end: i32,
    content: gtk::TextTag,
    a_open: gtk::TextTag,
    a_closed: gtk::TextTag,
    open: Cell<bool>,
}

impl Fold {
    pub fn toggle(&self) {
        let o = !self.open.get();
        self.open.set(o);
        self.content.set_property("invisible", !o);
        self.a_open.set_property("invisible", !o);
        self.a_closed.set_property("invisible", o);
    }
}

#[derive(Default)]
pub struct Rendered {
    /// (source line, char offset in preview) for each leaf block.
    pub blocks: Vec<(usize, i32)>,
    /// (start, end, url) in preview char offsets.
    pub links: Vec<(i32, i32, String)>,
    pub headings: Vec<(String, i32)>,
    pub folds: Vec<Fold>,
    table: Option<gtk::TextTagTable>,
}

impl Drop for Rendered {
    fn drop(&mut self) {
        if let Some(t) = &self.table {
            for f in &self.folds {
                t.remove(&f.content);
                t.remove(&f.a_open);
                t.remove(&f.a_closed);
            }
        }
    }
}

impl Rendered {
    pub fn offset_for_line(&self, line: usize) -> i32 {
        let i = self.blocks.partition_point(|b| b.0 <= line);
        if i == 0 { 0 } else { self.blocks[i - 1].1 }
    }
    pub fn line_for_offset(&self, off: i32) -> usize {
        let i = self.blocks.partition_point(|b| b.1 <= off);
        if i == 0 { 0 } else { self.blocks[i - 1].0 }
    }
    pub fn link_at(&self, off: i32) -> Option<&str> {
        self.links.iter().find(|l| l.0 <= off && off < l.1).map(|l| l.2.as_str())
    }
    pub fn fold_at(&self, off: i32) -> Option<&Fold> {
        self.folds.iter().find(|f| f.start <= off && off < f.end)
    }
    pub fn heading_offset(&self, frag: &str) -> Option<i32> {
        let want = frag.trim().to_lowercase();
        let ws = obs::slug(frag);
        self.headings.iter().find(|h| h.0.to_lowercase() == want || obs::slug(&h.0) == ws).map(|h| h.1)
    }
}

// ------------------------------------------------------------ builder

#[derive(Clone, Copy, PartialEq)]
enum MediaKind {
    Video,
    Audio,
    File,
    Web,
}

enum Anchor {
    Rule,
    Table(TableData),
    Image { url: String, alt: String, w: Option<i32>, h: Option<i32> },
    Media { url: String, kind: MediaKind },
}

#[derive(Default)]
struct TableData {
    aligns: Vec<Alignment>,
    head: Vec<String>,
    rows: Vec<Vec<String>>,
}

enum Inline {
    Emph,
    Strong,
    Strike,
    Link(String),
    Style(String, gtk::TextTag),
}

struct CalloutSt {
    bg: gtk::TextTag,
    title_tag: gtk::TextTag,
    icon: &'static str,
    fold: Option<bool>,
    arrows: Option<(gtk::TextTag, gtk::TextTag)>,
    title_para: bool,
    title_start: usize,
    content_start: Option<usize>,
}

struct Builder<'a> {
    tags: &'a Tags,
    base: Option<&'a Path>,
    vault: Option<obs::Vault>,
    out: String,
    pos: usize,
    spans: Vec<(usize, usize, gtk::TextTag)>,
    anchors: Vec<(usize, Anchor)>,
    blocks: Vec<(usize, i32)>,
    links: Vec<(i32, i32, String)>,
    headings: Vec<(String, i32)>,
    folds: Vec<Fold>,
    inline: Vec<(Inline, usize)>,
    quote_stack: Vec<Option<CalloutSt>>,
    qbase: usize,
    callouts: HashMap<(usize, usize), obs::CalloutInfo>,
    lists: Vec<Option<u64>>,
    leaf: Option<(usize, usize)>,
    last_line: usize,
    cur_line: usize,
    pending: Option<String>,
    bullet: Option<(usize, usize)>,
    image: Option<(usize, usize, String, bool)>,
    wiki_text: bool,
    table: Option<TableData>,
    row: Vec<String>,
    cell: String,
    cell_open: Vec<&'static str>,
    in_code: bool,
    code_lang: String,
    in_meta: bool,
    meta: String,
    hl: Option<usize>,
    comment: bool,
    title_open: Option<usize>,
    html_skip: Option<String>,
    media_pending: Option<MediaKind>,
    embedding: u8,
    visiting: Vec<PathBuf>,
}

fn opts() -> Options {
    let mut o = Options::empty();
    o.insert(Options::ENABLE_TABLES);
    o.insert(Options::ENABLE_FOOTNOTES);
    o.insert(Options::ENABLE_STRIKETHROUGH);
    o.insert(Options::ENABLE_TASKLISTS);
    o.insert(Options::ENABLE_WIKILINKS);
    o.insert(Options::ENABLE_MATH);
    o.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    o
}

fn ext_of(url: &str) -> String {
    let u = url.split(['?', '#']).next().unwrap_or(url);
    u.rsplit_once('.').map(|x| x.1).filter(|e| e.len() <= 5 && !e.contains('/')).unwrap_or("").to_lowercase()
}

fn media_kind(url: &str) -> Option<MediaKind> {
    match ext_of(url).as_str() {
        "mp4" | "webm" | "ogv" | "mov" | "mkv" | "m4v" | "avi" => Some(MediaKind::Video),
        "mp3" | "wav" | "ogg" | "oga" | "m4a" | "flac" | "opus" | "aac" => Some(MediaKind::Audio),
        "pdf" | "zip" | "docx" | "xlsx" | "pptx" | "csv" | "canvas" | "epub" => Some(MediaKind::File),
        _ => None,
    }
}

fn web_embed(url: &str) -> bool {
    youtube_id(url).is_some() || ["vimeo.com", "twitter.com", "x.com", "soundcloud.com", "spotify.com", "codepen.io"].iter().any(|h| url.contains(h))
}

fn youtube_id(url: &str) -> Option<String> {
    let id = |s: &str| s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>();
    if let Some(i) = url.find("youtu.be/") {
        return Some(id(&url[i + 9..])).filter(|s| !s.is_empty());
    }
    if url.contains("youtube.com") || url.contains("youtube-nocookie.com") {
        if let Some(i) = url.find("v=") {
            return Some(id(&url[i + 2..])).filter(|s| !s.is_empty());
        }
        for k in ["/embed/", "/shorts/", "/live/"] {
            if let Some(i) = url.find(k) {
                return Some(id(&url[i + k.len()..])).filter(|s| !s.is_empty());
            }
        }
    }
    None
}

/// "alt|300x200" -> (alt, Some(300), Some(200))
fn split_size(alt: &str) -> (String, Option<i32>, Option<i32>) {
    let parse = |s: &str| s.trim().parse::<i32>().ok();
    if let Some((a, sz)) = alt.rsplit_once('|') {
        if let Some((w, h)) = sz.split_once(['x', 'X']) {
            if let (Some(w), Some(h)) = (parse(w), parse(h)) {
                return (a.to_string(), Some(w), Some(h));
            }
        } else if let Some(w) = parse(sz) {
            return (a.to_string(), Some(w), None);
        }
    } else if let Some(w) = parse(alt) {
        return (String::new(), Some(w), None);
    }
    (alt.to_string(), None, None)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let low = tag.to_lowercase();
    let mut from = 0;
    while let Some(i) = low[from..].find(name) {
        let at = from + i;
        let before_ok = at == 0 || low.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[at + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let v = rest[1..].trim_start();
            let (q, body) = match v.chars().next() {
                Some(q @ ('"' | '\'')) => (Some(q), &v[1..]),
                _ => (None, v),
            };
            let end = match q {
                Some(q) => body.find(q).unwrap_or(body.len()),
                None => body.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(body.len()),
            };
            return Some(body[..end].to_string());
        }
        from = at + name.len();
    }
    None
}

fn pango_tag(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "b" | "strong" => ("<b>", "</b>"),
        "i" | "em" | "cite" => ("<i>", "</i>"),
        "u" | "ins" => ("<u>", "</u>"),
        "s" | "del" | "strike" => ("<s>", "</s>"),
        "sub" => ("<sub>", "</sub>"),
        "sup" => ("<sup>", "</sup>"),
        "small" => ("<small>", "</small>"),
        "code" | "kbd" | "tt" => ("<tt>", "</tt>"),
        "mark" => ("<span background=\"#ffd10061\">", "</span>"),
        _ => return None,
    })
}

impl<'a> Builder<'a> {
    fn new(tags: &'a Tags, base: Option<&'a Path>) -> Self {
        Builder {
            tags,
            base,
            vault: None,
            out: String::new(),
            pos: 0,
            spans: vec![],
            anchors: vec![],
            blocks: vec![],
            links: vec![],
            headings: vec![],
            folds: vec![],
            inline: vec![],
            quote_stack: vec![],
            qbase: 0,
            callouts: HashMap::new(),
            lists: vec![],
            leaf: None,
            last_line: 0,
            cur_line: 0,
            pending: None,
            bullet: None,
            image: None,
            wiki_text: false,
            table: None,
            row: vec![],
            cell: String::new(),
            cell_open: vec![],
            in_code: false,
            code_lang: String::new(),
            in_meta: false,
            meta: String::new(),
            hl: None,
            comment: false,
            title_open: None,
            html_skip: None,
            media_pending: None,
            embedding: 0,
            visiting: vec![],
        }
    }

    fn find_file(&mut self, name: &str) -> Option<PathBuf> {
        let base = self.base?;
        self.vault.get_or_insert_with(|| obs::Vault::new(base)).find(name)
    }

    fn push(&mut self, s: &str) {
        self.out.push_str(s);
        self.pos += s.chars().count();
    }

    fn mark_block(&mut self, line: usize, at: usize) {
        if self.embedding == 0 {
            self.blocks.push((line, at as i32));
        }
    }

    fn ensure_line_start(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.push("\n");
        }
    }

    fn span(&mut self, start: usize, tag: &gtk::TextTag) {
        if self.pos > start {
            self.spans.push((start, self.pos, tag.clone()));
        }
    }

    fn begin_leaf(&mut self, line: usize) {
        if self.leaf.is_some() {
            return;
        }
        self.ensure_line_start();
        self.leaf = Some((self.pos, self.out.len()));
        self.mark_block(line, self.pos);
        if let Some(b) = self.pending.take() {
            self.bullet = Some((self.out.len(), self.pos));
            self.push(&b);
        } else if matches!(self.quote_stack.last(), Some(None)) {
            let s = self.pos;
            self.push("\u{258E}\u{2002}");
            let t = self.tags.qbar.clone();
            self.span(s, &t);
        }
    }

    fn strip_block_id(&mut self, byte: usize) {
        let body = &self.out[byte..];
        let t = body.trim_end();
        if let Some((i, wc)) = t.char_indices().rev().find(|(_, c)| c.is_whitespace()) {
            let tok = &t[i + wc.len_utf8()..];
            if tok.len() > 1 && tok.starts_with('^') && tok[1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                let cut = byte + i;
                let removed = self.out[cut..].chars().count();
                self.out.truncate(cut);
                self.pos -= removed;
            }
        }
    }

    fn close_leaf(&mut self, base: Option<&gtk::TextTag>, spacing: &gtk::TextTag) {
        let Some((start, byte)) = self.leaf.take() else { return };
        self.hl = None;
        if !self.in_code {
            self.strip_block_id(byte);
        }
        // spacing belongs to the last line only (code blocks, hard breaks)
        let body = self.out[byte..].trim_end_matches('\n');
        let last = match body.rfind('\n') {
            Some(i) => start + body[..=i].chars().count(),
            None => start,
        };
        self.last_line = last;
        self.ensure_line_start();
        let ctx = self.tags.ctx(self.quote_stack.len() as u8, self.lists.len() as u8);
        self.span(start, &ctx);
        self.span(last, spacing);
        if let Some(Some(c)) = self.quote_stack.iter().rev().find(|c| c.is_some()) {
            let bg = c.bg.clone();
            self.span(start, &bg);
        }
        if let Some(b) = base {
            self.span(start, b);
        }
    }

    fn flush_block(&mut self) {
        let sp = self.tags.gap.clone();
        self.close_leaf(None, &sp);
    }

    fn spacing(&self) -> gtk::TextTag {
        if self.lists.is_empty() { self.tags.gap.clone() } else { self.tags.tight.clone() }
    }

    fn block_anchor(&mut self, line: usize, a: Anchor) {
        self.ensure_line_start();
        let start = self.pos;
        self.mark_block(line, start);
        self.anchors.push((start, a));
        self.push(OBJ);
        self.push("\n");
        let ctx = self.tags.ctx(self.quote_stack.len() as u8, self.lists.len() as u8);
        self.span(start, &ctx);
        let gap = self.tags.gap.clone();
        self.span(start, &gap);
        if let Some(Some(c)) = self.quote_stack.iter().rev().find(|c| c.is_some()) {
            let bg = c.bg.clone();
            self.span(start, &bg);
        }
    }

    fn prev_char(&self, s: &str, i: usize) -> Option<char> {
        if i > 0 { s[..i].chars().next_back() } else { self.out.chars().next_back() }
    }

    fn emit(&mut self, seg: &str, line: usize) {
        if !seg.is_empty() {
            self.begin_leaf(line);
            self.push(seg);
        }
    }

    fn text(&mut self, s: &str, line: usize) {
        if self.table.is_some() {
            let esc = glib::markup_escape_text(s).to_string();
            let mut out = String::new();
            let mut open = false;
            for (i, part) in esc.split("==").enumerate() {
                if i > 0 {
                    out.push_str(if open { "</span>" } else { "<span background=\"#ffd10061\">" });
                    open = !open;
                }
                out.push_str(part);
            }
            if open {
                out.push_str("</span>");
            }
            self.cell.push_str(&out);
            return;
        }
        if self.in_meta {
            self.meta.push_str(s);
            return;
        }
        if self.in_code {
            self.begin_leaf(line);
            self.push(s);
            return;
        }
        let in_link = self.inline.iter().any(|(k, _)| matches!(k, Inline::Link(_)));
        let mut i = 0;
        let mut seg = 0;
        while i < s.len() {
            let rest = &s[i..];
            let c = rest.chars().next().unwrap();
            if rest.starts_with("%%") {
                self.emit(&s[seg..i], line);
                self.comment = !self.comment;
                i += 2;
                seg = i;
                continue;
            }
            if self.comment {
                i += c.len_utf8();
                seg = i;
                continue;
            }
            if rest.starts_with("==") && !rest.starts_with("===") {
                let next = rest[2..].chars().next();
                let prev = self.prev_char(s, i);
                let opening = self.hl.is_none() && next.is_some_and(|n| !n.is_whitespace());
                let closing = self.hl.is_some() && prev.is_some_and(|p| !p.is_whitespace());
                if opening || closing {
                    self.emit(&s[seg..i], line);
                    if closing {
                        let st = self.hl.take().unwrap();
                        let m = self.tags.mark.clone();
                        self.span(st, &m);
                    } else {
                        self.begin_leaf(line);
                        self.hl = Some(self.pos);
                    }
                    i += 2;
                    seg = i;
                    continue;
                }
            }
            if c == '#' && !in_link && self.prev_char(s, i).map_or(true, |p| p.is_whitespace()) {
                let word: String = rest[1..].chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/')).collect();
                if !word.is_empty() && word.chars().any(|c| !c.is_ascii_digit()) {
                    self.emit(&s[seg..i], line);
                    self.begin_leaf(line);
                    let st = self.pos;
                    self.push(&format!("#{word}"));
                    let t = self.tags.tagpill.clone();
                    self.span(st, &t);
                    i += 1 + word.len();
                    seg = i;
                    continue;
                }
            }
            if !in_link && (rest.starts_with("http://") || rest.starts_with("https://")) && !self.prev_char(s, i).is_some_and(|p| p.is_alphanumeric()) {
                let end = rest.find(|c: char| c.is_whitespace() || c == '<' || c == '>').unwrap_or(rest.len());
                let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '\'', '"']);
                if url.len() > 8 {
                    self.emit(&s[seg..i], line);
                    self.begin_leaf(line);
                    let st = self.pos;
                    let url = url.to_string();
                    self.push(&url);
                    let t = self.tags.link.clone();
                    self.span(st, &t);
                    self.links.push((st as i32, self.pos as i32, url.clone()));
                    i += url.len();
                    seg = i;
                    continue;
                }
            }
            i += c.len_utf8();
        }
        let tail = &s[seg..];
        if self.wiki_text {
            let t = tail.trim_start_matches(['#', '^']).replace('#', " \u{203A} ").replace('^', " \u{203A} ");
            self.emit(&t, line);
        } else {
            self.emit(tail, line);
        }
    }

    /// Line break inside a paragraph; plain quotes repeat their bar.
    fn newline(&mut self, line: usize) {
        self.text("\n", line);
        if self.table.is_none() && self.pending.is_none() && matches!(self.quote_stack.last(), Some(None)) && self.leaf.is_some() {
            let s = self.pos;
            self.push("\u{258E}\u{2002}");
            let t = self.tags.qbar.clone();
            self.span(s, &t);
        }
    }

    fn start_inline(&mut self, k: Inline, line: usize) {
        if self.table.is_some() {
            let (open, close): (String, &'static str) = match &k {
                Inline::Emph => ("<i>".into(), "</i>"),
                Inline::Strong => ("<b>".into(), "</b>"),
                Inline::Strike => ("<s>".into(), "</s>"),
                Inline::Link(u) => (format!("<a href=\"{}\">", glib::markup_escape_text(u)), "</a>"),
                Inline::Style(n, _) => match pango_tag(n) {
                    Some((o, c)) => (o.into(), c),
                    None => return,
                },
            };
            self.cell.push_str(&open);
            self.cell_open.push(close);
            return;
        }
        self.begin_leaf(line);
        self.inline.push((k, self.pos));
    }

    fn end_inline(&mut self) {
        if self.table.is_some() {
            if let Some(m) = self.cell_open.pop() {
                self.cell.push_str(m);
            }
            return;
        }
        let Some((k, start)) = self.inline.pop() else { return };
        let tag = match &k {
            Inline::Emph => self.tags.italic.clone(),
            Inline::Strong => self.tags.bold.clone(),
            Inline::Strike => self.tags.strike.clone(),
            Inline::Link(_) => self.tags.link.clone(),
            Inline::Style(_, t) => t.clone(),
        };
        self.span(start, &tag);
        if let Inline::Link(url) = k {
            if self.pos > start {
                self.links.push((start as i32, self.pos as i32, url));
            }
        }
    }

    // ------------------------------------------------ callouts

    fn push_callout_prefix(&mut self) {
        let Some(Some(c)) = self.quote_stack.last() else { return };
        let (icon, arrows) = (c.icon, c.arrows.clone());
        if let Some((ao, ac)) = arrows {
            let s = self.pos;
            self.push("\u{25BE}");
            self.span(s, &ao);
            let s = self.pos;
            self.push("\u{25B8}");
            self.span(s, &ac);
            self.push("\u{2002}");
        }
        self.push(icon);
        self.push("\u{2002}");
    }

    fn start_blockquote(&mut self, line: usize) {
        let depth = self.quote_stack.len() + 1 - self.qbase;
        let Some(info) = self.callouts.remove(&(line, depth)) else {
            self.quote_stack.push(None);
            return;
        };
        let ck = obs::callout_kind(&info.kind);
        let (bg, title_tag) = self.tags.callout(ck.name, ck.rgb);
        let arrows = info.fold.map(|open| {
            let ao = gtk::TextTag::new(None);
            let ac = gtk::TextTag::new(None);
            ao.set_property("invisible", !open);
            ac.set_property("invisible", open);
            self.tags.table.add(&ao);
            self.tags.table.add(&ac);
            (ao, ac)
        });
        let default_title = obs::capitalize(&info.kind);
        self.ensure_line_start();
        self.quote_stack.push(Some(CalloutSt {
            bg,
            title_tag: title_tag.clone(),
            icon: ck.icon,
            fold: info.fold,
            arrows,
            title_para: !info.title.is_empty(),
            title_start: self.pos,
            content_start: None,
        }));
        if info.title.is_empty() {
            self.begin_leaf(line);
            self.push_callout_prefix();
            self.push(&default_title);
            let sp = self.tags.tight.clone();
            self.close_leaf(Some(&title_tag), &sp);
            let p = self.pos;
            if let Some(Some(c)) = self.quote_stack.last_mut() {
                c.content_start = Some(p);
            }
        }
    }

    fn end_blockquote(&mut self) {
        let Some(Some(c)) = self.quote_stack.pop() else { return };
        self.ensure_line_start();
        if let (Some(fold), Some((ao, ac)), Some(cs)) = (c.fold, c.arrows, c.content_start) {
            let content = gtk::TextTag::new(None);
            content.set_property("invisible", !fold);
            self.tags.table.add(&content);
            if self.pos > cs {
                self.spans.push((cs, self.pos, content.clone()));
            }
            if self.embedding == 0 {
                self.folds.push(Fold { start: c.title_start as i32, end: cs as i32, content, a_open: ao, a_closed: ac, open: Cell::new(fold) });
            }
        }
    }

    // ------------------------------------------------ embeds

    fn embed_note(&mut self, target: &str, line: usize) {
        let (name, frag) = target.split_once('#').unwrap_or((target, ""));
        let path = if name.is_empty() { None } else { self.find_file(name) };
        let Some(path) = path else {
            self.begin_leaf(line);
            let st = self.pos;
            self.push(&format!("![[{target}]]"));
            let t = self.tags.cm.clone();
            self.span(st, &t);
            return;
        };
        if self.embedding >= 3 || self.visiting.contains(&path) {
            return;
        }
        let Ok(meta) = std::fs::metadata(&path) else { return };
        if meta.len() > 400_000 {
            return;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        let text = obs::section_of(&text, frag);
        self.flush_block();
        self.visiting.push(path);
        self.embedding += 1;
        self.quote_stack.push(None);
        self.feed(&text);
        self.flush_block();
        self.quote_stack.pop();
        self.embedding -= 1;
        self.visiting.pop();
    }

    fn feed(&mut self, md: &str) {
        let (src, callouts) = obs::prep_callouts(md);
        let saved = std::mem::replace(&mut self.callouts, callouts);
        let saved_base = std::mem::replace(&mut self.qbase, self.quote_stack.len());
        let mut line_starts = vec![0usize];
        line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        let mut o = opts();
        if self.embedding > 0 {
            o.remove(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
        }
        for (ev, range) in Parser::new_ext(&src, o).into_offset_iter() {
            let line = line_starts.partition_point(|&s| s <= range.start).saturating_sub(1);
            self.cur_line = line;
            self.event(ev, line);
        }
        self.flush_block();
        self.callouts = saved;
        self.qbase = saved_base;
    }

    fn image_end(&mut self) {
        let Some((byte, chars, url, wiki)) = self.image.take() else { return };
        let raw_alt = self.out.split_off(byte);
        self.pos = chars;
        let (alt, w, h) = split_size(&raw_alt);
        let line = self.cur_line;
        let mut url = url;
        if wiki {
            let name = url.split('#').next().unwrap_or(&url).to_string();
            let ext = ext_of(&name);
            if ext.is_empty() || ext == "md" {
                self.embed_note(&url, line);
                return;
            }
            if let Some(p) = self.find_file(&name) {
                url = format!("file://{}", glib::uri_escape_string(p.to_string_lossy().as_ref(), Some("/"), true));
            }
        }
        let kind = media_kind(&url).or_else(|| if web_embed(&url) { Some(MediaKind::Web) } else { None });
        match kind {
            Some(k) => {
                self.flush_block();
                self.block_anchor(line, Anchor::Media { url, kind: k });
            }
            None => {
                self.anchors.push((self.pos, Anchor::Image { url, alt, w, h }));
                self.push(OBJ);
            }
        }
    }

    // ------------------------------------------------ html

    fn html(&mut self, s: &str, line: usize) {
        let mut i = 0;
        while i < s.len() {
            let Some(k) = s[i..].find('<') else {
                self.html_text(&s[i..], line);
                break;
            };
            let lt = i + k;
            self.html_text(&s[i..lt], line);
            let rest = &s[lt..];
            if rest.starts_with("<!--") {
                match rest.find("-->") {
                    Some(e) => {
                        i = lt + e + 3;
                        continue;
                    }
                    None => break,
                }
            }
            match rest.find('>') {
                None => {
                    self.html_text(rest, line);
                    break;
                }
                Some(g) => {
                    self.html_tag(&rest[1..g], line);
                    i = lt + g + 1;
                }
            }
        }
    }

    fn html_text(&mut self, t: &str, line: usize) {
        if t.is_empty() || self.html_skip.is_some() {
            return;
        }
        if t.contains('\n') || t.contains("  ") {
            let collapsed = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if collapsed.is_empty() {
                return;
            }
            let lead = if t.starts_with(char::is_whitespace) && self.leaf.is_some() { " " } else { "" };
            let trail = if t.ends_with(char::is_whitespace) { " " } else { "" };
            self.text(&format!("{lead}{collapsed}{trail}"), line);
        } else {
            self.text(t, line);
        }
    }

    fn end_html(&mut self, name: &str) {
        if self.table.is_some() {
            if pango_tag(name).is_some() || name == "a" {
                self.end_inline();
            }
            return;
        }
        let matches = |k: &Inline| match k {
            Inline::Style(n, _) => n == name,
            Inline::Link(_) => name == "a",
            _ => false,
        };
        if !self.inline.iter().any(|(k, _)| matches(k)) {
            return;
        }
        while let Some((k, _)) = self.inline.last() {
            let m = matches(k);
            self.end_inline();
            if m {
                break;
            }
        }
    }

    fn html_tag(&mut self, tag: &str, line: usize) {
        let t = tag.trim();
        let closing = t.starts_with('/');
        let t = t.trim_start_matches('/').trim_end_matches('/').trim();
        let name: String = t.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
        if let Some(skip) = &self.html_skip {
            if closing && *skip == name {
                self.html_skip = None;
            }
            return;
        }
        let tags = self.tags;
        let style = |n: &str| -> Option<gtk::TextTag> {
            Some(match n {
                "b" | "strong" => tags.bold.clone(),
                "i" | "em" | "cite" => tags.italic.clone(),
                "u" | "ins" => {
                    let t = gtk::TextTag::new(None);
                    t.set_property("underline", pango::Underline::Single);
                    tags.table.add(&t);
                    t
                }
                "s" | "del" | "strike" => tags.strike.clone(),
                "mark" => tags.mark.clone(),
                "kbd" => tags.kbd.clone(),
                "code" | "tt" => tags.code.clone(),
                "sub" => tags.sub.clone(),
                "sup" => tags.sup.clone(),
                "small" => tags.small.clone(),
                _ => return None,
            })
        };
        match name.as_str() {
            "script" | "style" if !closing => self.html_skip = Some(name),
            "br" => {
                if self.table.is_some() {
                    self.cell.push(' ');
                } else {
                    self.text("\n", line);
                }
            }
            "p" | "div" | "section" | "article" | "details" | "center" | "blockquote" | "figure" | "ul" | "ol" | "table" => {
                if self.table.is_none() {
                    self.flush_block();
                }
            }
            "hr" => {
                self.flush_block();
                self.block_anchor(line, Anchor::Rule);
            }
            "summary" => {
                if closing {
                    self.end_html("summary");
                    self.flush_block();
                } else {
                    self.flush_block();
                    self.start_inline(Inline::Style("summary".into(), self.tags.bold.clone()), line);
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if closing {
                    self.end_html("h");
                    self.flush_block();
                } else {
                    self.flush_block();
                    let n = (name.as_bytes()[1] - b'1') as usize;
                    self.start_inline(Inline::Style("h".into(), self.tags.h[n].clone()), line);
                }
            }
            "a" => {
                if closing {
                    self.end_html("a");
                } else if let Some(h) = attr(t, "href") {
                    self.start_inline(Inline::Link(h), line);
                }
            }
            "img" if !closing && self.table.is_none() => {
                if let Some(src) = attr(t, "src") {
                    let alt = attr(t, "alt").unwrap_or_default();
                    let w = attr(t, "width").and_then(|w| w.trim_end_matches("px").parse::<i32>().ok());
                    let h = attr(t, "height").and_then(|w| w.trim_end_matches("px").parse::<i32>().ok());
                    self.begin_leaf(line);
                    self.anchors.push((self.pos, Anchor::Image { url: src, alt, w, h }));
                    self.push(OBJ);
                }
            }
            "video" | "audio" | "iframe" | "source" | "embed" if !closing && self.table.is_none() => {
                let src = attr(t, "src");
                let kind = match name.as_str() {
                    "video" => MediaKind::Video,
                    "audio" => MediaKind::Audio,
                    "iframe" | "embed" => MediaKind::Web,
                    _ => self.media_pending.take().unwrap_or(MediaKind::Video),
                };
                match src {
                    Some(src) => {
                        self.flush_block();
                        self.block_anchor(line, Anchor::Media { url: src, kind });
                    }
                    None => self.media_pending = Some(kind),
                }
            }
            n => {
                if let Some(tag) = style(n) {
                    if closing {
                        self.end_html(n);
                    } else {
                        self.start_inline(Inline::Style(n.into(), tag), line);
                    }
                }
            }
        }
    }

    // ------------------------------------------------ events

    fn event(&mut self, ev: Event, line: usize) {
        match ev {
            Event::Start(t) => match t {
                Tag::Paragraph => {
                    self.begin_leaf(line);
                    let title = matches!(self.quote_stack.last(), Some(Some(c)) if c.title_para);
                    if title {
                        if let Some(Some(c)) = self.quote_stack.last_mut() {
                            c.title_para = false;
                        }
                        self.title_open = Some(self.leaf.map(|l| l.0).unwrap_or(self.pos));
                        self.push_callout_prefix();
                    }
                }
                Tag::Heading { .. } => self.begin_leaf(line),
                Tag::BlockQuote(_) => {
                    let sp = self.tags.tight.clone();
                    self.close_leaf(None, &sp);
                    self.start_blockquote(line)
                }
                Tag::CodeBlock(kind) => {
                    self.code_lang = match kind {
                        CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_string(),
                        _ => String::new(),
                    };
                    self.in_code = true;
                    self.begin_leaf(line);
                }
                Tag::MetadataBlock(_) => {
                    self.in_meta = true;
                    self.meta.clear();
                }
                Tag::List(first) => {
                    // flush text of a parent item before nesting
                    let sp = self.tags.tight.clone();
                    self.close_leaf(None, &sp);
                    self.lists.push(first);
                }
                Tag::Item => {
                    let sp = self.tags.tight.clone();
                    self.close_leaf(None, &sp);
                    let bullet = match self.lists.last_mut() {
                        Some(Some(n)) => {
                            let s = format!("{n}.\u{2002}");
                            *n += 1;
                            s
                        }
                        _ => match self.lists.len() {
                            1 => "\u{2022}\u{2002}".to_string(),
                            2 => "\u{25E6}\u{2002}".to_string(),
                            _ => "\u{25AA}\u{2002}".to_string(),
                        },
                    };
                    self.pending = Some(bullet);
                    self.begin_leaf(line);
                }
                Tag::Emphasis => self.start_inline(Inline::Emph, line),
                Tag::Strong => self.start_inline(Inline::Strong, line),
                Tag::Strikethrough => self.start_inline(Inline::Strike, line),
                Tag::Link { dest_url, link_type, .. } => {
                    let wiki = matches!(link_type, LinkType::WikiLink { .. });
                    if matches!(link_type, LinkType::WikiLink { has_pothole: false }) {
                        self.wiki_text = true;
                    }
                    let url = if wiki { format!("wiki:{dest_url}") } else { dest_url.to_string() };
                    self.start_inline(Inline::Link(url), line);
                }
                Tag::Image { dest_url, link_type, .. } => {
                    self.begin_leaf(line);
                    let wiki = matches!(link_type, LinkType::WikiLink { .. });
                    self.image = Some((self.out.len(), self.pos, dest_url.to_string(), wiki));
                }
                Tag::Table(aligns) => {
                    self.flush_block();
                    self.table = Some(TableData { aligns, ..Default::default() });
                }
                Tag::TableHead | Tag::TableRow => self.row.clear(),
                Tag::TableCell => {
                    self.cell.clear();
                    self.cell_open.clear();
                }
                Tag::FootnoteDefinition(label) => {
                    self.begin_leaf(line);
                    let s = self.pos;
                    self.push(&format!("[{label}]"));
                    let t = self.tags.sup.clone();
                    self.span(s, &t);
                    self.push(" ");
                }
                _ => {}
            },
            Event::End(t) => match t {
                TagEnd::Paragraph => {
                    if let Some(ts) = self.title_open.take() {
                        let tt = self.quote_stack.last().and_then(|c| c.as_ref()).map(|c| c.title_tag.clone());
                        if let Some(tt) = tt {
                            self.span(ts, &tt);
                            let sp = self.tags.tight.clone();
                            self.close_leaf(None, &sp);
                            let p = self.pos;
                            if let Some(Some(c)) = self.quote_stack.last_mut() {
                                c.content_start = Some(p);
                            }
                        }
                    } else {
                        let sp = self.spacing();
                        self.close_leaf(None, &sp);
                    }
                }
                TagEnd::Heading(l) => {
                    if self.embedding == 0 {
                        if let Some((start, byte)) = self.leaf {
                            let text = self.out[byte..].trim().to_string();
                            let text = text.rsplit_once(" ^").map(|(a, _)| a.to_string()).unwrap_or(text);
                            self.headings.push((text, start as i32));
                        }
                    }
                    let tag = self.tags.h[heading_idx(l)].clone();
                    let sp = self.tags.gap.clone();
                    self.close_leaf(Some(&tag), &sp);
                }
                TagEnd::BlockQuote(_) => {
                    self.flush_block();
                    self.end_blockquote();
                }
                TagEnd::CodeBlock => {
                    if let Some((start, byte)) = self.leaf {
                        let lang = std::mem::take(&mut self.code_lang);
                        let hls = obs::highlight(&lang, &self.out[byte..]);
                        for (a, b, k) in hls {
                            let tag = match k {
                                Hl::Kw => self.tags.kw.clone(),
                                Hl::Str => self.tags.st.clone(),
                                Hl::Com => self.tags.cm.clone(),
                                Hl::Num => self.tags.nu.clone(),
                            };
                            self.spans.push((start + a, start + b, tag));
                        }
                    }
                    let tag = self.tags.codeblock.clone();
                    let sp = self.spacing();
                    self.close_leaf(Some(&tag), &sp);
                    self.in_code = false;
                }
                TagEnd::HtmlBlock => {
                    if self.table.is_none() {
                        self.flush_block();
                    }
                }
                TagEnd::MetadataBlock(_) => {
                    self.in_meta = false;
                    let rows = obs::parse_props(&self.meta);
                    if self.embedding == 0 && !rows.is_empty() {
                        self.begin_leaf(line);
                        let n = rows.len();
                        for (i, (k, v)) in rows.into_iter().enumerate() {
                            let s = self.pos;
                            self.push(&k);
                            let t = self.tags.propkey.clone();
                            self.span(s, &t);
                            self.push(&format!("   {v}"));
                            if i + 1 < n {
                                self.push("\n");
                            }
                        }
                        let tag = self.tags.propbox.clone();
                        let sp = self.tags.gap.clone();
                        self.close_leaf(Some(&tag), &sp);
                    }
                }
                TagEnd::List(_) => {
                    let sp = self.tags.tight.clone();
                    self.close_leaf(None, &sp);
                    self.lists.pop();
                    if self.lists.is_empty() {
                        let gap = self.tags.gap.clone();
                        let from = self.last_line;
                        self.spans.push((from, self.pos, gap));
                    }
                }
                TagEnd::Item => {
                    let sp = self.tags.tight.clone();
                    self.close_leaf(None, &sp);
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.end_inline(),
                TagEnd::Link => {
                    self.wiki_text = false;
                    self.end_inline();
                }
                TagEnd::Image => self.image_end(),
                TagEnd::TableCell => {
                    while let Some(m) = self.cell_open.pop() {
                        self.cell.push_str(m);
                    }
                    let c = std::mem::take(&mut self.cell);
                    self.row.push(c);
                }
                TagEnd::TableHead => {
                    if let Some(t) = self.table.as_mut() {
                        t.head = std::mem::take(&mut self.row);
                    }
                }
                TagEnd::TableRow => {
                    if let Some(t) = self.table.as_mut() {
                        t.rows.push(std::mem::take(&mut self.row));
                    }
                }
                TagEnd::Table => {
                    if let Some(t) = self.table.take() {
                        self.block_anchor(line, Anchor::Table(t));
                    }
                }
                TagEnd::FootnoteDefinition => {
                    let sp = self.tags.tight.clone();
                    self.close_leaf(None, &sp);
                }
                _ => {}
            },
            Event::Text(s) => {
                if self.image.is_some() {
                    self.push(&s);
                } else {
                    self.text(&s, line);
                }
            }
            Event::Code(s) => {
                if self.table.is_some() {
                    self.cell.push_str(&format!("<tt>{}</tt>", glib::markup_escape_text(&s)));
                } else if self.image.is_none() {
                    self.begin_leaf(line);
                    let start = self.pos;
                    self.push(&s);
                    let t = self.tags.code.clone();
                    self.span(start, &t);
                }
            }
            Event::SoftBreak => {
                if self.table.is_some() {
                    self.cell.push(' ');
                } else if let Some(ts) = self.title_open.take() {
                    // callout title is only the first line
                    let tt = self.quote_stack.last().and_then(|c| c.as_ref()).map(|c| c.title_tag.clone());
                    if let Some(tt) = tt {
                        self.span(ts, &tt);
                    }
                    self.push("\n");
                    let tight = self.tags.tight.clone();
                    self.span(ts, &tight);
                    let p = self.pos;
                    if let Some(Some(c)) = self.quote_stack.last_mut() {
                        c.content_start = Some(p);
                    }
                } else if self.image.is_some() {
                    self.push(" ");
                } else {
                    // Obsidian shows single newlines as line breaks
                    self.newline(line);
                }
            }
            Event::HardBreak => {
                if self.table.is_some() {
                    self.cell.push(' ');
                } else {
                    self.newline(line);
                }
            }
            Event::Rule => {
                self.flush_block();
                self.block_anchor(line, Anchor::Rule)
            }
            Event::TaskListMarker(done) => {
                let m = if done { "\u{2611}\u{2002}" } else { "\u{2610}\u{2002}" };
                if let Some((byte, chars)) = self.bullet.take() {
                    self.out.truncate(byte);
                    self.pos = chars;
                }
                self.push(m);
            }
            Event::FootnoteReference(l) => {
                if self.table.is_some() {
                    self.cell.push_str(&format!("<sup>[{}]</sup>", glib::markup_escape_text(&l)));
                } else {
                    self.begin_leaf(line);
                    let s = self.pos;
                    self.push(&format!("[{l}]"));
                    let t = self.tags.sup.clone();
                    self.span(s, &t);
                }
            }
            Event::Html(s) | Event::InlineHtml(s) => self.html(&s, line),
            Event::InlineMath(s) => {
                if self.table.is_some() {
                    self.cell.push_str(&format!("<i>{}</i>", glib::markup_escape_text(&obs::latex(&s))));
                } else {
                    self.begin_leaf(line);
                    let st = self.pos;
                    self.push(&obs::latex(&s));
                    let t = self.tags.math.clone();
                    self.span(st, &t);
                }
            }
            Event::DisplayMath(s) => {
                self.begin_leaf(line);
                if self.leaf.is_some_and(|l| self.pos > l.0) {
                    self.push("\n");
                }
                let st = self.pos;
                self.push(obs::latex(&s).trim());
                let t = self.tags.math.clone();
                self.span(st, &t);
            }
        }
    }
}

fn heading_idx(l: HeadingLevel) -> usize {
    match l {
        HeadingLevel::H1 => 0,
        HeadingLevel::H2 => 1,
        HeadingLevel::H3 => 2,
        HeadingLevel::H4 => 3,
        HeadingLevel::H5 => 4,
        HeadingLevel::H6 => 5,
    }
}

pub fn render(view: &gtk::TextView, tags: &Tags, md: &str, base: Option<&Path>, cb: &LinkCb) -> Rendered {
    let mut b = Builder::new(tags, base);
    b.feed(md);

    let buf = view.buffer();
    buf.set_text(&b.out);

    // Anchors first (net zero char change per anchor), then tags.
    for (off, a) in b.anchors {
        let mut it = buf.iter_at_offset(off as i32);
        let mut end = buf.iter_at_offset(off as i32 + 1);
        buf.delete(&mut it, &mut end);
        let anchor = buf.create_child_anchor(&mut it);
        let w = match a {
            Anchor::Rule => {
                let r = rule_widget();
                fit_width(view, &r);
                r
            }
            Anchor::Table(t) => {
                let g = table_widget(&t, cb);
                fit_width(view, &g);
                g
            }
            Anchor::Image { url, alt, w, h } => image_widget(&url, &alt, w, h, base),
            Anchor::Media { url, kind } => media_widget(&url, kind, base),
        };
        view.add_child_at_anchor(&w, &anchor);
    }
    for (s, e, tag) in &b.spans {
        buf.apply_tag(tag, &buf.iter_at_offset(*s as i32), &buf.iter_at_offset(*e as i32));
    }
    Rendered { blocks: b.blocks, links: b.links, headings: b.headings, folds: b.folds, table: Some(buf.tag_table()) }
}

fn rule_widget() -> gtk::Widget {
    let s = gtk::Separator::new(gtk::Orientation::Horizontal);
    s.set_size_request(1, 1);
    s.set_valign(gtk::Align::Center);
    s.upcast()
}

fn table_widget(t: &TableData, cb: &LinkCb) -> gtk::Widget {
    let grid = gtk::Grid::builder().margin_top(6).margin_bottom(10).hexpand(true).build();
    grid.add_css_class("mdtable");
    let cols = t.head.len().max(1);
    let cell = |markup: &str, col: usize, head: bool| {
        let m = if head { format!("<b>{markup}</b>") } else { markup.to_string() };
        let l = gtk::Label::builder().use_markup(true).wrap(true).hexpand(true).build();
        l.set_markup(&m);
        l.add_css_class("mdcell");
        if head {
            l.add_css_class("mdhead");
        }
        l.set_xalign(match t.aligns.get(col) {
            Some(Alignment::Center) => 0.5,
            Some(Alignment::Right) => 1.0,
            _ => 0.0,
        });
        l.set_halign(gtk::Align::Fill);
        let cb = cb.clone();
        l.connect_activate_link(move |_, uri| {
            cb(uri);
            glib::Propagation::Stop
        });
        l
    };
    for (c, h) in t.head.iter().enumerate() {
        grid.attach(&cell(h, c, true), c as i32, 0, 1, 1);
    }
    for (r, row) in t.rows.iter().enumerate() {
        for c in 0..cols {
            let txt = row.get(c).map(String::as_str).unwrap_or("");
            grid.attach(&cell(txt, c, false), c as i32, r as i32 + 1, 1, 1);
        }
    }
    grid.upcast()
}

/// Keep an anchored widget as wide as the text column (max width, minus margins).
fn fit_width(view: &gtk::TextView, w: &gtk::Widget) {
    let Some(sw) = view.parent().and_then(|p| p.downcast::<gtk::ScrolledWindow>().ok()) else { return };
    let adj = sw.hadjustment();
    let apply = {
        let w = w.downgrade();
        let view = view.downgrade();
        move |a: &gtk::Adjustment| {
            let (Some(w), Some(view)) = (w.upgrade(), view.upgrade()) else { return };
            // the page size already excludes the view's own margins
            let avail = a.page_size() as i32;
            let _ = &view;
            if avail > 50 {
                w.set_size_request(avail, -1);
            }
        }
    };
    apply(&adj);
    let apply = Rc::new(apply);
    let id = adj.connect_notify_local(Some("page-size"), {
        let apply = apply.clone();
        move |a, _| apply(a)
    });
    let vid = view.connect_notify_local(Some("margin-start"), {
        let apply = apply.clone();
        let adj = adj.downgrade();
        move |_, _| {
            if let Some(a) = adj.upgrade() {
                apply(&a);
            }
        }
    });
    let adj2 = adj.downgrade();
    let view2 = view.downgrade();
    let ids = RefCell::new(Some((id, vid)));
    w.connect_destroy(move |_| {
        if let Some((id, vid)) = ids.borrow_mut().take() {
            if let Some(a) = adj2.upgrade() {
                a.disconnect(id);
            }
            if let Some(v) = view2.upgrade() {
                v.disconnect(vid);
            }
        }
    });
}

// ------------------------------------------------------------ images & media

fn resolve_image(url: &str, base: Option<&Path>) -> Option<PathBuf> {
    if url.contains("://") && !url.starts_with("file://") {
        return None;
    }
    let u = url.strip_prefix("file://").unwrap_or(url);
    let u = glib::uri_unescape_string(u, None::<&str>).map(|s| s.to_string()).unwrap_or_else(|| u.to_string());
    let p = PathBuf::from(u);
    if p.is_absolute() { Some(p) } else { base.map(|b| b.join(p)) }
}

fn is_remote(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let resp = ureq::get(url).set("User-Agent", "mditor").timeout(std::time::Duration::from_secs(15)).call().ok()?;
    let mut v = vec![];
    resp.into_reader().take(25_000_000).read_to_end(&mut v).ok()?;
    Some(v)
}

/// Decode scaled down while loading so big photos do not eat memory.
/// Returns PNG bytes (Send) plus the decoded size.
fn decode(bytes: Vec<u8>, want_w: Option<i32>) -> Option<(i32, i32, Vec<u8>)> {
    let target = want_w.unwrap_or(MAX_CONTENT_WIDTH - 40).clamp(16, MAX_CONTENT_WIDTH - 40);
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.connect_size_prepared(move |l, w, h| {
        if w > target {
            l.set_size(target, (h as i64 * target as i64 / w as i64).max(1) as i32);
        }
    });
    if loader.write(&bytes).is_ok() && loader.close().is_ok() {
        if let Some(pb) = loader.pixbuf() {
            let png = pb.save_to_bufferv("png", &[]).ok()?;
            return Some((pb.width(), pb.height(), png));
        }
    }
    let _ = loader.close();
    None
}

fn image_widget(url: &str, alt: &str, w: Option<i32>, h: Option<i32>, base: Option<&Path>) -> gtk::Widget {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let label = gtk::Label::new(Some(if alt.is_empty() { url } else { alt }));
    label.add_css_class("dim-label");
    holder.append(&label);
    let remote = is_remote(url);
    let path = if remote { None } else { resolve_image(url, base) };
    if !remote && path.is_none() {
        return holder.upcast();
    }
    let url = url.to_string();
    let weak = holder.downgrade();
    let alt = alt.to_string();
    glib::spawn_future_local(async move {
        let dec = gio::spawn_blocking(move || {
            let bytes = if remote { fetch(&url) } else { std::fs::read(path.unwrap()).ok() }?;
            decode(bytes, w)
        })
        .await
        .ok()
        .flatten();
        let (Some(holder), Some((tw, th, png))) = (weak.upgrade(), dec) else { return };
        let Ok(tex) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)) else { return };
        let (mut dw, mut dh) = (tw, th);
        if let Some(want) = w {
            dw = want.min(MAX_CONTENT_WIDTH - 40);
            dh = match h {
                Some(h) => h,
                None => (th as i64 * dw as i64 / tw.max(1) as i64) as i32,
            };
        }
        let pic = gtk::Picture::for_paintable(&tex);
        pic.set_can_shrink(true);
        pic.set_size_request(dw, dh);
        pic.set_halign(gtk::Align::Start);
        pic.set_tooltip_text(Some(&alt));
        while let Some(c) = holder.first_child() {
            holder.remove(&c);
        }
        holder.append(&pic);
    });
    holder.upcast()
}

fn open_external(url: &str, base: Option<&Path>) {
    let uri = if url.contains("://") && !url.starts_with("file://") {
        url.to_string()
    } else {
        match resolve_image(url, base) {
            Some(p) => gio::File::for_path(p).uri().to_string(),
            None => return,
        }
    };
    let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
}

fn media_widget(url: &str, kind: MediaKind, base: Option<&Path>) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("mdmedia");
    card.set_halign(gtk::Align::Start);
    card.set_size_request(420, -1);
    let name = url.split(['?', '#']).next().unwrap_or(url).rsplit('/').next().unwrap_or(url);
    let name = glib::uri_unescape_string(name, None::<&str>).map(|s| s.to_string()).unwrap_or_else(|| name.to_string());
    let yt = youtube_id(url);
    let (glyph, title) = match (kind, &yt) {
        (_, Some(_)) => ("\u{25B6}", "YouTube video".to_string()),
        (MediaKind::Video, _) => ("\u{25B6}", name.clone()),
        (MediaKind::Audio, _) => ("\u{266B}", name.clone()),
        (MediaKind::File, _) => ("\u{1F4C4}", name.clone()),
        (MediaKind::Web, _) => ("\u{1F310}", url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url).to_string()),
    };
    let thumb_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.append(&thumb_slot);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let g = gtk::Label::new(Some(glyph));
    g.add_css_class("title-3");
    row.append(&g);
    let l = gtk::Label::builder().label(&title).xalign(0.0).hexpand(true).ellipsize(pango::EllipsizeMode::Middle).build();
    row.append(&l);
    card.append(&row);

    if let Some(id) = yt.clone() {
        let weak = thumb_slot.downgrade();
        glib::spawn_future_local(async move {
            let dec = gio::spawn_blocking(move || decode(fetch(&format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))?, Some(420))).await.ok().flatten();
            let (Some(slot), Some((w, h, png))) = (weak.upgrade(), dec) else { return };
            let Ok(tex) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)) else { return };
            let pic = gtk::Picture::for_paintable(&tex);
            pic.set_can_shrink(true);
            pic.set_size_request(420, 420 * h / w.max(1));
            slot.append(&pic);
        });
    }

    let click = gtk::GestureClick::new();
    let (url, base) = (url.to_string(), base.map(Path::to_path_buf));
    let local_av = !is_remote(&url) && matches!(kind, MediaKind::Video | MediaKind::Audio) && yt.is_none();
    let played = Cell::new(false);
    let weak = card.downgrade();
    click.connect_released(move |_, _, _, _| {
        if local_av && !played.get() {
            if let (Some(card), Some(p)) = (weak.upgrade(), resolve_image(&url, base.as_deref())) {
                played.set(true);
                while let Some(c) = card.first_child() {
                    card.remove(&c);
                }
                let v = gtk::Video::for_file(Some(&gio::File::for_path(p)));
                v.set_autoplay(true);
                let w = (MAX_CONTENT_WIDTH - 40).min(640);
                v.set_size_request(w, if kind == MediaKind::Video { w * 9 / 16 } else { 56 });
                card.append(&v);
                return;
            }
        }
        open_external(&url, base.as_deref());
    });
    card.add_controller(click);
    card.set_cursor_from_name(Some("pointer"));
    card.upcast()
}
