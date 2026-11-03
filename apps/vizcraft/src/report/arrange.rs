//! Arrange: group/ungroup, z-order, align and distribute (Figma-style).

use vizcraft_core::doc::*;

use crate::app::VizApp;

pub fn group(app: &mut VizApp) {
    if app.selected.len() < 2 {
        return;
    }
    app.checkpoint("Group");
    let id = app.project.next_id();
    let sel = app.selected.clone();
    let Some(pg) = app.current_page_mut() else { return };
    let mut g = Element { id, name: "Group".into(), kind: ElementKind::Group, ..Element::default() };
    g.style.padding = 0.0;
    let insert_at = pg.elements.iter().position(|e| sel.contains(&e.id)).unwrap_or(pg.elements.len());
    for e in pg.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
        e.parent = Some(id);
    }
    pg.elements.insert(insert_at, g);
    super::canvas::sync_groups(pg);
    app.selected = vec![id];
}

pub fn ungroup(app: &mut VizApp) {
    let sel = app.selected.clone();
    let groups: Vec<u64> = sel.iter().copied().filter(|id| app.project.element(*id).is_some_and(|(_, e)| e.kind == ElementKind::Group)).collect();
    if groups.is_empty() {
        return;
    }
    app.checkpoint("Ungroup");
    let Some(pg) = app.current_page_mut() else { return };
    let mut kids = Vec::new();
    for g in &groups {
        let parent = pg.element(*g).and_then(|e| e.parent);
        for e in pg.elements.iter_mut().filter(|e| e.parent == Some(*g)) {
            e.parent = parent;
            kids.push(e.id);
        }
    }
    pg.elements.retain(|e| !groups.contains(&e.id));
    app.selected = kids;
}

/// Ids of the selection including group members, in page order.
fn expanded(app: &VizApp) -> Vec<u64> {
    let Some(pg) = app.current_page() else { return vec![] };
    let mut out: Vec<u64> = app.selected.clone();
    let mut i = 0;
    while i < out.len() {
        let cur = out[i];
        for e in pg.elements.iter().filter(|e| e.parent == Some(cur)) {
            if !out.contains(&e.id) {
                out.push(e.id);
            }
        }
        i += 1;
    }
    out
}

pub fn order(app: &mut VizApp, op: &str) {
    let ids = expanded(app);
    if ids.is_empty() {
        return;
    }
    app.checkpoint("Order");
    let Some(pg) = app.current_page_mut() else { return };
    match op {
        "front" | "back" => {
            let (moving, rest): (Vec<Element>, Vec<Element>) = pg.elements.drain(..).partition(|e| ids.contains(&e.id));
            pg.elements = if op == "front" { rest.into_iter().chain(moving).collect() } else { moving.into_iter().chain(rest).collect() };
        }
        "forward" => {
            let n = pg.elements.len();
            for i in (0..n.saturating_sub(1)).rev() {
                let a = pg.elements.get(i).map(|e| ids.contains(&e.id)).unwrap_or(false);
                let b = pg.elements.get(i + 1).map(|e| ids.contains(&e.id)).unwrap_or(true);
                if a && !b {
                    pg.elements.swap(i, i + 1);
                }
            }
        }
        _ => {
            let n = pg.elements.len();
            for i in 1..n {
                let a = pg.elements.get(i).map(|e| ids.contains(&e.id)).unwrap_or(false);
                let b = pg.elements.get(i - 1).map(|e| ids.contains(&e.id)).unwrap_or(true);
                if a && !b {
                    pg.elements.swap(i, i - 1);
                }
            }
        }
    }
}

pub fn align(app: &mut VizApp, op: &str) {
    let sel = app.selected.clone();
    let Some(pg) = app.current_page() else { return };
    let page_w = pg.width;
    let page_h = pg.height;
    let boxes: Vec<(u64, f32, f32, f32, f32)> = pg.elements.iter().filter(|e| sel.contains(&e.id)).map(|e| (e.id, e.x, e.y, e.w, e.h)).collect();
    if boxes.is_empty() {
        return;
    }
    app.checkpoint("Align");
    // One element aligns to the page; several align to their bounding box.
    let (x0, y0, x1, y1) = if boxes.len() == 1 {
        (0.0, 0.0, page_w, page_h)
    } else {
        (
            boxes.iter().map(|b| b.1).fold(f32::MAX, f32::min),
            boxes.iter().map(|b| b.2).fold(f32::MAX, f32::min),
            boxes.iter().map(|b| b.1 + b.3).fold(f32::MIN, f32::max),
            boxes.iter().map(|b| b.2 + b.4).fold(f32::MIN, f32::max),
        )
    };
    let mut targets: Vec<(u64, f32, f32)> = Vec::new();
    match op {
        "dist-h" | "dist-v" => {
            let mut b = boxes.clone();
            let horizontal = op == "dist-h";
            b.sort_by(|a, c| if horizontal { a.1.partial_cmp(&c.1) } else { a.2.partial_cmp(&c.2) }.unwrap_or(std::cmp::Ordering::Equal));
            let total: f32 = b.iter().map(|x| if horizontal { x.3 } else { x.4 }).sum();
            let span = if horizontal { x1 - x0 } else { y1 - y0 };
            let gap = if b.len() > 1 { (span - total) / (b.len() - 1) as f32 } else { 0.0 };
            let mut cur = if horizontal { x0 } else { y0 };
            for x in &b {
                if horizontal {
                    targets.push((x.0, cur, x.2));
                    cur += x.3 + gap;
                } else {
                    targets.push((x.0, x.1, cur));
                    cur += x.4 + gap;
                }
            }
        }
        _ => {
            for b in &boxes {
                let (nx, ny) = match op {
                    "left" => (x0, b.2),
                    "center" => ((x0 + x1) / 2.0 - b.3 / 2.0, b.2),
                    "right" => (x1 - b.3, b.2),
                    "top" => (b.1, y0),
                    "middle" => (b.1, (y0 + y1) / 2.0 - b.4 / 2.0),
                    "bottom" => (b.1, y1 - b.4),
                    _ => (b.1, b.2),
                };
                targets.push((b.0, nx, ny));
            }
        }
    }
    let Some(pg) = app.current_page_mut() else { return };
    for (id, x, y) in targets {
        let (ox, oy) = pg.element(id).map(|e| (e.x, e.y)).unwrap_or((x, y));
        let (dx, dy) = (x - ox, y - oy);
        // Move group members with their group.
        let members: Vec<u64> = pg.elements.iter().filter(|e| e.parent == Some(id)).map(|e| e.id).collect();
        for m in members.iter().chain(std::iter::once(&id)) {
            if let Some(e) = pg.element_mut(*m) {
                e.x += dx;
                e.y += dy;
            }
        }
    }
    super::canvas::sync_groups(pg);
}

/// Same width / height as the first selected.
pub fn match_size(app: &mut VizApp, w: bool, h: bool) {
    let sel = app.selected.clone();
    let Some(first) = sel.first().and_then(|id| app.project.element(*id)).map(|(_, e)| (e.w, e.h)) else { return };
    app.checkpoint("Match size");
    if let Some(pg) = app.current_page_mut() {
        for e in pg.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
            if w {
                e.w = first.0;
            }
            if h {
                e.h = first.1;
            }
        }
    }
}

/// Tableau-style tiled layout: arranges the non-floating elements of a dashboard in a grid.
pub fn tile(app: &mut VizApp) {
    app.checkpoint("Tile");
    let Some(pg) = app.current_page_mut() else { return };
    let (pw, ph) = (pg.width, pg.height);
    let ids: Vec<u64> = pg.elements.iter().filter(|e| !e.floating && !e.hidden && e.kind != ElementKind::Group).map(|e| e.id).collect();
    let n = ids.len();
    if n == 0 {
        return;
    }
    let cols = (n as f32).sqrt().ceil() as usize;
    let rows = n.div_ceil(cols);
    let gap = 12.0;
    let w = (pw - gap * (cols as f32 + 1.0)) / cols as f32;
    let h = (ph - gap * (rows as f32 + 1.0)) / rows as f32;
    for (i, id) in ids.iter().enumerate() {
        if let Some(e) = pg.element_mut(*id) {
            e.x = gap + (i % cols) as f32 * (w + gap);
            e.y = gap + (i / cols) as f32 * (h + gap);
            e.w = w;
            e.h = h;
        }
    }
}
