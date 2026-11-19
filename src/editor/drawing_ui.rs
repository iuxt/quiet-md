//! The drawing tools follow Excalidraw's toolbar order and tray layout.
use super::*;
use crate::{editor_ui::icon, ui::TipExt};

// Scene colors from Excalidraw's Open Color palettes. UI colors use Quiet.md's theme.
const STROKES: [&str; 5] = ["#1e1e1e", "#e03131", "#2f9e44", "#1971c2", "#f08c00"];
const FILLS: [&str; 5] = ["transparent", "#ffc9c9", "#b2f2bb", "#a5d8ff", "#ffec99"];
const PAPER: [&str; 5] = ["#ffffff", "#f8f9fa", "#f5faff", "#fffce8", "#fdf8f6"];
const EXTRA: [&str; 10] = ["#ffffff", "#868e96", "#c2255c", "#7048e8", "#3b5bdb", "#0c8599", "#099268", "#66a80f", "#e67700", "#d9480f"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Colors { Stroke, Fill, Paper }

impl DrawingEditor {
    pub(super) fn header(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let title = self.parent.upgrade().map(|ed| crate::model::display_title(&ed.read(cx).core.doc.title.text)).unwrap_or_default();
        let can_save = !self.busy && (self.dirty() || self.text_at.is_some() || self.edit_link.is_some());
        let save = tr("保存", "Save");
        div().id("drawing-header").occlude().h(px(crate::app::HEADER_H)).flex_none().flex().items_center().gap(px(6.))
            .pl(px(theme::TRAFFIC_LIGHTS_W)).pr(px(12.))
            .bg(Theme::window()).border_b_1().border_color(Theme::line_divider())
            .child(icon_button("drawing-back", "chevron-left", tr("返回笔记", "Back to note"), None, false, !self.busy)
                .on_click(cx.listener(|d, _, w, cx| d.cancel(w, cx))))
            .child(div().flex_1().min_w_0().flex().gap(px(6.)).items_center().text_size(px(14.)).font_weight(FontWeight::MEDIUM)
                .child(div().flex_none().child(tr("画板", "Drawing")))
                .when(!title.is_empty(), |d| d.child(div().flex_none().text_color(Theme::text_caption()).child("/")).child(div().min_w_0().truncate().child(ui::ui_text(title)))))
            .children(self.error.map(|msg| div().min_w_0().truncate().text_size(px(12.)).text_color(Theme::text_caption()).child(ui::ui_text(msg))))
            .when(self.busy, |d| d.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(tr("正在处理", "Working…"))))
            .child(div().id("drawing-save").flex_none().h(px(crate::app::HEADER_ACTIONS_H)).px(px(12.)).flex().items_center().rounded(px(8.))
                .text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(if can_save { Theme::text() } else { Theme::text_disabled() })
                .when(can_save, |d| d.bg(Theme::fill_hover()).cursor_pointer().hover(|s| s.bg(Theme::fill_hover_strong()))
                    .active(|s| s.bg(Theme::fill_pressed())).on_click(cx.listener(|d, _, w, cx| d.save(w, cx))))
                .child(save).tip(save, Some(crate::shortcuts::hint("editor::Save", "⌘S"))))
    }

    pub(super) fn canvas_chrome(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut result = Vec::new();
        if !self.zen {
            result.push(div().absolute().top(px(8.)).left_0().right_0().flex().justify_center()
                .child(self.toolbar(cx)).into_any_element());
            result.push(div().absolute().left(px(12.)).bottom(px(11.)).flex().items_end().gap(px(5.))
                .child(self.tray(cx)).child(self.zoom_tray(cx)).into_any_element());
            let max_h = (f32::from(window.viewport_size().height) - 156.).max(165.);
            let overlay = match self.panel {
                Some(Panel::Properties) => Some(self.properties(max_h, cx)),
                Some(Panel::Menu) => Some(self.menu(max_h, cx)),
                Some(Panel::Export) => Some(self.export_menu(cx)),
                _ => None,
            };
            if let Some(overlay) = overlay.filter(|_| !self.busy && self.edit_link.is_none() && self.text_at.is_none()) {
                result.push(div().absolute().left(px(12.)).bottom(px(57.)).child(overlay).into_any_element());
            }
        }
        result.push(div().absolute().right(px(12.)).top(px(62.)).child(ui::popover().id("drawing-viewport-tools").occlude().p(px(3.)).flex().flex_col().gap(px(2.))
            .when(!self.zen, |d| d.child(icon_button("drawing-fit-all", "fit-page", tr("缩放到全部内容", "Zoom to fit"), Some("0"), false, true)
                .on_click(cx.listener(|d, _, _, cx| { d.fit_view(); cx.notify(); }))))
            .child(icon_button("drawing-zen", "panel-left-close", tr("专注模式", "Zen mode"), None, self.zen, true)
                .on_click(cx.listener(|d, _, _, cx| { d.zen = !d.zen; d.panel = None; cx.notify(); }))))
            .into_any_element());
        if self.pending_image.is_some() {
            result.push(div().absolute().top(px(56.)).left_0().right_0().flex().justify_center().text_size(px(12.)).text_color(Theme::text_caption())
                .child(tr("点击画布放置图片", "Click the canvas to place the image")).into_any_element());
        }
        if self.text_at.is_some() || self.edit_link.is_some() {
            result.push(self.field_panel(cx));
        }
        result
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let enabled = !self.busy;
        let mut tools = ui::popover().id("drawing-toolbar").occlude().relative().p(px(4.)).max_w_full().flex().flex_wrap().gap(px(2.))
            .child(icon_button("drawing-lock", if self.tool_locked { "drawing-lock" } else { "drawing-unlock" },
                tr("锁定当前工具", "Keep the selected tool active"), Some("Q"), self.tool_locked, enabled)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.tool_locked = !d.tool_locked; cx.notify(); } })))
            .child(tool_separator());
        for &(tool, zh, en, key, glyph) in TOOLS.iter().filter(|(tool, ..)| *tool != Tool::Frame) {
            let key = key.to_ascii_uppercase();
            tools = tools.child(icon_button(("drawing-tool", tool as usize), glyph, tr(zh, en), Some(&key), self.tool == tool, enabled)
                .when(tool != Tool::Image && tool != Tool::Pan, |d| d.child(div().absolute().bottom(px(1.)).right(px(3.)).text_size(px(8.))
                    .text_color(if self.tool == tool { Theme::drawing_accent() } else { Theme::text_caption() }).child(key)))
                .on_click(cx.listener(move |d, _, w, cx| { if !d.busy { d.tool(tool, w, cx); } })));
        }
        tools.child(tool_separator())
            .child(icon_button("drawing-more-tools", "grip-vertical", tr("更多工具", "More tools"), None, self.panel == Some(Panel::More) || self.tool == Tool::Frame, enabled)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::More, cx))))
            .when(self.panel == Some(Panel::More), |d| d.child(ui::popover().id("drawing-tools-menu").occlude().absolute().right_0().top(px(42.)).w(px(160.)).py(px(5.))
                .child(menu_button("drawing-frame-tool", "drawing-corners", tr("画框", "Frame"), enabled)
                    .on_click(cx.listener(|d, _, w, cx| { if !d.busy { d.tool(Tool::Frame, w, cx); } })))))
    }

    fn tray(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let enabled = !self.busy;
        let has_selection = enabled && self.selected.is_some();
        ui::popover().id("drawing-tray").occlude().flex().items_center().p(px(4.)).gap(px(2.))
            .child(icon_button("drawing-main-menu", "drawing-menu", tr("菜单", "Menu"), None, matches!(self.panel, Some(Panel::Menu | Panel::Export)), enabled)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Menu, cx))))
            .child(icon_button("drawing-props", "palette", tr("形状属性", "Shape properties"), None, self.panel == Some(Panel::Properties), enabled)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Properties, cx))))
            .child(icon_button("drawing-duplicate", "copy", tr("创建副本", "Duplicate"), None, false, has_selection)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.duplicate(cx); } })))
            .child(icon_button("drawing-delete", "trash-2", tr("删除", "Delete"), Some("Delete"), false, has_selection)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.delete(cx); } })))
            .child(icon_button("drawing-undo", "undo", tr("撤销", "Undo"), None, false, enabled && !self.undo.is_empty())
                .on_click(cx.listener(|d, _, w, cx| { if !d.busy { d.finish_field(w, cx); d.history(false, cx); } })))
            .child(icon_button("drawing-redo", "redo", tr("重做", "Redo"), None, false, enabled && !self.redo.is_empty())
                .on_click(cx.listener(|d, _, w, cx| { if !d.busy { d.finish_field(w, cx); d.history(true, cx); } })))
    }

    fn zoom_tray(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        ui::popover().id("drawing-zoom").occlude().p(px(4.)).flex().items_center()
            .child(icon_button("drawing-zoom-out", "minus", tr("缩小", "Zoom out"), None, false, true)
                .on_click(cx.listener(|d, _, _, cx| { d.zoom_at(d.viewport.get().center(), 0.8); cx.notify(); })))
            .child(div().id("drawing-reset-zoom").h(px(31.)).min_w(px(52.)).flex().justify_center().items_center().rounded(px(7.)).text_size(px(13.))
                .cursor_pointer().hover(|s| s.bg(Theme::fill_hover())).child(format!("{:.0}%", self.zoom * 100.))
                .tip_above(tr("重置缩放", "Reset zoom"), None)
                .on_click(cx.listener(|d, _, _, cx| { d.zoom_at(d.viewport.get().center(), 1. / d.zoom); cx.notify(); })))
            .child(icon_button("drawing-zoom-in", "plus", tr("放大", "Zoom in"), None, false, true)
                .on_click(cx.listener(|d, _, _, cx| { d.zoom_at(d.viewport.get().center(), 1.25); cx.notify(); })))
    }

    fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        if self.busy { return; }
        self.panel = (self.panel != Some(panel)).then_some(panel);
        self.colors = None;
        cx.notify();
    }

    fn menu(&self, max_h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let available = !self.busy;
        let can_import = available && !self.dirty() && self.text_at.is_none() && self.edit_link.is_none();
        ui::popover().id("drawing-menu-panel").occlude().w(px(217.)).max_h(px(max_h)).overflow_y_scroll().py(px(7.))
            .child(div().px(px(10.)).pb(px(7.)).child(self.swatches(tr("画布背景", "Canvas background"), Colors::Paper, &self.doc.background, &PAPER, cx)))
            .child(menu_button("drawing-grid", "view-grid", tr("网格模式", "Grid mode"), available).when(self.grid, checked)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.grid = !d.grid; cx.notify(); } })))
            .child(menu_button("drawing-dark", "eye", tr("深色画布", "Dark canvas"), available).when(self.dark_canvas, checked)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.dark_canvas = !d.dark_canvas; cx.notify(); } })))
            .child(horizontal_line())
            .child(menu_button("drawing-menu-save", "drawing-save", tr("保存", "Save"), available)
                .on_click(cx.listener(|d, _, w, cx| d.save(w, cx))))
            .child(menu_button("drawing-menu-import", "import", tr("打开绘图…", "Open drawing…"), can_import)
                .on_click(cx.listener(move |d, _, w, cx| { if can_import { d.import(w, cx); } })))
            .child(menu_button("drawing-menu-export", "export", tr("导出绘图…", "Export drawing…"), available)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Export, cx))))
            .child(horizontal_line())
            .child(menu_button("drawing-reset", "trash-2", tr("重置画布", "Reset canvas"), available && self.doc.scene.non_deleted().next().is_some())
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy && d.doc.scene.non_deleted().next().is_some() { d.reset_confirmation = true; cx.notify(); } })))
    }

    fn export_menu(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut panel = ui::popover().id("drawing-export-panel").occlude().w(px(217.)).py(px(5.))
            .child(menu_button("drawing-export-back", "chevron-left", tr("导出绘图", "Export drawing"), true)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Menu, cx)))).child(horizontal_line());
        for (kind, label) in [(ExportKind::Source, "Excalidraw (.excalidraw)"), (ExportKind::Png, "PNG"), (ExportKind::Svg, "SVG")] {
            panel = panel.child(menu_button(("drawing-export", kind as usize), "export", label, !self.busy)
                .on_click(cx.listener(move |d, _, w, cx| { d.panel = None; d.export(kind, w, cx); cx.notify(); })));
        }
        panel
    }

    fn properties(&self, max_h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let element = self.selected.map(|i| &self.doc.scene.elements[i]);
        let b = element.map(Element::base);
        let tool = element.map_or(self.tool, |e| match e {
            Element::Text(_) => Tool::Text, Element::Line(_) => Tool::Line, Element::Arrow(_) => Tool::Arrow,
            Element::Freedraw(_) => Tool::Pen, Element::Ellipse { .. } => Tool::Ellipse, Element::Image(_) => Tool::Image,
            Element::StickyNote(_) => Tool::Sticky, Element::Frame(_) | Element::MagicFrame(_) => Tool::Frame, _ => Tool::Rect,
        });
        let opts = &self.style;
        let mut body = ui::popover().id("drawing-props-panel").occlude().w(px(214.)).max_h(px(max_h)).overflow_y_scroll().p(px(11.)).flex().flex_col().gap(px(9.));
        if tool != Tool::Image {
            body = body.child(self.swatches(tr("描边", "Stroke"), Colors::Stroke, b.map_or(&opts.stroke_color, |b| &b.stroke_color), &STROKES, cx));
        }
        if matches!(tool, Tool::Select | Tool::Rect | Tool::Diamond | Tool::Ellipse | Tool::Sticky) {
            let background = b.map_or(opts.background_color.as_str(), |b| &b.background_color);
            body = body.child(self.swatches(tr("背景", "Background"), Colors::Fill, background, &FILLS, cx));
            if background != "transparent" {
                let fill = b.map_or(opts.fill_style, |b| b.fill_style);
                body = body.child(options("fill-style", tr("填充", "Fill"), [
                    ("╱", tr("单线", "Hachure"), fill == FillStyle::Hachure, StyleChoice::FillStyle(FillStyle::Hachure)),
                    ("▧", tr("交叉线", "Cross-hatch"), fill == FillStyle::CrossHatch, StyleChoice::FillStyle(FillStyle::CrossHatch)),
                    ("■", tr("实色", "Solid"), fill == FillStyle::Solid, StyleChoice::FillStyle(FillStyle::Solid)),
                    ("⌁", tr("锯齿", "Zigzag"), fill == FillStyle::Zigzag, StyleChoice::FillStyle(FillStyle::Zigzag)),
                ], false, cx));
            }
        }
        if !matches!(tool, Tool::Text | Tool::Image) {
            let width = b.map_or(opts.stroke_width, |b| b.stroke_width);
            let width_buttons = [0.5, 1., 2., 4., 8.].into_iter().enumerate().map(|(i, v)| {
                property_button(("stroke-width", i), tr("描边宽度", "Stroke width"), (width - v).abs() < 0.001, cx, StyleChoice::Width(v))
                    .child(div().w(px(13. + v as f32 * 0.3)).h(px(v as f32)).rounded_full().bg(Theme::text()))
            });
            body = body.child(section(tr("描边宽度", "Stroke width"), div().flex().gap(px(4.)).children(width_buttons)));
        }
        if !matches!(tool, Tool::Text | Tool::Image | Tool::Pen) {
            let style = b.map_or(opts.stroke_style, |b| b.stroke_style);
            body = body.child(options("stroke-style", tr("边框样式", "Stroke style"), [
                ("━", tr("实线", "Solid"), style == StrokeStyle::Solid, StyleChoice::StrokeStyle(StrokeStyle::Solid)),
                ("┄", tr("虚线", "Dashed"), style == StrokeStyle::Dashed, StyleChoice::StrokeStyle(StrokeStyle::Dashed)),
                ("···", tr("点线", "Dotted"), style == StrokeStyle::Dotted, StyleChoice::StrokeStyle(StrokeStyle::Dotted)),
            ], false, cx));
            let rough = b.map_or(opts.roughness, |b| b.roughness);
            body = body.child(options("roughness", tr("线条风格", "Sloppiness"), [
                ("minus", tr("规整", "Architect"), rough == 0., StyleChoice::Roughness(0.)),
                ("drawing-sketch", tr("手绘", "Artist"), rough == 1., StyleChoice::Roughness(1.)),
                ("drawing-rough", tr("随意", "Cartoonist"), rough >= 2., StyleChoice::Roughness(2.)),
            ], true, cx));
        }
        if matches!(tool, Tool::Select | Tool::Rect | Tool::Diamond | Tool::Line | Tool::Arrow | Tool::Sticky) {
            let rounded = b.map_or(self.rounded, |b| b.roundness.is_some());
            body = body.child(options("roundness", tr("边角", "Edges"), [
                ("drawing-corners", tr("直角", "Sharp"), !rounded, StyleChoice::Round(false)),
                ("drawing-rounded", tr("圆角", "Round"), rounded, StyleChoice::Round(true)),
            ], true, cx));
        }
        if tool == Tool::Arrow {
            let arrowhead = match element { Some(Element::Arrow(a)) => a.linear.end_arrowhead, _ => self.arrowhead };
            body = body.child(options("arrowhead", tr("箭头末端", "Arrowhead"), [
                ("—", tr("无", "None"), arrowhead.is_none(), StyleChoice::ArrowHead(None)),
                ("→", tr("箭头", "Arrow"), arrowhead == Some(Arrowhead::Arrow), StyleChoice::ArrowHead(Some(Arrowhead::Arrow))),
                ("▸", tr("三角", "Triangle"), arrowhead == Some(Arrowhead::Triangle), StyleChoice::ArrowHead(Some(Arrowhead::Triangle))),
                ("●", tr("圆点", "Dot"), arrowhead == Some(Arrowhead::Dot), StyleChoice::ArrowHead(Some(Arrowhead::Dot))),
            ], false, cx));
        }
        if tool == Tool::Text {
            let (font_size, align) = match element { Some(Element::Text(t)) => (t.font_size, t.text_align), _ => (self.text_size, self.text_align) };
            body = body.child(options("text-size", tr("字号", "Font size"), [("S", 16.), ("M", 20.), ("L", 28.), ("XL", 36.)].map(|(label, v)| {
                (label, label, font_size == v, StyleChoice::TextSize(v))
            }), false, cx));
            body = body.child(options("text-align", tr("对齐", "Text alignment"), [
                ("align-left", tr("左对齐", "Left"), align == DrawingAlign::Left, StyleChoice::TextAlign(DrawingAlign::Left)),
                ("align-center", tr("居中", "Center"), align == DrawingAlign::Center, StyleChoice::TextAlign(DrawingAlign::Center)),
                ("align-right", tr("右对齐", "Right"), align == DrawingAlign::Right, StyleChoice::TextAlign(DrawingAlign::Right)),
            ], true, cx));
        }
        body = body.child(self.opacity_slider(b.map_or(opts.opacity, |b| b.opacity), cx));
        if element.is_some() {
            let mut layer = div().flex().gap(px(4.));
            for (direction, glyph, label) in [(LayerMove::Bottom, "drawing-layer-down", tr("置于底层", "Send to back")), (LayerMove::Back, "drawing-down", tr("下移一层", "Send backward")),
                (LayerMove::Forward, "drawing-up", tr("上移一层", "Bring forward")), (LayerMove::Top, "drawing-layer-up", tr("置于顶层", "Bring to front"))] {
                layer = layer.child(icon_button(("drawing-layer", direction as usize), glyph, label, None, false, true).bg(Theme::bg_overlay())
                    .on_click(cx.listener(move |d, _, _, cx| d.reorder(direction, cx))));
            }
            body = body.child(section(tr("图层", "Layers"), layer))
                .child(section(tr("操作", "Actions"), div().flex().gap(px(4.))
                    .child(icon_button("prop-copy", "copy", tr("创建副本", "Duplicate"), None, false, true).bg(Theme::bg_overlay()).on_click(cx.listener(|d, _, _, cx| d.duplicate(cx))))
                    .child(icon_button("prop-delete", "trash-2", tr("删除", "Delete"), None, false, true).bg(Theme::bg_overlay()).on_click(cx.listener(|d, _, _, cx| d.delete(cx))))
                    .child(icon_button("prop-link", "link", tr("链接", "Link"), None, b.is_some_and(|b| b.link.is_some()), true).bg(Theme::bg_overlay()).on_click(cx.listener(|d, _, w, cx| d.open_link_editor(w, cx))))));
        }
        body
    }

    fn swatches(&self, label: &'static str, target: Colors, current: &str, colors: &[&'static str], cx: &mut Context<Self>) -> Div {
        let s = target as usize;
        let chooser = div().id(("drawing-color-more", s)).size(px(25.)).flex().items_center().justify_center().rounded(px(5.)).ml(px(4.)).border_1()
            .border_color(if self.colors == Some(target) { Theme::drawing_accent() } else { Theme::line_border() }).cursor_pointer().child(swatch_fill(current, 20.))
            .tip(tr("更多颜色", "More colors"), None)
            .on_click(cx.listener(move |d, _, _, cx| { d.colors = (d.colors != Some(target)).then_some(target); cx.notify(); }));
        let buttons = |prefix, palette: &[&'static str]| palette.iter().copied().enumerate().map(|(i, color)| {
            div().id((prefix, s * 100 + i)).size(px(25.)).flex().items_center().justify_center().rounded(px(5.)).border_1().cursor_pointer()
                .border_color(if current.eq_ignore_ascii_case(color) { Theme::drawing_accent() } else { gpui::transparent_black() }).child(swatch_fill(color, 21.)).tip(color, None)
                .on_click(cx.listener(move |d, _, _, cx| match target {
                    Colors::Stroke => d.apply_style(StyleChoice::Stroke(color), true, cx),
                    Colors::Fill => d.apply_style(StyleChoice::Fill(color), true, cx),
                    Colors::Paper => d.change_paper(color, cx),
                }))
        }).collect::<Vec<_>>();
        section(label, div().flex().flex_col().gap(px(4.))
            .child(div().flex().gap(px(3.)).children(buttons("drawing-color", colors)).child(chooser))
            .when(self.colors == Some(target), |d| d.child(div().w(px(148.)).flex().flex_wrap().gap(px(3.)).children(buttons("drawing-extra-color", &EXTRA)))))
    }

    fn opacity_slider(&self, opacity: f64, cx: &mut Context<Self>) -> Div {
        let bounds = self.opacity_bounds.clone();
        section(tr("透明度", "Opacity"), div().flex().flex_col().gap(px(3.))
            .child(div().id("drawing-opacity").relative().w_full().h(px(19.)).flex().items_center().cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(|d, ev: &MouseDownEvent, _, cx| {
                    d.opacity_dragging = Some(false); d.set_opacity(ev.position.x, cx); cx.stop_propagation();
                }))
                .on_mouse_move(cx.listener(|d, ev: &MouseMoveEvent, _, cx| {
                    if ev.pressed_button == Some(MouseButton::Left) { d.set_opacity(ev.position.x, cx); }
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|d, _, _, _| d.opacity_dragging = None))
                .on_mouse_up_out(MouseButton::Left, cx.listener(|d, _, _, _| d.opacity_dragging = None))
                .child(div().absolute().left_0().right_0().h(px(3.)).rounded_full().bg(Theme::line_divider())
                    .child(div().h_full().w(relative((opacity / 100.) as f32)).rounded_full().bg(Theme::drawing_accent().opacity(0.38))))
                .child(canvas(move |b, _, _| bounds.set(b), move |b, _, window, _| {
                    let x = b.left() + px(7.) + (b.size.width - px(14.)) * ((opacity / 100.).clamp(0., 1.) as f32);
                    let knob = Bounds::new(point(x - px(7.), b.center().y - px(7.)), size(px(14.), px(14.)));
                    window.paint_quad(quad(knob, px(7.), Theme::text(), Edges::all(px(0.)), transparent_black(), BorderStyle::Solid));
                }).size_full()))
            .child(div().flex().justify_between().text_size(px(10.)).text_color(Theme::text_caption()).child("0").child("100")))
    }

    fn set_opacity(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(has_checkpoint) = self.opacity_dragging else { return };
        let track = self.opacity_bounds.get();
        if track.size.width <= px(14.) { return; }
        let value = (100. * f32::from(x - track.left() - px(7.)) / f32::from(track.size.width - px(14.))).round().clamp(0., 100.) as f64;
        let current = self.selected.map_or(self.style.opacity, |i| self.doc.scene.elements[i].base().opacity);
        if current != value {
            self.apply_style(StyleChoice::Opacity(value), !has_checkpoint, cx);
            self.opacity_dragging = Some(true);
        }
    }

    fn field_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.edit_link.is_some() { tr("链接", "Link") } else { tr("文字", "Text") };
        let view = self.viewport.get();
        let at = self.text_at.map(|(p, _)| self.offset + point(px((p.x * self.zoom) as f32), px((p.y * self.zoom) as f32)));
        let (x, y) = at.map_or((px(14.), px(62.)), |at| (at.x.clamp(px(14.), (view.size.width - px(265.)).max(px(14.))), at.y.clamp(px(52.), (view.size.height - px(60.)).max(px(52.)))));
        ui::popover().id("drawing-field").occlude().absolute().top(y).left(x).w(px(250.)).p(px(6.)).flex().items_center().gap(px(5.))
            .child(div().flex_1().min_w_0().px(px(7.)).py(px(4.)).rounded(px(6.)).bg(Theme::bg_overlay()).child(self.field.clone()))
            .child(icon_button("drawing-text-done", "check", label, Some("Enter"), true, true)
                .on_click(cx.listener(|d, _, w, cx| d.finish_field(w, cx)))).into_any_element()
    }

    pub(super) fn confirmation(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let reset = self.reset_confirmation;
        div().id("drawing-confirm-scrim").absolute().inset_0().occlude().bg(ui::scrim()).flex().justify_center().items_center()
            .child(ui::dialog("drawing-confirm").p(px(20.)).flex().flex_col().gap(px(16.))
                .child(if reset { tr("清空画布上的内容？", "Clear the canvas?") } else { tr("放弃未保存的画板修改？", "Discard unsaved drawing changes?") })
                .child(div().flex().justify_end().gap(px(8.))
                    .child(ui::secondary_button("drawing-keep", "取消").on_click(cx.listener(move |d, _, w, cx| {
                        if reset { d.reset_confirmation = false; cx.notify(); } else { d.keep_editing(w, cx); }
                    })))
                    .child(ui::primary_button("drawing-confirm-action", if reset { "重置画布" } else { "放弃修改" })
                        .on_click(cx.listener(move |d, _, w, cx| { if reset { d.reset_canvas(cx); } else { d.close(w, cx); } })))))
    }
}

fn icon_button(id: impl Into<ElementId>, glyph: &str, label: &str, key: Option<&str>, active: bool, enabled: bool) -> Stateful<Div> {
    let color = if !enabled { Theme::text_disabled() } else if active { Theme::drawing_accent() } else { Theme::text() };
    div().id(id).relative().flex_none().size(px(32.)).flex().items_center().justify_center().rounded(px(8.))
        .when(active, |d| d.bg(Theme::drawing_selection())).when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(Theme::fill_hover())))
        .child(icon(glyph, 17., color)).tip(label, key)
}

fn menu_button(id: impl Into<ElementId>, glyph: &str, label: &'static str, enabled: bool) -> Stateful<Div> {
    let color = if enabled { Theme::text() } else { Theme::text_disabled() };
    ui::menu_row(id, "drawing-menu".into()).cursor(if enabled { CursorStyle::PointingHand } else { CursorStyle::Arrow }).text_color(color)
        .child(icon(glyph, 16., color)).child(div().flex_1().child(label))
}

fn checked(row: Stateful<Div>) -> Stateful<Div> { row.child(icon("check", 14., Theme::drawing_accent())) }
fn horizontal_line() -> Div { div().mx(px(8.)).my(px(3.)).h(px(1.)).bg(Theme::line_divider()) }
fn tool_separator() -> Div { div().mx(px(3.)).my(px(4.)).w(px(1.)).h(px(24.)).bg(Theme::line_divider()) }

fn section(label: &'static str, content: Div) -> Div {
    div().flex_none().flex().flex_col().gap(px(4.)).child(div().text_size(px(12.)).text_color(Theme::text()).child(label)).child(content)
}

fn options<const N: usize>(id: &'static str, label: &'static str, items: [(&'static str, &'static str, bool, StyleChoice); N], icons: bool, cx: &mut Context<DrawingEditor>) -> Div {
    let buttons = items.into_iter().enumerate().map(|(i, (glyph, tip, selected, choice))| {
        property_button((id, i), tip, selected, cx, choice)
            .when(icons, |d| d.child(icon(glyph, 16., Theme::text())))
            .when(!icons, |d| d.child(glyph))
    });
    section(label, div().flex().gap(px(4.)).children(buttons))
}

fn property_button(id: impl Into<ElementId>, tip: &str, selected: bool, cx: &mut Context<DrawingEditor>, choice: StyleChoice) -> Stateful<Div> {
    div().id(id).size(px(32.)).flex_none().flex().items_center().justify_center().rounded(px(8.)).cursor_pointer().text_size(px(13.))
        .bg(if selected { Theme::drawing_selection() } else { Theme::bg_overlay() }).hover(|s| s.bg(Theme::fill_hover())).tip(tip, None)
        .on_click(cx.listener(move |d, _, _, cx| d.apply_style(choice, true, cx)))
}

fn swatch_fill(value: &str, extent: f32) -> AnyElement {
    let tile = div().size(px(extent)).rounded(px(4.)).overflow_hidden();
    if value == "transparent" {
        tile.bg(Theme::bg_body()).child(canvas(|_, _, _| (), move |b, _, w, _| {
            let side = b.size.width / 3.;
            for y in 0..3 { for x in 0..3 {
                if (x + y) % 2 == 0 {
                    w.paint_quad(fill(Bounds::new(b.origin + point(side * x as f32, side * y as f32), size(side, side)), Theme::line_divider()));
                }
            } }
        }).size_full()).into_any_element()
    } else { tile.bg(hsla_from_ink(Ink::from_hex(value), 1.)).into_any_element() }
}
