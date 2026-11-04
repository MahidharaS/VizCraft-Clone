//! The code editor: syntax highlighting (SQL or Python) through a `LayoutJob`, a line-number
//! gutter, the current statement highlighted, and an autocomplete popup.

use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, FontId, Key, Modifiers, Ui, vec2};
use vizardcraft_sql::text::{self, Tok};

use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Sql,
    Python,
}

/// One completion candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    pub text: String,
    /// "table", "column", "keyword", "function", "file", …
    pub kind: &'static str,
    pub detail: String,
}

#[derive(Default)]
pub struct Autocomplete {
    pub open: bool,
    pub items: Vec<Completion>,
    pub selected: usize,
    /// Byte range of the word being completed.
    pub word: Range<usize>,
}

pub struct EditorOut {
    /// Cursor (byte offset).
    pub cursor: usize,
    /// Selected text range (bytes), when there is a selection.
    pub selection: Option<Range<usize>>,
    pub changed: bool,
    pub focused: bool,
}

pub const FONT: f32 = 13.0;

struct Palette {
    keyword: Color32,
    function: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    qident: Color32,
    ident: Color32,
    op: Color32,
}

fn palette(t: &Tokens) -> Palette {
    if t.dark {
        Palette {
            keyword: Color32::from_rgb(0x7A, 0xA2, 0xF7),
            function: Color32::from_rgb(0xC4, 0xA7, 0xFF),
            string: Color32::from_rgb(0x9E, 0xCE, 0x6A),
            number: Color32::from_rgb(0xFF, 0x9E, 0x64),
            comment: Color32::from_rgb(0x6B, 0x75, 0x8A),
            qident: Color32::from_rgb(0x7D, 0xCF, 0xFF),
            ident: t.text,
            op: Color32::from_rgb(0x89, 0xDD, 0xFF),
        }
    } else {
        Palette {
            keyword: Color32::from_rgb(0x1D, 0x4E, 0xD8),
            function: Color32::from_rgb(0x7C, 0x3A, 0xED),
            string: Color32::from_rgb(0x04, 0x78, 0x57),
            number: Color32::from_rgb(0xB4, 0x53, 0x09),
            comment: Color32::from_rgb(0x8A, 0x91, 0x9C),
            qident: Color32::from_rgb(0x0E, 0x74, 0x90),
            ident: t.text,
            op: Color32::from_rgb(0x4B, 0x55, 0x63),
        }
    }
}

const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from",
    "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "print",
];

/// Python tokens: comments `#`, strings, numbers, keywords.
fn tokenize_python(s: &str) -> Vec<(Range<usize>, Tok)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        let kind = if c == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            Tok::Comment
        } else if c == b'\'' || c == b'"' {
            let triple = b.get(i + 1) == Some(&c) && b.get(i + 2) == Some(&c);
            if triple {
                i += 3;
                while i < b.len() && !(b[i] == c && b.get(i + 1) == Some(&c) && b.get(i + 2) == Some(&c)) {
                    i += 1;
                }
                i = (i + 3).min(b.len());
            } else {
                i += 1;
                while i < b.len() && b[i] != c && b[i] != b'\n' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(b.len());
            }
            Tok::Str
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                i += 1;
            }
            Tok::Number
        } else if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
                i += 1;
            }
            let w = &s[start..i];
            if PY_KEYWORDS.contains(&w) {
                Tok::Keyword
            } else if b.get(i) == Some(&b'(') {
                Tok::Function
            } else {
                Tok::Ident
            }
        } else if c.is_ascii_whitespace() {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            Tok::Space
        } else {
            i += 1;
            Tok::Operator
        };
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        out.push((start..i, kind));
    }
    out
}

/// The highlighted layout of `src`.
pub fn highlight(src: &str, lang: Lang, t: &Tokens) -> LayoutJob {
    let p = palette(t);
    let font = FontId::monospace(FONT);
    let mut job = LayoutJob::default();
    let toks = match lang {
        Lang::Sql => text::tokenize(src),
        Lang::Python => tokenize_python(src),
    };
    for (r, k) in toks {
        let color = match k {
            Tok::Keyword => p.keyword,
            Tok::Function => p.function,
            Tok::Str => p.string,
            Tok::Number => p.number,
            Tok::Comment => p.comment,
            Tok::QuotedIdent => p.qident,
            Tok::Ident | Tok::Space => p.ident,
            Tok::Operator | Tok::Punct => p.op,
        };
        let mut fmt = TextFormat::simple(font.clone(), color);
        if k == Tok::Comment {
            fmt.italics = true;
        }
        job.append(src.get(r).unwrap_or_default(), 0.0, fmt);
    }
    job
}

pub fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}
pub fn byte_to_char(s: &str, bi: usize) -> usize {
    s[..bi.min(s.len())].chars().count()
}

/// Shows the editor filling the available space.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    t: &Tokens,
    id: egui::Id,
    src: &mut String,
    lang: Lang,
    completions: &dyn Fn(&str, Option<&str>) -> Vec<Completion>,
    ac: &mut Autocomplete,
    highlight_range: Option<Range<usize>>,
) -> EditorOut {
    let gutter = 44.0;
    let row_h = ui.ctx().fonts_mut(|f| f.row_height(&FontId::monospace(FONT)));
    // Keys the popup takes before the text edit sees them.
    let mut accept = false;
    if ac.open {
        ui.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                ac.selected = (ac.selected + 1).min(ac.items.len().saturating_sub(1));
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                ac.selected = ac.selected.saturating_sub(1);
            }
            if i.consume_key(Modifiers::NONE, Key::Tab) || i.consume_key(Modifiers::NONE, Key::Enter) {
                accept = true;
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                ac.open = false;
            }
        });
    }
    let mut out = EditorOut { cursor: 0, selection: None, changed: false, focused: false };
    let avail = ui.available_size();
    egui::ScrollArea::vertical().id_salt(id.with("scroll")).auto_shrink([false, false]).show(ui, |ui| {
        let bg_idx = ui.painter().add(egui::Shape::Noop);
        let tok = *t;
        let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| -> Arc<egui::Galley> {
            let mut job = highlight(buf.as_str(), lang, &tok);
            job.wrap.max_width = wrap;
            ui.ctx().fonts_mut(|f| f.layout_job(job))
        };
        let te = egui::TextEdit::multiline(src)
            .id(id)
            .code_editor()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: gutter as i8 + 8, right: 8, top: 6, bottom: 6 }))
            .desired_width(f32::INFINITY)
            .min_size(vec2(avail.x, avail.y))
            .lock_focus(true)
            .layouter(&mut layouter);
        let mut o = te.show(ui);
        out.changed = o.response.changed();
        out.focused = o.response.has_focus();
        let galley = o.galley.clone();
        let gpos = o.galley_pos;
        let text_now: &str = src.as_str();
        // Cursor and selection.
        if let Some(cr) = o.cursor_range {
            let [a, b] = cr.sorted_cursors();
            out.cursor = char_to_byte(text_now, cr.primary.index.0);
            if a.index != b.index {
                out.selection = Some(char_to_byte(text_now, a.index.0)..char_to_byte(text_now, b.index.0));
            }
        }
        // Gutter, current-line and current-statement backgrounds.
        let rect = o.response.rect;
        let painter = ui.painter().clone();
        let gutter_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.min.x + gutter, rect.max.y));
        let mut shapes = vec![egui::Shape::rect_filled(gutter_rect, 0.0, t.panel_alt)];
        shapes.push(egui::Shape::line_segment([gutter_rect.right_top(), gutter_rect.right_bottom()], egui::Stroke::new(1.0, t.border)));
        if let Some(hr) = &highlight_range
            && out.focused
        {
            let a = galley.pos_from_cursor(CCursor::new(byte_to_char(text_now, hr.start)));
            let b = galley.pos_from_cursor(CCursor::new(byte_to_char(text_now, hr.end)));
            let band = egui::Rect::from_min_max(egui::pos2(gutter_rect.right() + 1.0, gpos.y + a.min.y), egui::pos2(rect.right(), gpos.y + b.max.y));
            shapes.push(egui::Shape::rect_filled(band, 0.0, if t.dark { Color32::from_white_alpha(8) } else { Color32::from_rgba_unmultiplied(0x2B, 0x6C, 0xE0, 10) }));
        }
        let cur_line_y = o.cursor_range.map(|cr| galley.pos_from_cursor(cr.primary).center().y + gpos.y);
        let mut line = 1;
        let mut new_line = true;
        for row in &galley.rows {
            let y = gpos.y + row.pos.y;
            if new_line {
                let is_cur = cur_line_y.is_some_and(|cy| cy >= y && cy < y + row.rect().height().max(row_h));
                if is_cur && out.focused {
                    shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_max(egui::pos2(gutter_rect.right() + 1.0, y), egui::pos2(rect.right(), y + row_h)), 0.0, if t.dark { Color32::from_white_alpha(6) } else { Color32::from_black_alpha(6) }));
                }
                let col = if is_cur { t.text } else { t.muted.gamma_multiply(0.8) };
                let g = painter.layout_no_wrap(line.to_string(), FontId::monospace(FONT - 1.5), col);
                shapes.push(egui::Shape::galley(egui::pos2(gutter_rect.right() - 8.0 - g.size().x, y + (row_h - g.size().y) / 2.0), g, col));
                line += 1;
            }
            new_line = row.ends_with_newline;
        }
        painter.set(bg_idx, egui::Shape::Vec(shapes));

        // Autocomplete: open while typing a word.
        if out.changed && out.focused {
            let (start, word) = text::word_before(text_now, out.cursor);
            let qualifier = text::qualifier_before(text_now, start);
            let typed_dot = text_now[..out.cursor].ends_with('.');
            if (!word.is_empty() || typed_dot) && lang == Lang::Sql {
                let items = completions(word, qualifier.as_deref());
                ac.open = !items.is_empty() && !(items.len() == 1 && items[0].text.eq_ignore_ascii_case(word));
                ac.items = items;
                ac.selected = 0;
                ac.word = start..out.cursor;
            } else {
                ac.open = false;
            }
        }
        // Accept a completion.
        let mut clicked: Option<usize> = None;
        if ac.open {
            if let Some(cr) = o.cursor_range {
                let p = galley.pos_from_cursor(cr.primary);
                let at = gpos + vec2(p.min.x, p.max.y + 2.0);
                let area = egui::Area::new(id.with("ac")).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).inner_margin(4.0).show(ui, |ui| {
                        ui.set_min_width(260.0);
                        for (i, it) in ac.items.iter().enumerate().take(12) {
                            let sel = i == ac.selected;
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(260.0), 20.0), egui::Sense::click());
                            if sel || resp.hovered() {
                                ui.painter().rect_filled(r, 3.0, t.accent_soft);
                            }
                            let badge = match it.kind {
                                "table" => ("T", Color32::from_rgb(0x2B, 0x6C, 0xE0)),
                                "column" => ("C", Color32::from_rgb(0x0E, 0x93, 0x84)),
                                "keyword" => ("K", Color32::from_rgb(0x6B, 0x72, 0x80)),
                                "function" => ("f", Color32::from_rgb(0x7C, 0x3A, 0xED)),
                                _ => ("•", t.muted),
                            };
                            let br = egui::Rect::from_min_size(r.min + vec2(4.0, 3.0), vec2(14.0, 14.0));
                            ui.painter().rect_filled(br, 3.0, badge.1.gamma_multiply(0.18));
                            ui.painter().text(br.center(), egui::Align2::CENTER_CENTER, badge.0, FontId::proportional(10.0), badge.1);
                            ui.painter().text(r.min + vec2(24.0, 10.0), egui::Align2::LEFT_CENTER, &it.text, FontId::monospace(12.0), t.text);
                            ui.painter().text(r.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, &it.detail, FontId::proportional(10.5), t.muted);
                            if resp.clicked() {
                                clicked = Some(i);
                            }
                        }
                    });
                });
                if !out.focused && !area.response.contains_pointer() && ui.input(|i| i.pointer.any_click()) && clicked.is_none() {
                    ac.open = false;
                }
            } else {
                ac.open = false;
            }
        }
        if let Some(i) = clicked {
            ac.selected = i;
            accept = true;
        }
        if accept && ac.open {
            if let Some(it) = ac.items.get(ac.selected).cloned() {
                let r = ac.word.clone();
                if r.end <= src.len() && r.start <= r.end {
                    let ins = if it.kind == "table" || it.kind == "column" { quote_if_needed(&it.text) } else { it.text.clone() };
                    src.replace_range(r.clone(), &ins);
                    let new_cursor = byte_to_char(src, r.start + ins.len());
                    o.state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(new_cursor))));
                    o.state.clone().store(ui.ctx(), id);
                    out.cursor = r.start + ins.len();
                    out.changed = true;
                    ui.ctx().memory_mut(|m| m.request_focus(id));
                }
            }
            ac.open = false;
        }
    });
    out
}

/// Identifiers that aren't plain words get double quotes.
pub fn quote_if_needed(name: &str) -> String {
    let plain = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !text::is_keyword(name);
    if plain { name.to_string() } else { vizardcraft_sql::quote_ident(name) }
}

/// Inserts text at a byte position, returning the new cursor.
pub fn insert_at(src: &mut String, at: usize, s: &str) -> usize {
    let at = at.min(src.len());
    let at = (0..=at).rev().find(|i| src.is_char_boundary(*i)).unwrap_or(0);
    src.insert_str(at, s);
    at + s.len()
}

/// Sets the editor's cursor (byte offset) and focuses it.
pub fn set_cursor(ctx: &egui::Context, id: egui::Id, src: &str, at: usize) {
    let mut st = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(byte_to_char(src, at)))));
    st.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}
