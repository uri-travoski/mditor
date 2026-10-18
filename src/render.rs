//! Markdown -> GtkTextBuffer renderer (no WebKit).
//!
//! The whole document is turned into one string plus a list of tag spans and
//! child anchors, then pushed into the buffer in a single `set_text`.

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_CONTENT_WIDTH: i32 = 760;
const OBJ: &str = "\u{FFFC}";

pub struct Tags {
    h: [gtk::TextTag; 6],
    gap: gtk::TextTag,
    tight: gtk::TextTag,
    codeblock: gtk::TextTag,
    quote: gtk::TextTag,
    bold: gtk::TextTag,
    italic: gtk::TextTag,
    strike: gtk::TextTag,
    code: gtk::TextTag,
    link: gtk::TextTag,
    ctx: RefCell<HashMap<(u8, u8), gtk::TextTag>>,
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
        let quote = mk("quote");
        quote.set_property("foreground-rgba", grey(1.0));
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
        Tags { h, gap, tight, codeblock, quote, bold, italic, strike, code, link, ctx: Default::default(), table }
    }

    /// Re-read theme colours (call when the system theme changes).
    pub fn refresh_theme(&self, w: &impl IsA<gtk::Widget>) {
        let c = w
            .style_context()
            .lookup_color("accent_color")
            .unwrap_or_else(|| gdk::RGBA::new(0.21, 0.52, 0.89, 1.0));
        self.link.set_property("foreground-rgba", c);
    }

    /// Margin tag for a (quote depth, list depth) context, created lazily.
    fn ctx(&self, q: u8, l: u8) -> gtk::TextTag {
        self.ctx
            .borrow_mut()
            .entry((q, l))
            .or_insert_with(|| {
                let t = gtk::TextTag::new(Some(&format!("ctx-{q}-{l}")));
                self.table.add(&t);
                let left = q as i32 * 20 + l as i32 * 22;
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
}

#[derive(Default)]
pub struct Rendered {
    /// (source line, char offset in preview) for each leaf block.
    pub blocks: Vec<(usize, i32)>,
    /// (start, end, url) in preview char offsets.
    pub links: Vec<(i32, i32, String)>,
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
}

enum Anchor {
    Rule,
    Table(TableData),
    Image { url: String, alt: String },
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
}

struct Builder<'a> {
    tags: &'a Tags,
    out: String,
    pos: usize,
    spans: Vec<(usize, usize, gtk::TextTag)>,
    anchors: Vec<(usize, Anchor)>,
    blocks: Vec<(usize, i32)>,
    links: Vec<(i32, i32, String)>,
    inline: Vec<(Inline, usize)>,
    quote: u8,
    lists: Vec<Option<u64>>,
    leaf: Option<(usize, usize)>,
    last_line: usize,
    pending: Option<String>,
    image: Option<(usize, usize, String)>,
    table: Option<TableData>,
    in_head: bool,
    row: Vec<String>,
    cell: String,
    cell_open: Vec<&'static str>,
}

impl<'a> Builder<'a> {
    fn new(tags: &'a Tags) -> Self {
        Builder {
            tags,
            out: String::new(),
            pos: 0,
            spans: vec![],
            anchors: vec![],
            blocks: vec![],
            links: vec![],
            inline: vec![],
            quote: 0,
            lists: vec![],
            leaf: None,
            last_line: 0,
            pending: None,
            image: None,
            table: None,
            in_head: false,
            row: vec![],
            cell: String::new(),
            cell_open: vec![],
        }
    }

    fn push(&mut self, s: &str) {
        self.out.push_str(s);
        self.pos += s.chars().count();
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
        self.blocks.push((line, self.pos as i32));
        if let Some(b) = self.pending.take() {
            self.push(&b);
        }
    }

    fn close_leaf(&mut self, base: Option<&gtk::TextTag>, spacing: &gtk::TextTag) {
        let Some((start, byte)) = self.leaf.take() else { return };
        // spacing belongs to the last line only (code blocks, hard breaks)
        let body = self.out[byte..].trim_end_matches('\n');
        let last = match body.rfind('\n') {
            Some(i) => start + body[..=i].chars().count(),
            None => start,
        };
        self.last_line = last;
        self.ensure_line_start();
        let ctx = self.tags.ctx(self.quote, self.lists.len() as u8);
        self.span(start, &ctx);
        self.span(last, spacing);
        if self.quote > 0 {
            let q = self.tags.quote.clone();
            self.span(start, &q);
        }
        if let Some(b) = base {
            self.span(start, b);
        }
    }

    fn spacing(&self) -> gtk::TextTag {
        if self.lists.is_empty() { self.tags.gap.clone() } else { self.tags.tight.clone() }
    }

    fn block_anchor(&mut self, line: usize, a: Anchor) {
        self.ensure_line_start();
        let start = self.pos;
        self.blocks.push((line, start as i32));
        self.anchors.push((start, a));
        self.push(OBJ);
        self.push("\n");
        let ctx = self.tags.ctx(self.quote, self.lists.len() as u8);
        self.span(start, &ctx);
        let gap = self.tags.gap.clone();
        self.span(start, &gap);
    }

    fn text(&mut self, s: &str, line: usize) {
        if self.table.is_some() {
            self.cell.push_str(&glib::markup_escape_text(s));
            return;
        }
        self.begin_leaf(line);
        self.push(s);
    }

    fn start_inline(&mut self, k: Inline, line: usize) {
        if let Some(_) = self.table {
            let m = match k {
                Inline::Emph => "i",
                Inline::Strong => "b",
                Inline::Strike => "s",
                Inline::Link(_) => "u",
            };
            self.cell.push_str(&format!("<{m}>"));
            self.cell_open.push(m);
            return;
        }
        self.begin_leaf(line);
        self.inline.push((k, self.pos));
    }

    fn end_inline(&mut self) {
        if self.table.is_some() {
            if let Some(m) = self.cell_open.pop() {
                self.cell.push_str(&format!("</{m}>"));
            }
            return;
        }
        let Some((k, start)) = self.inline.pop() else { return };
        let tag = match &k {
            Inline::Emph => self.tags.italic.clone(),
            Inline::Strong => self.tags.bold.clone(),
            Inline::Strike => self.tags.strike.clone(),
            Inline::Link(_) => self.tags.link.clone(),
        };
        self.span(start, &tag);
        if let Inline::Link(url) = k {
            if self.pos > start {
                self.links.push((start as i32, self.pos as i32, url));
            }
        }
    }

    fn event(&mut self, ev: Event, line: usize) {
        match ev {
            Event::Start(t) => match t {
                Tag::Paragraph => self.begin_leaf(line),
                Tag::Heading { .. } => self.begin_leaf(line),
                Tag::BlockQuote(_) => self.quote = self.quote.saturating_add(1).min(8),
                Tag::CodeBlock(kind) => {
                    let _ = matches!(kind, CodeBlockKind::Fenced(_));
                    self.begin_leaf(line);
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
                        _ => "\u{2022}\u{2002}".to_string(),
                    };
                    self.pending = Some(bullet);
                    self.blocks.push((line, self.pos as i32));
                    self.begin_leaf(line);
                }
                Tag::Emphasis => self.start_inline(Inline::Emph, line),
                Tag::Strong => self.start_inline(Inline::Strong, line),
                Tag::Strikethrough => self.start_inline(Inline::Strike, line),
                Tag::Link { dest_url, .. } => self.start_inline(Inline::Link(dest_url.to_string()), line),
                Tag::Image { dest_url, .. } => {
                    self.begin_leaf(line);
                    self.image = Some((self.out.len(), self.pos, dest_url.to_string()));
                }
                Tag::Table(aligns) => {
                    let sp = self.tags.gap.clone();
                    self.close_leaf(None, &sp);
                    self.table = Some(TableData { aligns, ..Default::default() });
                }
                Tag::TableHead => {
                    self.in_head = true;
                    self.row.clear();
                }
                Tag::TableRow => self.row.clear(),
                Tag::TableCell => {
                    self.cell.clear();
                    self.cell_open.clear();
                }
                Tag::FootnoteDefinition(label) => {
                    self.begin_leaf(line);
                    self.push(&format!("[{label}] "));
                }
                _ => {}
            },
            Event::End(t) => match t {
                TagEnd::Paragraph => {
                    let sp = self.spacing();
                    self.close_leaf(None, &sp);
                }
                TagEnd::Heading(l) => {
                    let tag = self.tags.h[heading_idx(l)].clone();
                    let sp = self.tags.gap.clone();
                    self.close_leaf(Some(&tag), &sp);
                }
                TagEnd::BlockQuote(_) => self.quote = self.quote.saturating_sub(1),
                TagEnd::CodeBlock => {
                    let tag = self.tags.codeblock.clone();
                    let sp = self.spacing();
                    self.close_leaf(Some(&tag), &sp);
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
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => self.end_inline(),
                TagEnd::Image => {
                    if let Some((byte, chars, url)) = self.image.take() {
                        let alt = self.out.split_off(byte);
                        self.pos = chars;
                        self.anchors.push((self.pos, Anchor::Image { url, alt }));
                        self.push(OBJ);
                    }
                }
                TagEnd::TableCell => {
                    while let Some(m) = self.cell_open.pop() {
                        self.cell.push_str(&format!("</{m}>"));
                    }
                    let c = std::mem::take(&mut self.cell);
                    self.row.push(c);
                }
                TagEnd::TableHead => {
                    if let Some(t) = self.table.as_mut() {
                        t.head = std::mem::take(&mut self.row);
                    }
                    self.in_head = false;
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
            Event::SoftBreak => self.text(" ", line),
            Event::HardBreak => {
                if self.table.is_some() {
                    self.cell.push(' ');
                } else {
                    self.text("\n", line);
                }
            }
            Event::Rule => self.block_anchor(line, Anchor::Rule),
            Event::TaskListMarker(done) => {
                let m = if done { "\u{2611}\u{2002}" } else { "\u{2610}\u{2002}" };
                self.pending = Some(m.to_string());
                if self.leaf.is_some() && self.out.ends_with(['\u{2022}', '\u{2002}']) {
                    // bullet already emitted: replace it
                    let cut = self.out.rfind('\u{2022}').unwrap_or(self.out.len());
                    let removed = self.out.split_off(cut);
                    self.pos -= removed.chars().count();
                    let m = self.pending.take().unwrap();
                    self.push(&m);
                }
            }
            Event::FootnoteReference(l) => self.text(&format!("[{l}]"), line),
            Event::Html(s) => {
                if self.table.is_none() {
                    self.text(&s, line);
                }
            }
            Event::InlineHtml(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
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

pub fn render(view: &gtk::TextView, tags: &Tags, md: &str, base: Option<&Path>) -> Rendered {
    let mut line_starts = vec![0usize];
    line_starts.extend(md.match_indices('\n').map(|(i, _)| i + 1));

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let mut b = Builder::new(tags);
    for (ev, range) in Parser::new_ext(md, opts).into_offset_iter() {
        let line = line_starts.partition_point(|&s| s <= range.start).saturating_sub(1);
        b.event(ev, line);
    }
    let sp = b.tags.gap.clone();
    b.close_leaf(None, &sp);

    let buf = view.buffer();
    buf.set_text(&b.out);

    // Anchors first (net zero char change per anchor), then tags.
    for (off, a) in b.anchors {
        let mut it = buf.iter_at_offset(off as i32);
        let mut end = buf.iter_at_offset(off as i32 + 1);
        buf.delete(&mut it, &mut end);
        let anchor = buf.create_child_anchor(&mut it);
        let w = match a {
            Anchor::Rule => rule_widget(),
            Anchor::Table(t) => table_widget(&t),
            Anchor::Image { url, alt } => image_widget(&url, &alt, base),
        };
        view.add_child_at_anchor(&w, &anchor);
    }
    for (s, e, tag) in &b.spans {
        buf.apply_tag(tag, &buf.iter_at_offset(*s as i32), &buf.iter_at_offset(*e as i32));
    }
    Rendered { blocks: b.blocks, links: b.links }
}

fn rule_widget() -> gtk::Widget {
    let s = gtk::Separator::new(gtk::Orientation::Horizontal);
    s.set_size_request(MAX_CONTENT_WIDTH - 40, 1);
    s.set_valign(gtk::Align::Center);
    s.upcast()
}

fn table_widget(t: &TableData) -> gtk::Widget {
    let grid = gtk::Grid::builder().column_spacing(18).row_spacing(6).margin_top(4).margin_bottom(4).build();
    let cell = |markup: &str, col: usize, bold: bool| {
        let m = if bold { format!("<b>{markup}</b>") } else { markup.to_string() };
        let l = gtk::Label::builder().use_markup(true).wrap(true).max_width_chars(48).build();
        l.set_markup(&m);
        l.set_xalign(match t.aligns.get(col) {
            Some(Alignment::Center) => 0.5,
            Some(Alignment::Right) => 1.0,
            _ => 0.0,
        });
        l
    };
    for (c, h) in t.head.iter().enumerate() {
        grid.attach(&cell(h, c, true), c as i32, 0, 1, 1);
    }
    grid.attach(&gtk::Separator::new(gtk::Orientation::Horizontal), 0, 1, t.head.len().max(1) as i32, 1);
    for (r, row) in t.rows.iter().enumerate() {
        for (c, txt) in row.iter().enumerate() {
            grid.attach(&cell(txt, c, false), c as i32, r as i32 + 2, 1, 1);
        }
    }
    grid.upcast()
}

fn resolve_image(url: &str, base: Option<&Path>) -> Option<PathBuf> {
    if url.contains("://") && !url.starts_with("file://") {
        return None;
    }
    let u = url.strip_prefix("file://").unwrap_or(url);
    let u = glib::uri_unescape_string(u, None::<&str>).map(|s| s.to_string()).unwrap_or_else(|| u.to_string());
    let p = PathBuf::from(u);
    if p.is_absolute() { Some(p) } else { base.map(|b| b.join(p)) }
}

fn image_widget(url: &str, alt: &str, base: Option<&Path>) -> gtk::Widget {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let label = gtk::Label::new(Some(if alt.is_empty() { url } else { alt }));
    label.add_css_class("dim-label");
    holder.append(&label);
    if let Some(path) = resolve_image(url, base) {
        let weak = holder.downgrade();
        let alt = alt.to_string();
        glib::spawn_future_local(async move {
            let bytes = gio::spawn_blocking(move || std::fs::read(path).ok()).await.ok().flatten();
            let (Some(bytes), Some(holder)) = (bytes, weak.upgrade()) else { return };
            let Ok(tex) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) else { return };
            let (mut w, mut h) = (tex.width(), tex.height());
            if w > MAX_CONTENT_WIDTH - 40 {
                h = h * (MAX_CONTENT_WIDTH - 40) / w;
                w = MAX_CONTENT_WIDTH - 40;
            }
            let pic = gtk::Picture::for_paintable(&tex);
            pic.set_can_shrink(true);
            pic.set_size_request(w, h);
            pic.set_tooltip_text(Some(&alt));
            while let Some(c) = holder.first_child() {
                holder.remove(&c);
            }
            holder.append(&pic);
        });
    }
    holder.upcast()
}
