//! Icons of the Power Query Editor, the Get Data gallery and the data source canvas, drawn in
//! code on a 16×16 grid (original artwork).

use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cat {
    Blue,
    Green,
    Orange,
    Red,
    Purple,
    Teal,
    Amber,
    Slate,
}

struct Palette {
    paper: Color32,
    dark: bool,
}

impl Palette {
    fn cat(&self, k: Cat) -> Color32 {
        let (l, d) = match k {
            Cat::Blue => (0x2563EB, 0x60A5FA),
            Cat::Green => (0x16A34A, 0x4ADE80),
            Cat::Orange => (0xEA580C, 0xFB923C),
            Cat::Red => (0xDC2626, 0xF87171),
            Cat::Purple => (0x7C3AED, 0xA78BFA),
            Cat::Teal => (0x0D9488, 0x2DD4BF),
            Cat::Amber => (0xD97706, 0xFBBF24),
            Cat::Slate => (0x64748B, 0x94A3B8),
        };
        let h = if self.dark { d } else { l };
        Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
    fn wash(&self, k: Cat) -> Color32 {
        let c = self.cat(k);
        if self.dark { c.gamma_multiply(0.35) } else { Color32::from_rgb(255 - (255 - c.r()) / 5, 255 - (255 - c.g()) / 5, 255 - (255 - c.b()) / 5) }
    }
}

struct G<'a> {
    p: &'a Painter,
    r: Rect,
    s: f32,
}

impl G<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.left() + x * self.s, self.r.top() + y * self.s)
    }
    fn line(&self, pts: &[(f32, f32)], c: Color32, w: f32) {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        self.p.add(PathShape::line(v, PathStroke::new(w * self.s.clamp(1.0, 2.0), c)));
    }
    fn stroke(&self, pts: &[(f32, f32)], c: Color32) {
        self.line(pts, c, 1.15);
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, fill: Option<Color32>, stroke: Option<Color32>) {
        let r = Rect::from_min_size(self.pt(x, y), vec2(w * self.s, h * self.s));
        if let Some(f) = fill {
            self.p.rect_filled(r, 1.0, f);
        }
        if let Some(c) = stroke {
            self.p.rect_stroke(r, 1.0, Stroke::new(1.0, c), egui::StrokeKind::Inside);
        }
    }
    fn poly(&self, pts: &[(f32, f32)], fill: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        self.p.add(PathShape::convex_polygon(v, fill, Stroke::NONE));
    }
    fn circle(&self, x: f32, y: f32, r: f32, fill: Option<Color32>, stroke: Option<Color32>) {
        if let Some(f) = fill {
            self.p.circle_filled(self.pt(x, y), r * self.s, f);
        }
        if let Some(c) = stroke {
            self.p.circle_stroke(self.pt(x, y), r * self.s, Stroke::new(1.0, c));
        }
    }
    fn text(&self, x: f32, y: f32, t: &str, size: f32, c: Color32, _style: (bool, bool)) {
        let font = egui::FontId::proportional(size * self.s);
        self.p.text(self.pt(x, y), egui::Align2::CENTER_CENTER, t, font, c);
    }
}

/// Draws the named icon in `rect` (square). Unknown names fall back to the app's icon set.
pub fn paint(p: &Painter, rect: Rect, name: &str, c: Color32, dark: bool) {
    let side = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(side, side));
    let g = G { p, r, s: side / 16.0 };
    let pal = Palette { paper: if dark { Color32::from_rgb(0x23, 0x28, 0x33) } else { Color32::WHITE }, dark };
    let light = c.gamma_multiply(0.45);
    if !draw(&g, name, c, light, &pal) {
        crate::icons::paint(p, rect, name, c);
    }
}

fn table(g: &G<'_>, (x, y, w, h): (f32, f32, f32, f32), c: Color32, head: Color32, light: Color32) {
    g.rect(x, y, w, 3.0, Some(head), None);
    g.rect(x, y, w, h, None, Some(c));
    g.stroke(&[(x + w / 2.0, y), (x + w / 2.0, y + h)], light);
    let mut ry = y + 3.0 + (h - 3.0) / 3.0;
    while ry < y + h - 0.5 {
        g.stroke(&[(x, ry), (x + w, ry)], light);
        ry += (h - 3.0) / 3.0;
    }
}

/// A circular arrow centred at (cx, cy), radius r, open at the top right, head at its end.
fn circular_arrow(g: &G<'_>, cx: f32, cy: f32, r: f32, c: Color32) {
    let pts: Vec<(f32, f32)> = (0..=16)
        .map(|i| {
            let a = (-50.0 + i as f32 * 17.0_f32).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    g.line(&pts, c, 1.4);
    let a = (-50.0_f32).to_radians();
    let (ex, ey) = (cx + r * a.cos(), cy + r * a.sin());
    g.poly(&[(ex - 2.6, ey - 1.0), (ex + 1.6, ey - 2.4), (ex + 0.6, ey + 2.0)], c);
}

fn arrow_down(g: &G<'_>, x: f32, y0: f32, y1: f32, c: Color32) {
    g.line(&[(x, y0), (x, y1 - 1.5)], c, 1.3);
    g.poly(&[(x - 2.0, y1 - 2.5), (x, y1), (x + 2.0, y1 - 2.5)], c);
}

fn sparkle(g: &G<'_>, x: f32, y: f32, s: f32, c: Color32) {
    g.poly(
        &[
            (x, y - s),
            (x + s * 0.3, y - s * 0.3),
            (x + s, y),
            (x + s * 0.3, y + s * 0.3),
            (x, y + s),
            (x - s * 0.3, y + s * 0.3),
            (x - s, y),
            (x - s * 0.3, y - s * 0.3),
        ],
        c,
    );
}

fn cross(g: &G<'_>, x: f32, y: f32, s: f32, c: Color32) {
    g.line(&[(x - s, y - s), (x + s, y + s)], c, 1.5);
    g.line(&[(x - s, y + s), (x + s, y - s)], c, 1.5);
}

fn draw(g: &G<'_>, icon: &str, c: Color32, light: Color32, pal: &Palette) -> bool {
    let blue = pal.cat(Cat::Blue);
    let green = pal.cat(Cat::Green);
    let red = pal.cat(Cat::Red);
    let orange = pal.cat(Cat::Orange);
    let purple = pal.cat(Cat::Purple);
    let amber = pal.cat(Cat::Amber);
    let teal = pal.cat(Cat::Teal);
    let wash = |k: Cat| pal.wash(k);
    let _ = (teal, purple);
    let head = wash(Cat::Blue);
    match icon {
        "new-source" => {
            for y in [2.5, 6.5, 10.5] {
                g.rect(1.5, y, 9.0, 3.2, Some(wash(Cat::Slate)), Some(c));
            }
            g.line(&[(13.0, 2.0), (13.0, 8.0)], green, 1.6);
            g.line(&[(10.0, 5.0), (16.0, 5.0)], green, 1.6);
        }
        "enter-data" => {
            table(g, (1.0, 1.5, 11.0, 11.0), c, head, light);
            g.line(&[(8.5, 14.5), (14.5, 8.5)], orange, 2.0);
            g.poly(&[(7.5, 15.5), (8.0, 13.5), (9.5, 15.0)], c);
        }
        "refresh" => circular_arrow(g, 8.0, 8.5, 5.5, green),
        "refresh-all" => {
            table(g, (1.0, 1.0, 8.0, 8.0), c, head, light);
            g.circle(10.5, 10.5, 5.0, Some(pal.paper), None);
            circular_arrow(g, 10.5, 10.5, 3.8, green);
        }
        "advanced-editor" => {
            g.rect(2.5, 1.5, 11.0, 13.0, Some(pal.paper), Some(c));
            g.text(8.0, 8.0, "</>", 5.5, blue, (true, false));
        }
        "manage" => {
            table(g, (1.0, 1.0, 10.0, 10.0), c, head, light);
            g.circle(12.0, 12.0, 3.2, Some(pal.paper), Some(c));
            g.circle(12.0, 12.0, 1.2, Some(c), None);
            for (dx, dy) in [(0.0, -4.0), (0.0, 4.0), (-4.0, 0.0), (4.0, 0.0)] {
                g.line(&[(12.0 + dx * 0.8, 12.0 + dy * 0.8), (12.0 + dx, 12.0 + dy)], c, 1.6);
            }
        }
        "choose-columns" => {
            g.rect(5.5, 1.5, 4.5, 13.0, Some(wash(Cat::Green)), None);
            g.rect(1.0, 1.5, 14.0, 13.0, None, Some(c));
            g.stroke(&[(5.5, 1.5), (5.5, 14.5)], light);
            g.stroke(&[(10.0, 1.5), (10.0, 14.5)], light);
            g.line(&[(6.2, 8.0), (7.5, 9.6), (9.6, 6.0)], green, 1.5);
        }
        "remove-columns" => {
            g.rect(1.0, 1.5, 14.0, 13.0, None, Some(c));
            g.stroke(&[(5.5, 1.5), (5.5, 14.5)], light);
            g.stroke(&[(10.0, 1.5), (10.0, 14.5)], light);
            g.rect(10.0, 1.5, 5.0, 13.0, Some(wash(Cat::Red)), None);
            cross(g, 12.5, 8.0, 1.8, red);
        }
        "keep-rows" => {
            g.rect(1.0, 5.5, 14.0, 5.0, Some(wash(Cat::Green)), None);
            g.rect(1.0, 1.5, 14.0, 13.0, None, Some(c));
            for y in [5.5, 10.5] {
                g.stroke(&[(1.0, y), (15.0, y)], light);
            }
            g.line(&[(5.5, 8.0), (7.0, 9.5), (10.0, 6.5)], green, 1.5);
        }
        "remove-rows" => {
            g.rect(1.0, 1.5, 14.0, 13.0, None, Some(c));
            for y in [5.5, 10.5] {
                g.stroke(&[(1.0, y), (15.0, y)], light);
            }
            g.rect(1.0, 10.5, 14.0, 4.0, Some(wash(Cat::Red)), None);
            g.line(&[(5.0, 12.5), (11.0, 12.5)], red, 1.8);
        }
        "split-column" => {
            g.rect(1.5, 2.0, 5.0, 12.0, None, Some(c));
            g.rect(9.5, 2.0, 5.0, 12.0, None, Some(c));
            g.line(&[(8.0, 1.0), (8.0, 15.0)], red, 1.0);
            g.poly(&[(6.0, 6.0), (4.0, 8.0), (6.0, 10.0)], blue);
            g.poly(&[(10.0, 6.0), (12.0, 8.0), (10.0, 10.0)], blue);
        }
        "group-by" => {
            for y in [3.0, 6.0] {
                g.stroke(&[(1.5, y), (6.0, y)], c);
            }
            for y in [10.0, 13.0] {
                g.stroke(&[(1.5, y), (6.0, y)], c);
            }
            g.stroke(&[(7.5, 2.5), (8.5, 2.5), (8.5, 6.5), (7.5, 6.5)], blue);
            g.stroke(&[(7.5, 9.5), (8.5, 9.5), (8.5, 13.5), (7.5, 13.5)], blue);
            for y in [2.5, 9.5] {
                g.line(&[(14.5, y), (10.5, y), (12.8, y + 2.0), (10.5, y + 4.0), (14.5, y + 4.0)], blue, 1.2);
            }
        }
        "data-type" => {
            g.text(8.0, 4.8, "ABC", 6.0, c, (true, false));
            g.text(8.0, 11.5, "123", 6.0, blue, (true, false));
        }
        "headers" => {
            g.rect(1.0, 4.0, 14.0, 4.0, Some(wash(Cat::Blue)), None);
            g.rect(1.0, 4.0, 14.0, 11.0, None, Some(c));
            g.stroke(&[(8.0, 4.0), (8.0, 15.0)], light);
            g.stroke(&[(1.0, 11.5), (15.0, 11.5)], light);
            g.line(&[(8.0, 7.5), (8.0, 1.5)], blue, 1.4);
            g.poly(&[(6.0, 3.0), (8.0, 0.5), (10.0, 3.0)], blue);
        }
        "replace" => {
            g.text(4.0, 4.5, "a", 8.0, c, (true, false));
            g.text(12.0, 11.5, "b", 8.0, blue, (true, false));
            g.line(&[(4.0, 8.5), (4.0, 12.0), (8.0, 12.0)], orange, 1.3);
            g.poly(&[(7.5, 10.0), (10.0, 12.0), (7.5, 14.0)], orange);
        }
        "merge-queries" => {
            g.circle(5.8, 8.0, 4.6, Some(wash(Cat::Blue)), Some(blue));
            g.circle(10.2, 8.0, 4.6, Some(wash(Cat::Green)), Some(green));
        }
        "append-queries" => {
            table(g, (2.0, 0.8, 12.0, 6.0), c, head, light);
            table(g, (2.0, 9.2, 12.0, 6.0), c, wash(Cat::Green), light);
            g.line(&[(8.0, 6.8), (8.0, 9.2)], green, 1.6);
        }
        "combine-files" => {
            g.rect(5.0, 1.0, 9.0, 10.0, Some(pal.paper), Some(light));
            g.rect(3.5, 2.5, 9.0, 10.0, Some(pal.paper), Some(light));
            g.rect(2.0, 4.0, 9.0, 10.0, Some(pal.paper), Some(c));
            arrow_down(g, 6.5, 6.0, 12.5, green);
        }
        "parameters" => {
            for (y, k) in [(3.5, 10.0), (8.0, 5.0), (12.5, 8.5)] {
                g.stroke(&[(1.5, y), (14.5, y)], c);
                g.circle(k, y, 1.8, Some(blue), None);
            }
        }
        "close-load" => {
            table(g, (1.0, 5.0, 10.0, 10.0), c, wash(Cat::Green), light);
            g.line(&[(14.5, 1.5), (14.5, 5.0), (9.0, 5.0)], green, 1.5);
            g.poly(&[(10.0, 2.8), (7.2, 5.0), (10.0, 7.2)], green);
        }
        "transpose" => {
            table(g, (1.0, 1.0, 8.0, 8.0), c, head, light);
            g.rect(11.0, 7.0, 4.0, 8.0, None, Some(c));
            g.rect(7.0, 11.0, 8.0, 4.0, None, Some(c));
            g.line(&[(4.0, 10.5), (4.0, 13.0), (5.5, 13.0)], blue, 1.3);
            g.poly(&[(5.0, 11.2), (7.0, 13.0), (5.0, 14.8)], blue);
        }
        "reverse-rows" => {
            arrow_down(g, 5.0, 2.0, 14.5, c);
            g.line(&[(11.0, 14.0), (11.0, 3.5)], blue, 1.3);
            g.poly(&[(9.0, 4.5), (11.0, 1.5), (13.0, 4.5)], blue);
        }
        "count-rows" => {
            for y in [3.0, 6.0, 9.0, 12.0] {
                g.stroke(&[(1.5, y), (7.0, y)], light);
            }
            g.text(11.5, 8.0, "#", 11.0, blue, (true, false));
        }
        "detect-type" => {
            g.text(7.0, 10.0, "ABC", 6.0, c, (true, false));
            sparkle(g, 12.5, 4.0, 3.0, amber);
        }
        "rename" => {
            g.rect(1.0, 4.0, 14.0, 8.0, Some(pal.paper), Some(c));
            g.text(6.0, 8.0, "ab", 7.0, c, (false, false));
            g.line(&[(11.0, 2.5), (11.0, 13.5)], blue, 1.3);
        }
        "pivot-column" => {
            g.rect(1.0, 4.0, 4.0, 11.0, Some(wash(Cat::Blue)), Some(c));
            g.rect(7.0, 1.0, 8.0, 4.0, Some(wash(Cat::Green)), Some(c));
            g.line(&[(3.0, 3.0), (3.0, 1.5), (5.5, 1.5)], blue, 1.3);
            g.poly(&[(5.0, 0.0), (7.0, 1.5), (5.0, 3.0)], blue);
            g.rect(7.0, 7.0, 8.0, 8.0, None, Some(light));
        }
        "unpivot" => {
            g.rect(1.0, 1.0, 8.0, 4.0, Some(wash(Cat::Green)), Some(c));
            g.rect(11.0, 7.0, 4.0, 8.0, Some(wash(Cat::Blue)), Some(c));
            g.line(&[(10.5, 3.0), (13.0, 3.0), (13.0, 5.0)], blue, 1.3);
            g.poly(&[(11.5, 4.5), (13.0, 7.0), (14.5, 4.5)], blue);
            g.rect(1.0, 7.0, 8.0, 8.0, None, Some(light));
        }
        "move-columns" => {
            g.rect(1.0, 3.0, 5.0, 11.0, None, Some(c));
            g.rect(10.0, 3.0, 5.0, 11.0, Some(wash(Cat::Blue)), Some(c));
            g.line(&[(4.0, 1.5), (12.5, 1.5)], blue, 1.2);
            g.poly(&[(11.5, 0.0), (13.5, 1.5), (11.5, 3.0)], blue);
        }
        "to-list" => {
            for y in [3.5, 8.0, 12.5] {
                g.circle(3.0, y, 1.3, Some(blue), None);
                g.stroke(&[(6.0, y), (14.5, y)], c);
            }
        }
        "format" => g.text(8.0, 8.5, "Aa", 10.0, c, (false, false)),
        "extract" => {
            g.text(8.0, 7.0, "ABC", 6.5, light, (true, false));
            g.text(8.0, 7.0, " B ", 6.5, blue, (true, false));
            g.stroke(&[(5.5, 11.0), (5.5, 13.0), (10.5, 13.0), (10.5, 11.0)], blue);
        }
        "parse" => g.text(8.0, 8.0, "{ }", 9.0, purple, (true, false)),
        "standard" => {
            g.text(4.5, 4.5, "+", 7.0, c, (true, false));
            g.text(11.5, 4.5, "-", 7.0, c, (true, false));
            g.text(4.5, 11.5, "x", 7.0, blue, (true, false));
            g.text(11.5, 11.5, "/", 7.0, blue, (true, false));
        }
        "scientific" => {
            g.text(6.5, 9.5, "x", 11.0, c, (false, true));
            g.text(12.5, 4.5, "2", 6.0, blue, (true, false));
        }
        "duration" => {
            g.stroke(&[(3.0, 1.5), (13.0, 1.5)], c);
            g.stroke(&[(3.0, 14.5), (13.0, 14.5)], c);
            g.stroke(&[(4.0, 1.5), (12.0, 14.5)], c);
            g.stroke(&[(12.0, 1.5), (4.0, 14.5)], c);
            g.poly(&[(5.5, 13.5), (10.5, 13.5), (8.0, 10.0)], amber);
        }
        "expand-column" => {
            table(g, (1.0, 1.0, 9.0, 9.0), c, head, light);
            g.line(&[(9.5, 9.5), (14.0, 14.0)], blue, 1.4);
            g.poly(&[(15.0, 15.0), (10.5, 14.6), (14.6, 10.5)], blue);
        }
        "from-examples" => {
            table(g, (1.0, 3.0, 11.0, 12.0), c, head, light);
            sparkle(g, 12.5, 3.5, 3.2, amber);
        }
        "custom-column" => {
            table(g, (1.0, 1.0, 9.0, 13.0), c, head, light);
            g.rect(8.0, 7.0, 7.5, 7.5, Some(pal.paper), Some(c));
            g.text(11.8, 10.7, "fx", 6.0, blue, (false, true));
        }
        "conditional" => {
            g.line(&[(1.5, 8.0), (6.0, 8.0)], c, 1.3);
            g.line(&[(6.0, 8.0), (10.5, 3.5), (13.0, 3.5)], green, 1.3);
            g.line(&[(6.0, 8.0), (10.5, 12.5), (13.0, 12.5)], red, 1.3);
            g.circle(6.0, 8.0, 1.6, Some(c), None);
            g.circle(14.0, 3.5, 1.4, Some(green), None);
            g.circle(14.0, 12.5, 1.4, Some(red), None);
        }
        "index-column" => {
            g.rect(3.0, 1.0, 10.0, 14.0, None, Some(c));
            for (i, y) in [4.0f32, 8.0, 12.0].iter().enumerate() {
                g.text(8.0, *y, ["1", "2", "3"].get(i).copied().unwrap_or(""), 5.5, blue, (true, false));
            }
        }
        "dependencies" => {
            g.stroke(&[(4.0, 4.0), (12.0, 4.0)], light);
            g.stroke(&[(4.0, 4.0), (8.0, 12.0)], light);
            g.stroke(&[(12.0, 4.0), (8.0, 12.0)], light);
            g.rect(1.0, 2.0, 5.0, 4.0, Some(wash(Cat::Blue)), Some(c));
            g.rect(10.0, 2.0, 5.0, 4.0, Some(wash(Cat::Green)), Some(c));
            g.rect(5.5, 10.0, 5.0, 4.0, Some(wash(Cat::Orange)), Some(c));
        }
        "go-to-column" => {
            g.rect(1.0, 5.0, 14.0, 10.0, None, Some(c));
            g.rect(6.0, 5.0, 4.0, 10.0, Some(wash(Cat::Blue)), None);
            g.stroke(&[(6.0, 5.0), (6.0, 15.0)], light);
            g.stroke(&[(10.0, 5.0), (10.0, 15.0)], light);
            arrow_down(g, 8.0, 0.5, 5.0, blue);
        }
        "monospace" => {
            g.rect(1.0, 2.5, 14.0, 11.0, None, Some(light));
            g.text(8.0, 8.0, "Mi", 8.0, c, (true, false));
            for x in [4.5, 11.5] {
                g.stroke(&[(x - 2.5, 12.0), (x + 2.5, 12.0)], blue);
            }
        }
        "whitespace" => {
            g.text(3.5, 7.5, "a", 7.0, c, (false, false));
            g.text(12.5, 7.5, "b", 7.0, c, (false, false));
            g.circle(8.0, 8.5, 1.1, Some(blue), None);
            g.stroke(&[(1.5, 13.0), (14.5, 13.0)], light);
        }
        "quality" => {
            g.rect(1.0, 6.0, 9.5, 4.0, Some(green), None);
            g.rect(10.5, 6.0, 2.5, 4.0, Some(red), None);
            g.rect(13.0, 6.0, 2.0, 4.0, Some(light), None);
            g.text(5.0, 2.8, "OK", 4.5, c, (true, false));
        }
        "profile" => {
            g.rect(1.0, 1.0, 14.0, 14.0, None, Some(c));
            for (x, h) in [(3.0, 6.0), (6.0, 9.0), (9.0, 4.0), (12.0, 7.0)] {
                g.rect(x, 13.5 - h, 2.0, h, Some(teal), None);
            }
        }
        "all-rows" => {
            for y in [2.5, 5.5, 8.5, 11.5, 14.0] {
                g.stroke(&[(1.5, y), (10.0, y)], c);
            }
            arrow_down(g, 13.0, 1.5, 15.0, blue);
        }

        "sort-asc" => {
            g.text(4.5, 4.0, "A", 6.5, c, (true, false));
            g.text(4.5, 12.0, "Z", 6.5, c, (true, false));
            arrow_down(g, 12.0, 2.0, 14.5, blue);
        }
        "sort-desc" => {
            g.text(4.5, 4.0, "Z", 6.5, c, (true, false));
            g.text(4.5, 12.0, "A", 6.5, c, (true, false));
            arrow_down(g, 12.0, 2.0, 14.5, blue);
        }
        "fill" => {
            g.rect(4.0, 1.0, 8.0, 14.0, None, Some(c));
            g.rect(4.0, 1.0, 8.0, 4.0, Some(wash(Cat::Blue)), None);
            arrow_down(g, 8.0, 5.5, 14.0, blue);
        }
        "merge-columns" => {
            g.rect(1.0, 2.0, 5.0, 12.0, None, Some(c));
            g.rect(10.0, 2.0, 5.0, 12.0, None, Some(c));
            g.poly(&[(6.5, 6.0), (8.5, 8.0), (6.5, 10.0)], blue);
            g.poly(&[(9.5, 6.0), (7.5, 8.0), (9.5, 10.0)], blue);
        }
        "statistics" => g.text(8.0, 8.0, "\u{2211}", 13.0, blue, (true, false)),
        "rounding" => g.text(8.0, 8.0, ".00", 8.0, c, (true, false)),
        "information" => {
            g.circle(8.0, 8.0, 6.5, None, Some(blue));
            g.text(8.0, 8.5, "i", 9.0, blue, (true, false));
        }
        "date" => {
            g.rect(1.5, 2.5, 13.0, 12.0, Some(pal.paper), Some(c));
            g.rect(1.5, 2.5, 13.0, 3.5, Some(red), None);
            for (x, y) in [(4.5, 9.0), (8.0, 9.0), (11.5, 9.0), (4.5, 12.0), (8.0, 12.0)] {
                g.rect(x - 1.0, y - 1.0, 2.0, 2.0, Some(light), None);
            }
        }
        "time" => {
            g.circle(8.0, 8.0, 6.5, Some(pal.paper), Some(c));
            g.line(&[(8.0, 8.0), (8.0, 4.0)], c, 1.3);
            g.line(&[(8.0, 8.0), (11.0, 9.5)], blue, 1.3);
        }
        "duplicate-column" => {
            g.rect(1.0, 1.0, 6.0, 12.0, None, Some(c));
            g.rect(8.0, 3.0, 6.0, 12.0, Some(wash(Cat::Blue)), Some(c));
        }
        "invoke-function" => {
            g.rect(1.5, 2.0, 13.0, 12.0, Some(pal.paper), Some(c));
            g.text(8.0, 8.0, "f()", 7.0, purple, (false, true));
        }
        "python" => {
            g.rect(1.5, 1.5, 6.5, 6.5, Some(blue), None);
            g.rect(8.0, 8.0, 6.5, 6.5, Some(amber), None);
            g.text(8.0, 8.0, "Py", 7.0, c, (true, false));
        }
        "close-apply" => {
            table(g, (1.0, 5.0, 10.0, 10.0), c, wash(Cat::Amber), light);
            g.rect(8.0, 0.5, 7.5, 6.5, Some(amber), None);
            g.line(&[(9.5, 3.8), (11.2, 5.4), (14.3, 2.0)], pal.paper, 1.4);
        }
        "close" => {
            cross(g, 8.0, 8.0, 5.0, c);
        }
        "recent" => {
            g.circle(8.0, 8.0, 6.3, Some(pal.paper), Some(c));
            g.line(&[(8.0, 4.0), (8.0, 8.0), (11.0, 10.0)], blue, 1.4);
        }
        "data-source" => {
            g.rect(1.0, 2.0, 6.0, 5.0, Some(wash(Cat::Blue)), Some(c));
            g.rect(9.0, 2.0, 6.0, 5.0, Some(wash(Cat::Green)), Some(c));
            g.line(&[(7.0, 4.5), (9.0, 4.5)], c, 1.2);
            g.rect(1.0, 10.0, 6.0, 5.0, Some(wash(Cat::Orange)), Some(c));
            g.line(&[(4.0, 7.0), (4.0, 10.0)], c, 1.2);
        }
        "database" => {
            g.rect(2.5, 3.0, 11.0, 10.0, Some(wash(Cat::Blue)), None);
            g.circle(8.0, 3.0, 0.1, None, None);
            for y in [3.0, 13.0] {
                g.line(&[(2.5, y), (13.5, y)], c, 1.0);
            }
            g.line(&[(2.5, 3.0), (2.5, 13.0)], c, 1.0);
            g.line(&[(13.5, 3.0), (13.5, 13.0)], c, 1.0);
            g.line(&[(2.5, 6.5), (13.5, 6.5)], light, 1.0);
            g.line(&[(2.5, 10.0), (13.5, 10.0)], light, 1.0);
        }
        "file-csv" => {
            g.rect(2.5, 1.0, 11.0, 14.0, Some(pal.paper), Some(c));
            g.text(8.0, 9.0, "CSV", 5.0, green, (true, false));
        }
        "file-excel" => {
            g.rect(2.5, 1.0, 11.0, 14.0, Some(pal.paper), Some(c));
            g.rect(2.5, 5.0, 8.0, 7.0, Some(green), None);
            g.text(6.5, 8.5, "X", 6.5, pal.paper, (true, false));
        }
        "file-json" => {
            g.rect(2.5, 1.0, 11.0, 14.0, Some(pal.paper), Some(c));
            g.text(8.0, 8.5, "{ }", 7.0, amber, (true, false));
        }
        "file-xml" => {
            g.rect(2.5, 1.0, 11.0, 14.0, Some(pal.paper), Some(c));
            g.text(8.0, 8.5, "</>", 5.5, orange, (true, false));
        }
        "file-parquet" => {
            g.rect(2.5, 1.0, 11.0, 14.0, Some(pal.paper), Some(c));
            for (i, y) in [4.0f32, 7.0, 10.0].iter().enumerate() {
                g.rect(4.5, *y, 7.0, 2.0, Some(if i % 2 == 0 { teal } else { blue }), None);
            }
        }
        "folder" => {
            g.poly(&[(1.0, 4.0), (6.0, 4.0), (7.5, 5.5), (15.0, 5.5), (15.0, 13.5), (1.0, 13.5)], amber);
            g.rect(1.0, 6.5, 14.0, 7.0, None, Some(c));
        }
        "web" => {
            g.circle(8.0, 8.0, 6.5, Some(pal.paper), Some(blue));
            g.line(&[(1.5, 8.0), (14.5, 8.0)], blue, 1.0);
            g.line(&[(8.0, 1.5), (8.0, 14.5)], blue, 1.0);
            g.circle(8.0, 8.0, 3.0, None, Some(blue));
        }
        "odata" => {
            g.circle(5.0, 8.0, 3.8, None, Some(orange));
            g.rect(10.0, 4.0, 4.5, 8.0, Some(orange), None);
        }
        "sqlite" | "postgresql" | "mysql" | "sqlserver" | "odbc" | "oracle" => {
            let col = match icon {
                "sqlite" => teal,
                "postgresql" => blue,
                "mysql" => orange,
                "sqlserver" => red,
                "oracle" => red,
                _ => c,
            };
            g.rect(2.5, 3.0, 11.0, 10.0, Some(pal.paper), None);
            g.line(&[(2.5, 3.0), (2.5, 13.0)], col, 1.3);
            g.line(&[(13.5, 3.0), (13.5, 13.0)], col, 1.3);
            for y in [3.0, 6.5, 10.0, 13.0] {
                g.line(&[(2.5, y), (13.5, y)], col, if y == 3.0 || y == 13.0 { 1.3 } else { 0.9 });
            }
        }
        "blank" => {
            g.rect(2.5, 1.0, 11.0, 14.0, Some(pal.paper), Some(c));
        }
        "more" => {
            for x in [3.5, 8.0, 12.5] {
                g.circle(x, 8.0, 1.5, Some(c), None);
            }
        }
        "query" => {
            table(g, (1.0, 2.0, 14.0, 12.0), c, head, light);
        }
        "parameter" => {
            g.rect(1.5, 3.0, 13.0, 10.0, Some(pal.paper), Some(c));
            g.text(8.0, 8.0, "P", 7.5, purple, (true, false));
        }
        "group-folder" => {
            g.poly(&[(1.0, 4.0), (6.0, 4.0), (7.5, 5.5), (15.0, 5.5), (15.0, 13.5), (1.0, 13.5)], wash(Cat::Amber));
            g.rect(1.0, 5.5, 14.0, 8.0, None, Some(amber));
        }
        "gear" => {
            g.circle(8.0, 8.0, 4.5, None, Some(c));
            g.circle(8.0, 8.0, 1.6, Some(c), None);
            for (dx, dy) in [(0.0f32, -1.0f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0), (0.7, 0.7), (-0.7, 0.7), (0.7, -0.7), (-0.7, -0.7)] {
                g.line(&[(8.0 + dx * 4.5, 8.0 + dy * 4.5), (8.0 + dx * 6.5, 8.0 + dy * 6.5)], c, 1.6);
            }
        }
        "join" => {
            g.circle(5.8, 8.0, 4.6, Some(wash(Cat::Blue)), Some(blue));
            g.circle(10.2, 8.0, 4.6, Some(wash(Cat::Blue)), Some(blue));
        }
        "union" => {
            table(g, (2.0, 0.8, 12.0, 6.0), c, head, light);
            table(g, (2.0, 9.2, 12.0, 6.0), c, head, light);
        }
        "help" => {
            g.circle(8.0, 8.0, 6.5, None, Some(blue));
            g.text(8.0, 8.5, "?", 9.0, blue, (true, false));
        }
        "settings" => {
            g.circle(8.0, 8.0, 4.0, None, Some(c));
            g.circle(8.0, 8.0, 1.5, Some(c), None);
        }
        _ => return false,
    }
    true
}
