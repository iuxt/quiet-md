//! File drops on the library home.
use crate::app::{View, Workspace};
use crate::settings::display_title;
use crate::sidebar_drag::{Fade, Spring};
use crate::theme::{self, Theme, craft, ca};
use crate::ui::{icon, ui_text};
use gpui::{prelude::FluentBuilder, *};
use std::{rc::Rc, time::Instant};

/// Papers in the tray illustration.
const PAPERS: usize = 3;
const TAB: f32 = 0.06;

/// One paper's pose in tray coordinates: centre, size and tilt (radians).
#[derive(Clone, Copy)]
struct Paper { cx: f32, cy: f32, w: f32, h: f32, rot: f32, seed: u64 }

/// Everything both canvases need for one frame.
#[derive(Clone)]
struct Pose {
    w: f32,
    h: f32,
    lift: f32,
    open: f32,
    /// Front's top edge: left (higher) and right, and how far its top corners reach in
    /// (negative: out, the flap tipping toward the viewer as it opens).
    yl: f32,
    yr: f32,
    inset: f32,
    papers: Vec<Paper>,
}

/// A rounded rectangle turned by `rot` about its centre (card-local `c`, painted at `o`).
fn rounded(o: Point<Pixels>, c: (f32, f32), w: f32, h: f32, r: f32, rot: f32) -> Option<Path<Pixels>> {
    let mut pb = PathBuilder::fill();
    append_rounded(&mut pb, o, c, w, h, r, rot);
    pb.build().ok()
}

fn append_rounded(pb: &mut PathBuilder, o: Point<Pixels>, c: (f32, f32), w: f32, h: f32, r: f32, rot: f32) {
    let (s, k) = rot.sin_cos();
    let r = r.min(w / 2.).min(h / 2.);
    let mut first = true;
    for (ox, oy, start) in [(w / 2. - r, -h / 2. + r, -90f32), (w / 2. - r, h / 2. - r, 0.), (-w / 2. + r, h / 2. - r, 90.), (-w / 2. + r, -h / 2. + r, 180.)] {
        for i in 0..=6 {
            let a = (start + 15. * i as f32).to_radians();
            let (x, y) = (ox + r * a.cos(), oy + r * a.sin());
            let p = o + point(px(c.0 + x * k - y * s), px(c.1 + x * s + y * k));
            if first { pb.move_to(p); first = false; } else { pb.line_to(p); }
        }
    }
    pb.close();
}

impl Paper {
    /// A point on the paper, from its own top-left corner.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        let (s, k) = self.rot.sin_cos();
        let (lx, ly) = (x - self.w / 2., y - self.h / 2.);
        (self.cx + lx * k - ly * s, self.cy + lx * s + ly * k)
    }

    /// The sheet, its hairline edge and decorative lines.
    fn paint(&self, o: Point<Pixels>, titled: bool, window: &mut Window) {
        let b = Bounds::new(o + point(px(self.cx - self.w / 2.), px(self.cy - self.h / 2.)), size(px(self.w), px(self.h)));
        window.paint_shadows(b, Corners::all(px(10.)), &[BoxShadow { color: theme::rgbaf(0, 0, 0, if theme::dark() { 0.3 } else { 0.07 }), offset: point(px(0.), px(3.)), blur_radius: px(10.), spread_radius: px(0.) }]);
        if let Some(p) = rounded(o, (self.cx, self.cy), self.w + 1., self.h + 1., 10.5, self.rot) { window.paint_path(p, ca(craft::N1000, 0.08)); }
        if let Some(p) = rounded(o, (self.cx, self.cy), self.w, self.h, 10., self.rot) { window.paint_path(p, theme::raised(Theme::bg_body())); }
        let mut rng = self.seed;
        let mut next = || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; (rng % 1000) as f32 / 1000. };
        let pad = 12.;
        let mut y = if titled { 36. } else {
            // An untitled paper still carries a heading.
            let len = (self.w - 2. * pad) * (0.42 + 0.3 * next());
            if let Some(p) = rounded(o, self.at(pad + len / 2., 16.), len, 4., 2., self.rot) { window.paint_path(p, ca(craft::N900, 0.55)); }
            30.
        };
        let mut pb = PathBuilder::fill();
        let mut has_lines = false;
        while y < self.h - 14. {
            let full = self.w - 2. * pad;
            let len = if next() < 0.2 { full * (0.3 + 0.3 * next()) } else { full * (0.72 + 0.28 * next()) };
            append_rounded(&mut pb, o, self.at(pad + len / 2., y), len, 3., 1.5, self.rot);
            has_lines = true;
            y += 8.5;
        }
        if has_lines && let Ok(p) = pb.build() { window.paint_path(p, ca(craft::N1000, 0.10)); }
    }
}

/// The folder's back: a plate with a tab on its top-left.
fn back_plate(o: Point<Pixels>, p: &Pose, window: &mut Window) {
    let (w, h, r) = (p.w, p.h, 18.);
    let top = TAB * h + p.lift;
    let tab_top = p.lift;
    let tab = 0.40 * w;
    let pt = |x: f32, y: f32| o + point(px(x), px(y));
    let mut pb = PathBuilder::fill();
    pb.move_to(pt(0., h - r + p.lift));
    pb.line_to(pt(0., tab_top + r));
    pb.curve_to(pt(r, tab_top), pt(0., tab_top));
    pb.line_to(pt(tab - 22., tab_top));
    pb.cubic_bezier_to(pt(tab + 14., top), pt(tab - 6., tab_top), pt(tab - 2., top));
    pb.line_to(pt(w - r, top));
    pb.curve_to(pt(w, top + r), pt(w, top));
    pb.line_to(pt(w, h - r + p.lift));
    pb.curve_to(pt(w - r, h + p.lift), pt(w, h + p.lift));
    pb.line_to(pt(r, h + p.lift));
    pb.curve_to(pt(0., h - r + p.lift), pt(0., h + p.lift));
    pb.close();
    if let Ok(path) = pb.build() {
        window.paint_path(path, linear_gradient(180., linear_color_stop(ca(craft::N1000, 0.11), 0.), linear_color_stop(ca(craft::N1000, 0.07), 1.)));
    }
}

/// The frosted front: a sheet whose top steps down from a raised left part,
/// milky at the top and nearly opaque where the name sits, with a lit edge.
fn glass(o: Point<Pixels>, p: &Pose, window: &mut Window) {
    let (w, h, r) = (p.w, p.h + p.lift, 18.);
    let (yl, yr, i) = (p.yl + p.lift, p.yr + p.lift, p.inset);
    let step = 0.44 * w;
    let pt = |x: f32, y: f32| o + point(px(x), px(y));
    let edge = |pb: &mut PathBuilder| {
        pb.move_to(pt(i, yl + r));
        pb.curve_to(pt(i + r, yl), pt(i, yl));
        pb.line_to(pt(step - 16., yl));
        pb.cubic_bezier_to(pt(step + 26., yr), pt(step + 6., yl), pt(step + 6., yr));
        pb.line_to(pt(w - i - r, yr));
        pb.curve_to(pt(w - i, yr + r), pt(w - i, yr));
    };
    // The whole folder's shadow sits under the front.
    let lift = -p.lift / 4.;
    window.paint_shadows(Bounds::new(pt(6., yr + 10.), size(px(w - 12.), px(h - yr - 10.))), Corners::all(px(r)),
        &[BoxShadow { color: theme::rgbaf(0, 0, 0, if theme::dark() { 0.35 } else { 0.06 + 0.04 * lift }), offset: point(px(0.), px(8. + 6. * lift)), blur_radius: px(22. + 10. * lift), spread_radius: px(-2.) }]);
    let mut pb = PathBuilder::fill();
    edge(&mut pb);
    pb.line_to(pt(w, h - r));
    pb.curve_to(pt(w - r, h), pt(w, h));
    pb.line_to(pt(r, h));
    pb.curve_to(pt(0., h - r), pt(0., h));
    pb.close();
    let face = theme::raised(Theme::bg_body());
    if let Ok(path) = pb.build() {
        // Milky where the papers show through, all but opaque by the name; a faint
        // cool tint keeps the glass apart from the white papers behind it.
        window.paint_path(path.clone(), linear_gradient(180., linear_color_stop(face.opacity(0.78), 0.), linear_color_stop(face.opacity(0.97), 0.42)));
        window.paint_path(path, linear_gradient(180., linear_color_stop(ca(craft::N1000, 0.012), 0.), linear_color_stop(ca(craft::N1000, 0.035), 1.)));
    }
    let mut hair = PathBuilder::stroke(px(1.));
    edge(&mut hair);
    hair.line_to(pt(w, h - r));
    hair.curve_to(pt(w - r, h), pt(w, h));
    hair.line_to(pt(r, h));
    hair.curve_to(pt(0., h - r), pt(0., h));
    hair.close();
    if let Ok(path) = hair.build() { window.paint_path(path, ca(craft::N1000, 0.07)); }
    // The lit top edge: brighter as the front leans into the light.
    let mut lit = PathBuilder::stroke(px(1.5));
    edge(&mut lit);
    if let Ok(path) = lit.build() { window.paint_path(path, theme::rgbaf(255, 255, 255, if theme::dark() { 0.18 } else { 0.85 + 0.15 * p.open.clamp(0., 1.) })); }
}

/// A drag of files from outside (the Finder) over 主页:
/// the list dims behind a receiving tray, a glass folder whose papers (one
/// per file, up to three) rise out of it, follow the pointer and part as it
/// comes near; let go, they slide in, the folder dips, and the documents are
/// imported where it says. The tray steps back when the pointer is over a document row.
pub(crate) struct FileDrop {
    paths: Vec<std::path::PathBuf>,
    md: usize,
    xml: usize,
    dirs: usize,
    other: usize,
    at: Point<Pixels>,
    /// The document row under the pointer.
    over: Option<String>,
    /// The pane's bounds last frame: anywhere in it takes the drop.
    zone: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// Where it was let go, which the ripple spreads from.
    drop_at: Point<Pixels>,
    inside: bool,
    show: Fade,
    veil: Fade,
    arm: Fade,
    dip: Fade,
    land: Fade,
    shake: Fade,
    rise: [Fade; PAPERS],
    /// The pointer's place over the tray (−1…1 each way), which the papers lean toward.
    tilt: Spring<2>,
    /// Let go: where the files went (None: the top of 我的文档).
    landed: Option<Option<String>>,
    leaving: bool,
    born: Instant,
    last: Option<Instant>,
}

/// Tray illustration: the glass folder's box.
const ART_W: f32 = 196.;
const ART_H: f32 = 168.;

impl FileDrop {
    fn new(paths: Vec<std::path::PathBuf>) -> Self {
        let (mut md, mut xml, mut dirs, mut other) = (0, 0, 0, 0);
        for p in &paths {
            if p.is_dir() { dirs += 1; }
            else if crate::local_files::is_markdown_path(p) { md += 1; }
            else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")) { xml += 1; }
            else { other += 1; }
        }
        Self {
            paths, md, xml, dirs, other, at: point(px(0.), px(0.)), over: None, zone: Default::default(), inside: false, drop_at: point(px(0.), px(0.)),
            show: Fade::new(0., 240., 21.), veil: Fade::new(0., 160., 25.), arm: Fade::new(0., 300., 23.),
            dip: Fade::new(0., 520., 15.), land: Fade::new(0., 150., 21.), shake: Fade::new(0., 700., 9.),
            // Each paper surfaces a little after the one in front.
            rise: [Fade::new(0., 320., 21.), Fade::new(0., 210., 17.), Fade::new(0., 140., 14.)],
            tilt: Spring::new([0., 0.]), landed: None, leaving: false, born: Instant::now(), last: None,
        }
    }

    fn importable(&self) -> usize { self.md + self.xml + self.dirs }

    /// The pointer went somewhere else that takes the drop (the sidebar).
    pub(crate) fn leave(&mut self) { if self.landed.is_none() { self.leaving = true; } }

    /// "2 个 Markdown 文件 · 1 个文件夹".
    fn summary(&self) -> String {
        let mut parts = vec![];
        if self.md > 0 { parts.push(crate::localized_format!("{} 个 Markdown 文件", "{} Markdown files", self.md)); }
        if self.xml > 0 { parts.push(crate::localized_format!("{} 个 DocxXML 文件", "{} DocxXML files", self.xml)); }
        if self.dirs > 0 { parts.push(crate::localized_format!("{} 个文件夹", "{} folders", self.dirs)); }
        if self.other > 0 && !parts.is_empty() { parts.push(crate::localized_format!("{} 个不支持的文件会被跳过", "{} unsupported skipped", self.other)); }
        parts.join(" · ")
    }

    /// The front paper's tag: what kind of file leads.
    fn tag(&self) -> Option<&'static str> {
        if self.md > 0 { Some("MD") } else if self.xml > 0 { Some("XML") } else { None }
    }
}

impl Workspace {
    /// Files dragged over 主页 (the pointer, the folder
    /// row under it, whether it is over the tray).
    pub(crate) fn file_drag_move(&mut self, ev: &DragMoveEvent<ExternalPaths>, cx: &mut Context<Self>) {
        // Every listener hears every move: off the page (over the sidebar) it steps back.
        if !ev.bounds.contains(&ev.event.position) {
            if let Some(d) = self.home.file_drop.as_mut() { d.leave(); cx.notify(); }
            return;
        }
        let paths = ev.drag(cx).paths().to_vec();
        self.file_drag_at(ev.event.position, paths, cx);
    }

    /// `file_drag_move` from a position and the dragged paths (scripted runs too).
    pub(crate) fn file_drag_at(&mut self, at: Point<Pixels>, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        if self.view != View::Home { return; }
        let fresh = self.home.file_drop.as_ref().is_none_or(|d| d.landed.is_some());
        if fresh { self.home.file_drop = Some(FileDrop::new(paths)); }
        let over = self.home.doc_bounds.borrow().iter().find(|(_, b)| b.contains(&at)).map(|(id, _)| id.clone());
        let Some(d) = self.home.file_drop.as_mut() else { return };
        d.leaving = false;
        d.over = over;
        d.at = at;
        d.inside = d.zone.get().is_some_and(|z| z.contains(&at));
        cx.notify();
    }

    /// Import under the document row at the pointer, or at the top of 我的文档.
    pub(crate) fn file_dropped(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        let paths = paths.paths().to_vec();
        self.file_drag_at(window.mouse_position(), paths.clone(), cx);
        self.file_drop_paths(paths, window, cx);
    }

    pub(crate) fn file_drop_paths(&mut self, paths: Vec<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if self.view != View::Home { return; }
        if self.home.file_drop.is_none() { self.home.file_drop = Some(FileDrop::new(paths)); }
        let Some(d) = self.home.file_drop.as_mut() else { return };
        if d.importable() == 0 {
            d.shake.v += 34.;
            d.leaving = true;
            cx.notify();
            return;
        }
        let target = d.over.clone();
        d.landed = Some(target.clone());
        d.drop_at = d.at;
        d.land.to = 1.;
        d.dip.v += 5.5;
        let (dirs, files): (Vec<_>, Vec<_>) = d.paths.iter().cloned().partition(|p| p.is_dir());
        let files: Vec<_> = files.into_iter().filter(|p| crate::local_files::is_markdown_path(p) || p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml"))).collect();
        let paths: Vec<_> = files.into_iter().chain(dirs).collect();
        self.import_paths(&paths, target, false, window, cx);
        cx.notify();
    }

    pub(crate) fn file_drop_over(&self, id: &str) -> bool {
        self.home.file_drop.as_ref().is_some_and(|d| d.landed.is_none() && !d.leaving && d.importable() > 0 && d.over.as_deref() == Some(id))
    }

    /// The veil and the tray, over the page while files are dragged in.
    pub(crate) fn render_file_drop(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.view != View::Home { self.home.file_drop = None; return None; }
        let dragging = cx.has_active_drag() || crate::devtools::file_drag();
        let target_title = |w: &Self, id: Option<&String>| id.and_then(|id| w.entries.iter().find(|e| &e.id == id)).map(|e| display_title(&e.title))
            .unwrap_or_else(|| crate::settings::tr("我的文档", "My documents").to_string());
        let d = self.home.file_drop.as_ref()?;
        let place = target_title(self, d.landed.clone().flatten().as_ref().or(d.over.as_ref()));
        let d = self.home.file_drop.as_mut()?;
        // The drag went away without a drop here (left the window, or let go elsewhere).
        if !dragging && d.landed.is_none() { d.leaving = true; }
        let now = Instant::now();
        let dt = d.last.replace(now).map_or(1. / 120., |t| now.duration_since(t).as_secs_f32().min(0.032));
        let reject = d.importable() == 0;
        // Leave the target row visible.
        let over = d.over.is_some() && !reject && d.landed.as_ref().is_none_or(|t| t.is_some() && *t == d.over);
        d.show.to = if d.leaving { 0. } else { 1. };
        d.veil.to = if d.leaving { 0. } else if over { 0.12 } else { 1. };
        d.arm.to = if !reject && !d.leaving && (d.inside || d.landed.is_some()) { 1. } else { 0. };
        for (i, r) in d.rise.iter_mut().enumerate() { r.to = if d.leaving && d.landed.is_none() { 0. } else { 1. - 0.12 * i as f32 }; }
        let zone = d.zone.get();
        d.tilt.target = match zone.filter(|_| d.inside && d.landed.is_none()) {
            Some(z) => [((d.at.x - z.center().x) / (z.size.width / 2.)).clamp(-1., 1.), ((d.at.y - z.center().y) / (z.size.height / 2.)).clamp(-1., 1.)],
            None => [0., 0.],
        };
        for f in [&mut d.show, &mut d.veil, &mut d.arm, &mut d.dip, &mut d.land, &mut d.shake] { f.step(dt); }
        d.tilt.step(dt);
        for r in &mut d.rise { r.step(dt); }
        // Landed and settled: the tray goes.
        if d.landed.is_some() && d.land.p > 0.97 && !d.leaving { d.leaving = true; }
        if d.leaving && d.show.p < 0.01 && d.veil.p < 0.01 { self.home.file_drop = None; cx.notify(); return None; }
        // The tray breathes while it waits, so it draws every frame while shown.
        window.request_animation_frame();

        // ---- The illustration's pose.
        // `lit`: the pane's glow, full while the pointer is over it. The glass folder
        // opens only a little with it, so it keeps its shape.
        let (show, lit, land, dip) = (d.show.unit(), d.arm.p, d.land.p, d.dip.p);
        let arm = 0.3 * lit;
        let t = d.born.elapsed().as_secs_f32();
        let breath = 0.5 + 0.5 * (t * std::f32::consts::TAU / 1.8).sin();
        let shake = d.shake.p * (t * 46.).sin() * 0.35;
        let n = d.importable().clamp(1, PAPERS);
        let (w, h) = (ART_W, ART_H);
        let (o, m) = (arm.max(0.35 * show), arm);
        let lift = -5. * o - 2. * m + 3. * dip + 6. * land;
        let [tx, ty] = d.tilt.p;
        let pose_of = |i: usize, r: f32| -> Paper {
            let (rest, rise, part) = [
                ((0.0, 0.16, 0.0), (0.0, -0.07, 0.0), (0.0, -0.03, 0.0)),
                ((0.03, 0.135, 2.5), (0.03, -0.055, 2.0), (0.06, -0.02, 4.0)),
                ((-0.035, 0.115, -3.0), (-0.03, -0.05, -2.5), (-0.06, -0.02, -4.0)),
            ][i];
            let (pw, ph) = (0.62 * w, 0.62 * h);
            // Out of the folder by `r`, higher while armed, sunk back in by `land`.
            let up = r * (1.6 + 1.1 * arm) - 2.4 * land;
            let dx = (rest.0 + rise.0 * up + part.0 * m) * w + tx * (5. + 3. * i as f32) * (1. - land);
            let top = (rest.1 + rise.1 * up + part.1 * m) * h + lift + ty * 3. * (1. - land);
            Paper { cx: w / 2. + dx, cy: top + ph / 2., w: pw, h: ph, rot: (rest.2 + rise.2 * up + part.2 * m + 2.5 * tx * (1. - land)).to_radians(), seed: 7 + i as u64 }
        };
        let papers: Vec<Paper> = (0..n).map(|i| pose_of(i, d.rise[i].p)).collect();
        let lean = 0.03 * h * o + 0.045 * h * m + 0.05 * h * dip;
        let pose = Pose { w, h, lift, open: o, yl: 0.49 * h + lean, yr: 0.54 * h + lean, inset: -(3. * o + 7. * m), papers };
        let tag = d.tag();
        // The front paper's name: the first file of the kind it shows.
        let first = d.paths.iter().find(|p| match tag { Some("MD") => crate::local_files::is_markdown_path(p), Some(_) => p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")), None => p.is_dir() })
            .or(d.paths.first()).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let summary = d.summary();
        let record = d.zone.clone();
        let (at, drop_at, landed, veil_p, shake_x) = (d.at, d.drop_at, d.landed.is_some(), d.veil.p, shake * 18.);

        // ---- Elements.
        let accent = Theme::primary();
        let quiet = if reject { 1. } else { 0. };
        let (title, sub) = if reject {
            (crate::settings::tr("这些文件不能导入", "These files can't be imported").to_string(), crate::settings::tr("支持 Markdown、DocxXML 文件和文件夹", "Markdown, DocxXML files and folders are supported").to_string())
        } else if d.landed.is_some() {
            (crate::localized_format!("已导入到「{}」", "Imported into “{}”", place), summary)
        } else if over {
            (crate::localized_format!("松手放入「{}」", "Release to put it in “{}”", place), summary)
        } else {
            (crate::localized_format!("松手导入到「{}」", "Release to import into “{}”", place), summary)
        };
        let back = { let pose = pose.clone(); canvas(|_, _, _| {}, move |b, _, window, _| {
            back_plate(b.origin, &pose, window);
            // The front paper is the first file: its kind and name in place of a heading.
            for (i, p) in pose.papers.iter().enumerate().rev() { p.paint(b.origin, i == 0, window); }
        }).absolute().inset_0() };
        let chip = pose.papers.first().copied().map(|p| div().absolute().left(px(p.cx - p.w / 2. + 10.)).top(px(p.cy - p.h / 2. + 9.)).w(px(p.w - 20.))
            .flex().items_center().gap(px(5.)).overflow_hidden()
            .map(|d| match tag {
                Some(t) => d.child(div().flex_none().px(px(4.)).h(px(14.)).rounded(px(4.)).bg(accent).flex().items_center()
                    .text_size(px(8.5)).line_height(px(12.)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(t)),
                None => d.child(icon("folder", 12., Theme::icon())),
            })
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(10.)).line_height(px(14.))
                .font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(ui_text(first.clone()))));
        let front = { let pose = pose.clone(); canvas(|_, _, _| {}, move |b, _, window, _| glass(b.origin, &pose, window)).absolute().inset_0() };
        let grow = 0.9 + 0.1 * show + 0.05 * lit;
        let art = div().relative().flex_none().w(px(w)).h(px(h)).ml(px(shake_x)).opacity(1. - 0.35 * quiet)
            .child(back).children(chip).child(front);
        // The middle: the glass folder on a halo of the accent, the line under it. No box:
        // the whole pane is the place to drop.
        let middle = div().relative().flex().flex_col().items_center().gap(px(6.))
            .opacity(show * (0.08 + 0.92 * ((veil_p - 0.12) / 0.88).clamp(0., 1.)).min(show))
            .child(div().relative().flex_none().w(px(w * grow)).h(px(h * grow)).flex().items_center().justify_center()
                .child(div().absolute().left(px(w * grow / 2. - 40.)).top(px(h * grow / 2. - 30.)).size(px(80.)).rounded_full()
                    .shadow(vec![BoxShadow { color: if reject { ca(craft::N1000, 0.06) } else { accent.opacity(0.10 + 0.10 * lit + 0.06 * breath) }, offset: point(px(0.), px(0.)), blur_radius: px(90.), spread_radius: px(40. + 24. * lit) }]))
                .child(art))
            .child(div().mt(px(14.)).text_size(px(18.)).line_height(px(26.)).font_weight(FontWeight::SEMIBOLD)
                .text_color(if reject { Theme::text_caption() } else { Theme::text() }).child(ui_text(title)))
            .child(div().text_size(px(13.)).line_height(px(20.)).text_color(Theme::text_caption()).child(ui_text(sub)));
        // The pane's edge: a glow breathing in from every side, a dashed line settling
        // onto it, a halo under the pointer, and on a drop a
        // ripple from where it landed while the edge flares.
        let flare = if landed { (land * std::f32::consts::PI).sin().max(0.) } else { 0. };
        let edge = canvas(move |b, _, _| record.set(Some(b)), move |b, _, window, _| window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
            let tint = if reject { ca(craft::N1000, 1.) } else { accent };
            let clear = tint.opacity(0.);
            let (x0, y0, wd, ht) = (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
            let rect = |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(px(x), px(y)), size(px(w.max(0.)), px(h.max(0.))));
            // A dashed edge, set in from the pane: the place to let go. It settles
            // from further inside onto its place as it shows.
            let inset = 22. + 30. * (1. - show);
            let r: f32 = 88.;
            let ring = rect(x0 + inset + shake_x * 0.4, y0 + inset, wd - 2. * inset, ht - 2. * inset);
            // The glow starts at the dashed line and fades inward: the blurred shadow of a
            // frame just outside it, seen only inside. Blur keeps the falloff continuous
            // and rounds the corners.
            let depth = (46. + 34. * lit + 30. * flare) * show;
            let a = if reject { 0.10 } else { (0.16 + 0.06 * breath + 0.16 * lit + 0.2 * flare) * show };
            let (l, t, rw, rh, big) = (f32::from(ring.left()), f32::from(ring.top()), f32::from(ring.size.width), f32::from(ring.size.height), 400.);
            let glow = [BoxShadow { color: tint.opacity(a), offset: point(px(0.), px(0.)), blur_radius: px(depth), spread_radius: px(0.) }];
            // Masks are only rectangles, so the ring's squircle inside (see `ui::squircle_path`)
            // is cut in 1px rows through the corner bands (the dashed line covers the steps);
            // one rect between.
            let rr = r.min(rw / 2.).min(rh / 2.).max(0.);
            let n = rr.ceil() as usize;
            let rows = (0..n).flat_map(|i| {
                let (y, hh) = (i as f32, (rr - i as f32).min(1.));
                let dx = rr - (rr.powi(4) - (rr - y - hh / 2.).powi(4)).max(0.).powf(0.25);
                [rect(l + dx, t + y, rw - 2. * dx, hh), rect(l + dx, t + rh - y - hh, rw - 2. * dx, hh)]
            }).chain([rect(l, t + rr, rw, rh - 2. * rr)]);
            for clip in rows {
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    // Each strip reaches a point inside, or GPUI culls a shadow wholly outside the clip.
                    for side in [rect(l - big, t - big, rw + 2. * big, big + 1.), rect(l - big, t + rh - 1., rw + 2. * big, big + 1.), rect(l - big, t, big + 1., rh), rect(l + rw - 1., t, big + 1., rh)] {
                        window.paint_shadows(side, px(0.).into(), &glow);
                    }
                });
            }
            // The halo under the pointer.
            if !reject && !landed {
                let c = point(at.x.max(b.left()).min(b.right()), at.y.max(b.top()).min(b.bottom()));
                window.paint_shadows(Bounds::centered_at(c, size(px(40.), px(40.))), px(20.).into(),
                    &[BoxShadow { color: accent.opacity(0.12 * lit * show), offset: point(px(0.), px(0.)), blur_radius: px(120.), spread_radius: px(70.) }]);
            }
            let ring_a = if reject { 0.22 } else { (0.40 + 0.12 * breath + 0.35 * lit + 0.2 * flare).min(1.) } * show;
            if let Some(p) = crate::ui::squircle_path(ring, r, 1., PathBuilder::stroke(px(2.)).dash_array(&[px(6.), px(5.)])) { window.paint_path(p, tint.opacity(ring_a)); }
            // The ripple from the drop.
            if landed {
                let far = wd.hypot(ht);
                for (k, delay) in [(0, 0.), (1, 0.18)] {
                    let q = ((land - delay) / (1. - delay)).clamp(0., 1.);
                    if q <= 0. { continue; }
                    let e = 1. - (1. - q).powi(3);
                    let rad = 30. + e * far * 0.8;
                    let c = point(drop_at.x, drop_at.y);
                    window.paint_quad(quad(Bounds::centered_at(c, size(px(2. * rad), px(2. * rad))), px(rad), clear, px(3. - k as f32), accent.opacity(0.5 * (1. - q)), BorderStyle::Solid));
                }
            }
        })).absolute().inset_0();
        let veil = theme::raised(Theme::bg_body()).opacity(0.86 * d.veil.p.clamp(0., 1.));
        Some(div().absolute().inset_0().bg(veil).child(edge).flex().items_center().justify_center().child(middle).into_any_element())
    }
}
