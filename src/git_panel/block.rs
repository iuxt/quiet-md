//! Read-only document blocks in Git comparisons.
use super::{EquationNumbers, text::PreviewText};
use crate::{editor::image::picture, model::BlockKind, settings::display_title, theme::{self, Theme}, ui::icon};
use gpui::{prelude::FluentBuilder, *};

const RADIUS: f32 = 10.;
// Keep the existing callout's closed border: a focus decoration is a separate
// quote treatment, not a replacement for a stored border colour.
const CALLOUT_RADIUS: f32 = 8.;
const INSET: f32 = 16.;
const ICON: f32 = 22.;
const ICON_GAP: f32 = 12.;
// Native Styling Basics sample, measured at 2x; these are not PageStyles CSS.
const QUOTE_INSET: f32 = 14.;
const QUOTE_BAR: f32 = 4.;
const QUOTE_PAD: f32 = 4.;
const PARAGRAPH_GAP: f32 = 11.;
const LIST_GAP: f32 = 5.;
const CODE_HEADER: f32 = 36.;
const CODE_GUTTER: f32 = 40.;
const CODE_GAP: f32 = 16.;

fn typography(kind: &BlockKind) -> crate::editor_theme::BlockMetrics {
    match kind {
        BlockKind::Heading { level, .. } => crate::editor_theme::heading_metrics(*level),
        BlockKind::Code { .. } => crate::editor_theme::BlockMetrics { size: 13., line_height: 20., mt: 16., mb: 16. },
        _ => crate::editor_theme::BlockMetrics { size: crate::editor_theme::body_size(), line_height: crate::editor_theme::body_line(), mt: 0., mb: PARAGRAPH_GAP },
    }
}

fn weight(kind: &BlockKind) -> FontWeight {
    if crate::settings::feishu() { return if matches!(kind, BlockKind::Heading { .. }) { FontWeight::MEDIUM } else { FontWeight::NORMAL }; }
    match kind { BlockKind::Heading { level: 1 | 2, .. } => FontWeight::BOLD, BlockKind::Heading { .. } => FontWeight::SEMIBOLD, _ => FontWeight::NORMAL }
}

fn gap(kind: &BlockKind) -> (f32, f32) {
    match kind {
        BlockKind::Heading { .. } => { let m = typography(kind); (m.mt, m.mb) }
        BlockKind::Bullet | BlockKind::Ordered { .. } | BlockKind::Todo { .. } => (0., LIST_GAP),
        BlockKind::Table(_) => (11., 11.),
        BlockKind::Code { .. } | BlockKind::Callout { .. } | BlockKind::Image { .. } | BlockKind::Drawing(_) | BlockKind::File(_) | BlockKind::Grid => (16., 16.),
        BlockKind::Quote => (8., 14.),
        BlockKind::Divider => (12., 12.),
        _ => (0., PARAGRAPH_GAP),
    }
}

fn preview_text(text: &crate::model::RichText) -> crate::model::RichText {
    let mut preview = crate::model::RichText::default();
    for (range, marks) in text.spans() {
        let mut marks = marks.clone();
        if marks.inline.as_ref().is_some_and(|i| i.equation().is_some()) {
            preview.push(&text.text[range], marks);
            continue;
        }
        let label = marks.inline.as_ref().map(|inline| match &inline.kind {
            crate::model::InlineKind::Equation(_) => unreachable!(),
            crate::model::InlineKind::File(file) => file.name.clone(),
            crate::model::InlineKind::Button(button) => button.text.clone(),
            crate::model::InlineKind::Reminder(reminder) => crate::editor::reminder::reminder_label(reminder, " "),
            crate::model::InlineKind::DocLink(doc) => display_title(&doc.title).to_string(),
            crate::model::InlineKind::Other { tag } => tag.clone(),
        }).map(|label| label.chars().take(160).collect::<String>());
        if let Some(inline) = &marks.inline { match &inline.kind {
            crate::model::InlineKind::Equation(_) => {},
            crate::model::InlineKind::Button(_) => marks.bold = true,
            crate::model::InlineKind::DocLink(_) | crate::model::InlineKind::File(_) => marks.link = Some(String::new()),
            _ => {},
        } }
        marks.inline = None;
        preview.push(&text.text[range].replace('\u{fffc}', label.as_deref().unwrap_or("")), marks);
    }
    preview
}

/// Inert document rendering for the Git panel, at the normal reading size.
pub(super) fn formatted_block(b: &crate::model::Block, eqnos: &EquationNumbers) -> AnyElement {
    render_block(b, Theme::text(), false, (false, 0), eqnos)
}

fn render_block(b: &crate::model::Block, inherited: Hsla, quote: bool, list: (bool, usize), eqnos: &EquationNumbers) -> AnyElement {
    let (cell, list_depth) = list;
    let metrics = typography(&b.kind);
    let (font_size, line_height) = if cell && !matches!(b.kind, BlockKind::Heading { .. } | BlockKind::Code { .. }) { (14., 22.) } else { (metrics.size, metrics.line_height) };
    let done = matches!(b.kind, BlockKind::Todo { done: true });
    let color = if done { Theme::todo_done() } else { inherited };
    let mut node = div().w_full().min_w_0().flex_none().flex().flex_col()
        .text_size(px(font_size)).line_height(px(line_height)).text_color(color)
        .when(b.kind.has_text(), |d| d.min_h(px(line_height)));
    if let Some(bg) = b.bg.as_ref().and_then(|c| theme::block_bg(&c.0)) { node = node.rounded(px(6.)).bg(bg); }
    match &b.kind {
        BlockKind::Drawing(d) => {
            let w = (d.width as f32 / 2.).clamp(32., 640.);
            let h = (w * d.height as f32 / d.width as f32).clamp(42., 420.);
            let preview = crate::editor::image::drawing_picture(d, w.max(h));
            return node.child(div().w(px(w)).h(px(h)).max_w_full().rounded(px(RADIUS)).overflow_hidden()
                .child(preview)).into_any_element();
        }
        BlockKind::Image { src, width, height, caption, .. } => {
            let path = crate::store::image_path(src);
            let ratio = width.unwrap_or(690.).max(1.) / height.unwrap_or(460.).max(1.);
            let w = width.unwrap_or(690.).min(690.);
            let long_edge = w.max(w / ratio);
            node = match b.align { crate::model::Align::Left => node.items_start(), crate::model::Align::Center => node.items_center(), crate::model::Align::Right => node.items_end() };
            // Reserve the proportional height at the available width before loading.
            let media = div().relative().flex_none().w_full().max_w_full().overflow_hidden().when_some(*width, |d, w| d.w(px(w)))
                .map(|mut d| { d.style().aspect_ratio = Some(ratio); d });
            node = node.child(if let Some(path) = path {
                media.child(picture(&path, long_edge * 0.75).absolute().inset_0().size_full().min_w_0().min_h_0().rounded(px(RADIUS)).object_fit(ObjectFit::Contain)).into_any_element()
            } else {
                media.rounded(px(RADIUS)).bg(Theme::code_bg()).flex().items_center().justify_center().child(icon("image-broken", 24., Theme::text_disabled())).into_any_element()
            });
            return node.when_some(caption.as_ref().filter(|c| !c.is_empty()), |d, caption| d.child(div().w_full().max_w_full().when_some(*width, |d, w| d.w(px(w))).mt(px(6.)).text_size(px(13.)).line_height(px(20.)).text_color(Theme::text_caption()).text_center().child(caption.clone()))).into_any_element();
        }
        BlockKind::Divider => return node.h(px(25.)).justify_center().child(div().h(px(1.)).w_full().bg(Theme::line_divider())).into_any_element(),
        BlockKind::Grid => node = node.flex_row().gap(px(20.)),
        BlockKind::Column { .. } => node = node.flex_1(),
        BlockKind::Quote => node = node.relative().py(px(QUOTE_PAD)).pl(px(QUOTE_INSET))
            .child(div().absolute().left_0().top_0().bottom_0().w(px(QUOTE_BAR)).rounded_full().bg(color)),
        BlockKind::Callout { emoji, bg, border, text_color } => {
            let tc = text_color.as_ref().and_then(|c| theme::text_color(&c.0)).unwrap_or(color);
            let mut body = div().flex_1().min_w_0().flex().flex_col();
            for (i, child) in b.children.iter().enumerate() {
                let gap = if i == 0 { 0. } else { gap(&b.children[i - 1].kind).1.max(gap(&child.kind).0) };
                body = body.child(div().mt(px(gap)).child(render_block(child, tc, quote, (cell, list_depth), eqnos)));
            }
            let first_line = b.children.first().map(|b| typography(&b.kind).line_height).unwrap_or(theme::BODY_LINE);
            return node.relative().p(px(INSET)).rounded(px(CALLOUT_RADIUS)).border_1().border_color(border.as_ref().and_then(|c| crate::editor_theme::callout_border(&c.0)).unwrap_or_else(transparent_black)).bg(bg.as_ref().and_then(|c| crate::editor_theme::callout_bg(&c.0)).unwrap_or_else(transparent_black))
                .child(div().flex().gap(px(ICON_GAP)).child(div().size(px(ICON)).mt(px((first_line - ICON) / 2.)).text_size(px(20.)).line_height(px(22.)).flex_none().child(emoji.clone())).child(body)).into_any_element();
        }
        BlockKind::Code { lang, caption, wrap } => {
            node = node.rounded(px(RADIUS)).border_1().border_color(Theme::line_border()).bg(Theme::code_bg()).font_family(theme::MONO_FONT)
                .child(div().h(px(CODE_HEADER)).pl(px(28.)).pr(px(8.)).rounded_t(px(RADIUS - 1.)).bg(Theme::code_gutter()).flex().items_center().font_family(theme::UI_FONT).text_size(px(13.)).text_color(Theme::text_caption()).child(caption.clone().unwrap_or_else(|| crate::settings::text("代码块").into())));
            let spans = crate::highlight::spans(lang, &b.text.text);
            let mut body = div().pt(px(14.)).pb(px(18.)).pr(px(20.)).flex().flex_col();
            let mut off = 0;
            for (i, line) in b.text.text.split('\n').enumerate() {
                let end = off + line.len();
                let mut cuts = vec![off, end];
                for (r, _, _) in &spans {
                    if r.start > off && r.start < end { cuts.push(r.start); }
                    if r.end > off && r.end < end { cuts.push(r.end); }
                }
                cuts.sort_unstable(); cuts.dedup();
                let runs = cuts.windows(2).map(|r| {
                    let h = spans.iter().find(|(h, _, _)| h.start <= r[0] && r[1] <= h.end);
                    let mut f = font(theme::MONO_FONT); if h.is_some_and(|(_, _, it)| *it) { f.style = FontStyle::Italic; }
                    TextRun { len: r[1] - r[0], font: f, color: h.map_or(Theme::text_code(), |(_, c, _)| *c), background_color: None, underline: None, strikethrough: None }
                }).collect::<Vec<_>>();
                body = body.child(div().flex().items_start()
                    .child(div().w(px(CODE_GUTTER)).mr(px(CODE_GAP)).flex_none().text_right().text_color(Theme::text_caption()).child((i + 1).to_string()))
                    .child(div().min_w_0().flex_1().when(!*wrap, |d| d.whitespace_nowrap()).child(if line.is_empty() { StyledText::new(" ") } else { StyledText::new(line.to_string()).with_runs(runs) })));
                off = end + 1;
            }
            return node.child(div().id(("diff-code", b.id.0)).w_full().overflow_x_scroll().child(body)).into_any_element();
        }
        BlockKind::Table(table) => {
            let widths = &table.col_widths;
            let mut table_el = div().grid().grid_col_widths(widths.clone()).w(px(widths.iter().sum()));
            for (r, c, value) in table.cells() {
                let (rs, cs) = (value.rowspan as usize, value.colspan as usize);
                let header = r < table.header_rows || (table.header_col && c == 0);
                let mut col = div().min_w_0().flex().flex_col().row_start(r as i16 + 1).row_end((r + rs) as i16 + 1).col_start(c as i16 + 1).col_end((c + cs) as i16 + 1)
                    .p(px(8.)).border_r(px(1.)).border_b(px(1.)).when(r == 0, |d| d.border_t(px(1.))).when(c == 0, |d| d.border_l(px(1.))).border_color(Theme::line_border())
                    .when(header, |d| d.bg(Theme::table_header())).when_some(value.bg.as_ref().and_then(|c| theme::block_bg(&c.0)), |d, c| d.bg(c))
                    .map(|d| match value.valign.as_deref() { Some("middle") => d.justify_center(), Some("bottom") => d.justify_end(), _ => d });
                for (i, child) in value.blocks.iter().enumerate() {
                    let gap = if i == 0 { 0. } else { gap(&value.blocks[i - 1].kind).1.min(6.).max(gap(&child.kind).0.min(8.)) };
                    col = col.child(div().mt(px(gap)).child(render_block(child, color, quote, (true, 0), eqnos)));
                }
                table_el = table_el.child(col);
            }
            return node.child(div().id(("diff-table", b.id.0)).w_full().overflow_x_scroll().child(table_el)).into_any_element();
        }
        BlockKind::File(f) => return node.p(px(12.)).rounded(px(RADIUS)).border_1().border_color(Theme::line_border()).bg(Theme::code_bg()).flex_row().items_center().gap(px(12.))
            .child(icon("file-text", 30., Theme::icon())).child(div().flex_1().min_w_0().text_size(px(15.)).font_weight(FontWeight::MEDIUM).child(f.name.clone())).into_any_element(),
        BlockKind::Countdown(c) => {
            let left = c.remaining(crate::time::now_ms());
            let color = u32::from_str_radix(c.color.trim_start_matches('#'), 16).map(theme::c).unwrap_or(Theme::primary_500());
            let mut ink = color; ink.l = ink.l.min(0.38);
            let mut row = div().flex().justify_center().gap(px(16.));
            for (i, value) in [left / 86400, left / 3600 % 24, left / 60 % 60, left % 60].into_iter().enumerate() {
                row = row.child(div().flex().flex_col().items_center().child(div().size(px(52.)).rounded(px(8.)).bg(color).flex().items_center().justify_center().font_family(theme::MONO_FONT).font_weight(FontWeight::MEDIUM).text_size(px(26.)).text_color(ink).child(format!("{value:02}")))
                    .child(div().mt(px(4.)).text_size(px(14.)).line_height(px(20.)).text_color(Theme::text_caption()).child([crate::settings::text("天"), crate::settings::text("时"), crate::settings::text("分"), crate::settings::text("秒")][i])));
            }
            return node.py(px(18.)).rounded(px(RADIUS)).border_1().border_color(Theme::line_border()).bg(Theme::code_bg()).child(row).into_any_element();
        }
        BlockKind::SubPageList => {
            node = node.p(px(8.)).rounded(px(RADIUS)).border_1().border_color(Theme::line_border()).bg(Theme::bg_overlay());
            if b.children.is_empty() { return node.h(px(96.)).justify_center().text_center().text_color(Theme::text_caption()).child(crate::settings::text("目录下暂无子页面")).into_any_element(); }
            for child in &b.children { node = node.child(div().py(px(9.)).px(px(6.)).flex().items_start().gap(px(6.)).child(icon("doc", 16., Theme::icon())).child(div().flex_1().min_w_0().font_weight(FontWeight::MEDIUM).child(child.text.text.clone()))); }
            return node.into_any_element();
        }
        BlockKind::Raw { tag, .. } => return node.p(px(8.)).rounded(px(8.)).bg(Theme::bg_overlay()).text_color(Theme::text_caption()).child(crate::localized_format!("<{tag}> 已保留的内容", "<{tag}> Preserved content")).into_any_element(),
        _ => {}
    }
    if b.kind.has_text() {
        let text = preview_text(&b.text);
        let runs = text.runs.iter().map(|r| {
            let mut f = font(if r.marks.code { theme::MONO_FONT } else { theme::UI_FONT });
            f.weight = if r.marks.bold { FontWeight::BOLD } else { weight(&b.kind) };
            if r.marks.italic || quote { f.style = FontStyle::Italic; }
            let mut ink = if r.marks.link.is_some() { Theme::link() } else { r.marks.text_color.as_ref().and_then(|c| theme::text_color(&c.0)).unwrap_or(color) };
            if done { ink.a = 0.5; }
            let mark = r.marks.bg_color.as_ref().and_then(|c| theme::bg_color(&c.0));
            TextRun { len: r.len, font: f, color: if mark.is_some() { crate::theme::on_mark(ink) } else { ink },
                background_color: mark.or_else(|| r.marks.code.then(Theme::code_bg)),
                underline: (r.marks.underline || r.marks.link.is_some()).then_some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
                strikethrough: (r.marks.strike || done).then_some(StrikethroughStyle { thickness: px(1.), color: None }) }
        }).collect::<Vec<_>>();
        let marker = match b.kind {
            BlockKind::Bullet => Some(div().w(px(24.)).h(px(line_height)).pl(px(4.)).flex_none().flex().items_center().child(match list_depth % 3 {
                0 => div().size(px(5.5)).rounded_full().bg(Theme::text_caption()),
                1 => div().size(px(5.5)).rounded_full().border_1().border_color(Theme::text_caption()),
                _ => div().size(px(5.)).bg(Theme::text_caption()),
            }).into_any_element()),
            BlockKind::Ordered { seq } => Some(div().min_w(px(18.)).mr(px(6.)).flex_none().text_color(Theme::text_caption()).child(crate::editor::core::ordered_marker(seq.unwrap_or(1), list_depth)).into_any_element()),
            BlockKind::Todo { done } => Some(div().w(px(24.)).h(px(line_height)).flex_none().flex().items_center().child(div().size(px(16.)).rounded(px(4.)).border_1().border_color(if done { Theme::primary_500() } else { Theme::text_disabled() }).bg(if done { Theme::primary_500() } else { Theme::bg_body() }).flex().items_center().justify_center().when(done, |d| d.child(icon("todo-check", 12., gpui::white())))).into_any_element()),
            _ => None,
        };
        let text = if text.runs.iter().any(|r| r.marks.inline.as_ref().is_some_and(|i| i.equation().is_some())) {
            PreviewText { text: text.clone(), runs, size: font_size, line_height, align: b.align, eq_counter: eqnos.get(&b.id).copied().unwrap_or(0) }.into_any_element()
        } else { StyledText::new(text.text.clone()).with_runs(runs).into_any_element() };
        node = node.child(div().flex().items_start().children(marker).child(div().min_w_0().flex_1().map(|d| match b.align { crate::model::Align::Center => d.text_center(), crate::model::Align::Right => d.text_right(), _ => d }).child(text)));
    }
    for (i, child) in b.children.iter().enumerate() {
        let nested = b.kind.nests();
        let gap = if i == 0 { if nested { 8. } else { 0. } } else { gap(&b.children[i - 1].kind).1.max(gap(&child.kind).0) };
        node = node.child(div().min_w_0().when(!matches!(b.kind, BlockKind::Grid), |d| d.mt(px(gap)))
            .when(nested, |d| d.pl(px(25.)))
            .when(matches!(b.kind, BlockKind::Grid), |d| d.flex().w_0().map(|mut d| { d.style().flex_grow = Some(match child.kind { BlockKind::Column { ratio } => ratio, _ => 1. }); d }))
            .child(render_block(child, color, quote || matches!(b.kind, BlockKind::Quote), (cell, list_depth + usize::from(nested)), eqnos)));
    }
    node.into_any_element()
}

