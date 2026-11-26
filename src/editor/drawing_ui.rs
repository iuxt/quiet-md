//! The drawing tools follow Excalidraw's toolbar order and tray layout.
use super::*;
use crate::{editor_ui::icon, ui::TipExt};

// Scene colors from Excalidraw's Open Color palettes. UI colors use Quiet.md's theme.
const STROKES: [&str; 5] = ["#1e1e1e", "#e03131", "#2f9e44", "#1971c2", "#f08c00"];
const FILLS: [&str; 5] = ["transparent", "#ffc9c9", "#b2f2bb", "#a5d8ff", "#ffec99"];
const PAPER: [&str; 5] = ["#ffffff", "#f8f9fa", "#f5faff", "#fffce8", "#fdf8f6"];
const EXTRA: [&str; 10] = ["#ffffff", "#868e96", "#c2255c", "#7048e8", "#3b5bdb", "#0c8599", "#099268", "#66a80f", "#e67700", "#d9480f"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Colors { Stroke, Fill, Paper, Text }

const PROPERTIES_W: f32 = 214.;
const MENU_W: f32 = 217.;
const LIBRARY_W: f32 = 260.;
const FIELD_W: f32 = 250.;

pub(super) struct ExportPreview { pub image: Arc<RenderImage>, revision: u64, selection: Vec<usize>, dark: bool, background: bool, show_frames: bool }

impl DrawingEditor {
    pub(super) fn header(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let title = self.parent.upgrade().map(|ed| crate::model::display_title(&ed.read(cx).core.doc.title.text)).unwrap_or_default();
        let can_save = !self.busy && (self.dirty() || self.mermaid || self.text_at.is_some() || self.edit_link.is_some() || self.edit_frame.is_some() || self.edit_property.is_some() || !self.polyline.is_empty());
        let save = tr("保存", "Save");
        div().id("drawing-header").occlude().h(px(crate::app::HEADER_H)).flex_none().flex().items_center().gap(px(6.))
            .pl(px(theme::TRAFFIC_LIGHTS_W)).pr(px(12.))
            .bg(Theme::window()).border_b_1().border_color(Theme::line_divider())
            .child(icon_button("drawing-back", "chevron-left", tr("返回笔记", "Back to note"), None, false, !self.busy)
                .on_click(cx.listener(|d, _, w, cx| d.cancel(w, cx))))
            .child(div().flex_1().min_w_0().flex().gap(px(6.)).items_center().text_size(px(14.)).font_weight(FontWeight::MEDIUM)
                .child(div().flex_none().child(tr("画板", "Drawing")))
                .when(!title.is_empty(), |d| d.child(div().flex_none().text_color(Theme::text_caption()).child("/")).child(div().min_w_0().truncate().child(ui::ui_text(title)))))
            .children(self.error.map(|msg| div().id("drawing-error").min_w_0().truncate().text_size(px(12.)).text_color(Theme::text_caption()).child(ui::ui_text(msg)).tip(msg, None)))
            .when(self.busy, |d| d.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(tr("正在处理", "Working…"))))
            .child(div().id("drawing-save").flex_none().h(px(crate::app::HEADER_ACTIONS_H)).px(px(12.)).flex().items_center().rounded(px(8.))
                .text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(if can_save { Theme::text() } else { Theme::text_disabled() })
                .when(can_save, |d| d.bg(Theme::fill_hover()).cursor_pointer().hover(|s| s.bg(Theme::fill_hover_strong()))
                    .active(|s| s.bg(Theme::fill_pressed())).on_click(cx.listener(|d, _, w, cx| d.save(w, cx))))
                .child(save).tip(save, Some(crate::shortcuts::hint("editor::Save", "⌘S"))))
    }

    pub(super) fn canvas_chrome(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut result = Vec::new();
        let field_open = self.text_at.is_some() || self.edit_link.is_some() || self.edit_frame.is_some() || self.edit_property.is_some() || self.searching || self.mermaid;
        let fixed_field = self.searching || self.mermaid || self.edit_frame.is_some();
        let stack_field = fixed_field && !self.zen && window.viewport_size().width < px(600.);
        if !self.zen {
            let left = if fixed_field && !stack_field { FIELD_W + 22. }
                else if field_open { 60. }
                else { match self.panel {
                    Some(Panel::Properties) => PROPERTIES_W + 20.,
                    Some(Panel::Library) => LIBRARY_W + 20.,
                    Some(Panel::Menu | Panel::Export) => MENU_W + 20.,
                    _ => 60.,
                }};
            let right = if f32::from(window.viewport_size().width) - 2. * left >= 440. { left } else { 60. };
            if !self.view_only { result.push(div().absolute().top(px(8.)).left_0().right_0().flex().flex_col().items_center().gap(px(8.))
                .child(self.toolbar(cx)).children(self.canvas_hint().filter(|_| self.show_hints).map(|hint|
                    div().w_full().pl(px(left)).pr(px(right)).flex().justify_center().child(hint)))
                .when(stack_field, |d| d.child(div().w_full().pl(px(14.)).child(self.field_panel(cx, false)))).into_any_element()); }
            result.push(div().absolute().left(px(12.)).bottom(px(11.)).flex().items_end().gap(px(5.))
                .child(self.tray(cx)).child(self.zoom_tray(cx)).into_any_element());
            let max_h = (f32::from(window.viewport_size().height) - 156.).max(165.);
            let overlay = match self.panel {
                Some(Panel::Properties) => Some(self.properties(max_h, cx)),
                Some(Panel::Menu) => Some(self.menu(max_h, cx)),
                Some(Panel::Convert) => Some(self.conversion_menu(cx)),
                Some(Panel::Export) => Some(self.export_menu(window,cx)),
                Some(Panel::Help) => Some(self.help_menu(max_h)),
                Some(Panel::Library) => Some(self.library_panel(max_h, cx)),
                Some(Panel::Chart) => self.chart.as_ref().map(|(data,_,_)| {
                    let mut menu = ui::popover().id("chart-paste").occlude().w(px(MENU_W)).p(px(6.));
                    for (i,(kind,label)) in [(Some(chart::Chart::Bar),tr("柱状图","Bar chart")), (Some(chart::Chart::Line),tr("折线图","Line chart")),
                        (Some(chart::Chart::Radar),tr("雷达图","Radar chart")), (None,tr("粘贴为文字","Paste as text"))].into_iter().enumerate() {
                        if kind == Some(chart::Chart::Radar) && data.labels.len()<3 { continue; }
                        menu = menu.child(menu_button(("paste-chart",i),"plus",label,true).on_click(cx.listener(move |d,_,w,cx| {
                            let Some((data,text,at)) = d.chart.take() else { return }; d.panel = None;
                            if let Some(kind) = kind { d.insert_scene(data.draw(kind),at,w,cx); } else { d.start_text(at,None,text,w,cx); } cx.notify();
                        })));
                    }
                    menu
                }),
                _ => None,
            };
            if let Some(overlay) = overlay.filter(|_| !self.busy && !field_open) {
                result.push(div().absolute().left(px(12.)).bottom(px(57.)).child(overlay).into_any_element());
            }
        }
        if !self.zen && self.minimap { result.push(self.minimap_panel(cx)); }
        if !self.zen && self.stats {
            let count = self.doc.scene.non_deleted().count();
            let selected = self.selection.len();
            let bounds = selection_bounds(&self.doc.scene, &self.selection).or_else(|| drawing_scene::bounds(&self.doc.scene));
            let panel = ui::popover().id("drawing-stats").occlude().p(px(10.)).text_size(px(12.)).flex().flex_col().gap(px(4.))
                .child(format!("{}  {}", tr("元素", "Elements"), count)).child(format!("{}  {}", tr("已选", "Selected"), selected))
                .children(bounds.map(|b| format!("{:.0} × {:.0} px", b.width(), b.height())));
            result.push(div().absolute().right(px(12.)).bottom(px(if self.minimap { 210. } else { 57. })).child(panel).into_any_element());
        }
        if let Some(current) = self.palette {
            let matches = self.palette_items(cx);
            let first = current.saturating_sub(6);
            let menu = ui::popover().id("drawing-palette").occlude().w(px(320.)).max_w_full().p(px(8.))
                .child(div().mb(px(6.)).p(px(6.)).child(self.field.clone()))
                .when(matches.is_empty(), |p| p.child(tr("没有匹配的命令", "No matching commands")))
                .children(matches.into_iter().enumerate().skip(first).take(12).map(|(i,(cmd,label))|
                    menu_button(("drawing-command",i), "chevron-right", label,true).when(i == current, |d| d.bg(Theme::fill_hover()))
                        .on_click(cx.listener(move |d,_,w,cx| d.run_command(cmd,w,cx)))));
            result.push(div().absolute().left_0().right_0().top(px(65.)).flex().justify_center().child(menu).into_any_element());
        }
        if self.panel == Some(Panel::Context) {
            let view = self.viewport.get();
            let at = self.offset + point(px((self.pointer.x * self.zoom) as f32), px((self.pointer.y * self.zoom) as f32));
            result.push(div().absolute().left(at.x.clamp(px(12.), (view.size.width - px(245.)).max(px(12.))))
                .top(at.y.clamp(px(12.), (view.size.height - px(540.)).max(px(12.)))).child(self.context_menu(cx)).into_any_element());
        }
        result.push(div().absolute().right(px(12.)).top(px(62.)).child(ui::popover().id("drawing-viewport-tools").occlude().p(px(3.)).flex().flex_col().gap(px(2.))
            .when(!self.zen, |d| d.child(icon_button("drawing-fit-all", "fit-page", tr("缩放到全部内容", "Zoom to fit"), Some("Shift+1"), false, true)
                .on_click(cx.listener(|d, _, _, cx| { d.fit_view(); cx.notify(); }))))
            .child(icon_button("drawing-zen", "panel-left-close", tr("专注模式", "Zen mode"), None, self.zen, true)
                .on_click(cx.listener(|d, _, _, cx| { d.zen = !d.zen; d.panel = None; cx.notify(); }))))
            .into_any_element());
        if field_open && !stack_field {
            result.push(self.field_panel(cx, true));
        }
        result
    }

    fn minimap_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = self.viewport.get();
        let a = self.world(view.origin); let b = self.world(view.bottom_right());
        let visible = scene::bounds::Bounds::new(a.x, a.y, b.x, b.y);
        let extent = quiet_excalidraw::render::export::visible_bounds(&self.doc.scene, self.export_options.show_frames).map_or(visible, |r| r.union(&visible));
        let scale = (180. / extent.width().max(1.)).min(118. / extent.height().max(1.));
        let center = extent.center(); let art = self.artwork.clone(); let images = self.images.clone();
        let show_frames = self.export_options.show_frames;
        let geometry: Vec<_> = self.doc.scene.elements.iter().map(|e| {
            let hidden = !show_frames && matches!(e, Element::Frame(_) | Element::MagicFrame(_));
            let clip = if show_frames { e.base().frame_id.as_deref().and_then(|id| self.doc.scene.get(id)).map(element_bounds) } else { None };
            (hidden, element_center(e), e.base().angle, clip)
        }).collect();
        let dark = self.dark_canvas;
        let area = Rc::new(Cell::new(Point::<Pixels>::default())); let paint_area = area.clone();
        let content = canvas(move |bounds, _, _| paint_area.set(bounds.center()), move |bounds, (), w, cx| {
            let origin = bounds.center() - point(px((center.x * scale) as f32), px((center.y * scale) as f32));
            let screen = |p: World| origin + point(px((p.x * scale) as f32), px((p.y * scale) as f32));
            for (artwork, (hidden, pivot, angle, clip)) in art.iter().zip(geometry) {
                if hidden { continue; }
                let mask = clip.map(|r| ContentMask { bounds: Bounds::from_corners(
                    screen(World::new(r.min_x,r.min_y)), screen(World::new(r.max_x,r.max_y))) });
                w.with_content_mask(mask, |w| {
                    paint_artwork(artwork, origin, scale, pivot, angle, dark, &images, w, cx);
                });
            }
            w.paint_quad(outline(Bounds::from_corners(screen(a), screen(b)), Theme::drawing_selection(), BorderStyle::Solid));
        }).size_full();
        div().absolute().bottom(px(57.)).right(px(12.)).child(ui::popover().id("drawing-minimap").occlude()
            .w(px(196.)).h(px(134.)).overflow_hidden().cursor_pointer().child(content)
            .on_mouse_down(MouseButton::Left, cx.listener(move |d, e: &MouseDownEvent, _, cx| {
                d.map_drag = Some((center, area.get(), scale)); d.pan_minimap(e.position); cx.stop_propagation(); cx.notify();
            })).on_mouse_move(cx.listener(|d, e: &MouseMoveEvent, _, cx| {
                if d.map_drag.is_some() && e.pressed_button == Some(MouseButton::Left) { d.pan_minimap(e.position); cx.notify(); }
            })).on_mouse_up(MouseButton::Left, cx.listener(|d, _, _, _| d.map_drag = None))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|d, _, _, _| d.map_drag = None))).into_any_element()
    }

    fn pan_minimap(&mut self, p: Point<Pixels>) {
        let Some((center, origin, scale)) = self.map_drag else { return };
        let delta = p - origin; let at = center + World::new(f64::from(f32::from(delta.x))/scale, f64::from(f32::from(delta.y))/scale);
        let view = self.viewport.get(); self.fit = false;
        self.offset = view.center() - view.origin - point(px((at.x*self.zoom) as f32), px((at.y*self.zoom) as f32));
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let enabled = !self.busy;
        let mut tools = ui::popover().id("drawing-toolbar").occlude().relative().p(px(4.)).max_w_full().flex().flex_wrap().gap(px(2.))
            .child(icon_button("drawing-lock", if self.tool_locked { "drawing-lock" } else { "drawing-unlock" },
                tr("锁定当前工具", "Keep the selected tool active"), Some("Q"), self.tool_locked, enabled)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.tool_locked = !d.tool_locked; cx.notify(); } })))
            .child(tool_separator());
        for &(tool, zh, en, key, glyph) in TOOLS.iter().filter(|(tool, ..)| !matches!(tool, Tool::Frame | Tool::Laser | Tool::Eyedropper | Tool::Bucket | Tool::Lasso | Tool::AutoShape)) {
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
                .children(TOOLS.iter().filter(|(tool, ..)| matches!(tool, Tool::Frame | Tool::Laser | Tool::Eyedropper | Tool::Bucket | Tool::Lasso | Tool::AutoShape)).map(|&(tool, zh, en, _, glyph)| {
                    menu_button(("drawing-extra-tool", tool as usize), glyph, tr(zh, en), enabled)
                        .on_click(cx.listener(move |d, _, w, cx| { if !d.busy { d.tool(tool, w, cx); } }))
                }))))
    }

    fn tray(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let enabled = !self.busy;
        let has_selection = enabled && !self.selection.is_empty();
        ui::popover().id("drawing-tray").occlude().flex().items_center().p(px(4.)).gap(px(2.))
            .child(icon_button("drawing-main-menu", "drawing-menu", tr("菜单", "Menu"), None, matches!(self.panel, Some(Panel::Menu | Panel::Export)), enabled)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Menu, cx))))
            .when(!self.view_only, |tray| tray.child(icon_button("drawing-props", "palette", tr("形状属性", "Shape properties"), None, self.panel == Some(Panel::Properties), enabled)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Properties, cx))))
            .child(icon_button("drawing-duplicate", "copy", tr("创建副本", "Duplicate"), None, false, has_selection)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.duplicate(cx); } })))
            .child(icon_button("drawing-delete", "trash-2", tr("删除", "Delete"), Some("Delete"), false, has_selection)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.delete(cx); } })))
            .child(icon_button("drawing-undo", "undo", tr("撤销", "Undo"), None, false, enabled && !self.undo.is_empty())
                .on_click(cx.listener(|d, _, w, cx| { if !d.busy { d.finish_field(w, cx); d.history(false, cx); } })))
            .child(icon_button("drawing-redo", "redo", tr("重做", "Redo"), None, false, enabled && !self.redo.is_empty())
                .on_click(cx.listener(|d, _, w, cx| { if !d.busy { d.finish_field(w, cx); d.history(true, cx); } }))))
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
        self.more_fonts = false;
        cx.notify();
    }

    fn menu(&self, max_h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let available = !self.busy;
        let can_import = available && !self.view_only && self.text_at.is_none() && self.edit_link.is_none() && self.edit_frame.is_none() && self.edit_property.is_none() && self.polyline.is_empty() && self.gesture.is_none();
        ui::popover().id("drawing-menu-panel").occlude().w(px(MENU_W)).max_h(px(max_h)).overflow_y_scroll().py(px(7.))
            .when(!self.view_only, |d| d.child(div().px(px(10.)).pb(px(7.)).child(self.swatches(tr("画布背景", "Canvas background"), Colors::Paper, &self.doc.background, &PAPER, cx))))
            .child(menu_button("drawing-commands", "search", tr("命令面板", "Command palette"), available)
                .on_click(cx.listener(|d,_,w,cx| d.open_palette(w,cx))))
            .child(menu_button("drawing-overlap", "drawing-pointer", tr("相交即选中", "Select on overlap"), available).when(self.box_overlap, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.box_overlap = !d.box_overlap; cx.notify(); })))
            .child(menu_button("drawing-bind-arrows", "drawing-arrow", tr("箭头绑定", "Arrow binding"), available).when(self.bind_arrows, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.bind_arrows = !d.bind_arrows; cx.notify(); })))
            .child(menu_button("drawing-minimap-toggle", "fit-page", tr("小地图", "Minimap"), available).when(self.minimap, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.minimap = !d.minimap; d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-arrow-midpoints", "drawing-arrow", tr("箭头吸附到边中点", "Snap arrows to midpoints"), available)
                .when(self.snap_arrow_midpoints, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.snap_arrow_midpoints = !d.snap_arrow_midpoints; cx.notify(); })))
            .child(menu_button("drawing-show-frames", "drawing-corners", tr("显示画框", "Show frames"), available).when(self.export_options.show_frames, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.export_options.show_frames = !d.export_options.show_frames; cx.notify(); })))
            .child(menu_button("drawing-stats-toggle", "view-grid", tr("画板统计", "Canvas statistics"), available).when(self.stats, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.stats = !d.stats; d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-hints", "keyboard", tr("工具提示", "Toolbar hints"), available).when(self.show_hints, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.show_hints = !d.show_hints; cx.notify(); })))
            .when(!self.view_only, |d| d.child(menu_button("drawing-mermaid", "code", "Mermaid", available)
                .on_click(cx.listener(|d, _, w, cx| {
                    d.panel = None; d.run_command(palette::Command::Mermaid,w,cx);
                }))))
            .child(menu_button("drawing-grid", "view-grid", tr("网格模式", "Grid mode"), available).when(self.grid, checked)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.toggle_grid(cx); } })))
            .child(menu_button("drawing-snap", "drawing-corners", tr("对象吸附", "Snap to objects"), available).when(self.objects_snap, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.objects_snap = !d.objects_snap; cx.notify(); })))
            .child(menu_button("drawing-view", "eye", tr("只读浏览", "View mode"), available).when(self.view_only, checked)
                .on_click(cx.listener(|d, _, _, cx| { d.view_only = !d.view_only; d.selection.clear(); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-dark", "eye", tr("深色画布", "Dark canvas"), available).when(self.dark_canvas, checked)
                .on_click(cx.listener(|d, _, _, cx| { if !d.busy { d.dark_canvas = !d.dark_canvas; cx.notify(); } })))
            .child(horizontal_line())
            .child(menu_button("drawing-menu-save", "drawing-save", tr("保存", "Save"), available)
                .on_click(cx.listener(|d, _, w, cx| d.save(w, cx))))
            .child(menu_button("drawing-menu-import", "import", tr("打开绘图…", "Open drawing…"), can_import)
                .on_click(cx.listener(move |d, _, w, cx| { if can_import { d.import(w, cx); } })))
            .child(menu_button("drawing-menu-export", "export", tr("导出绘图…", "Export drawing…"), available)
                .on_click(cx.listener(|d, _, _, cx| d.open_export(cx))))
            .child(menu_button("drawing-library", "drawing-rectangle", tr("素材库", "Library"), available)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Library, cx))))
            .child(menu_button("drawing-find", "search", tr("查找画板文字", "Find text on canvas"), available)
                .on_click(cx.listener(|d, _, w, cx| d.start_find(w, cx))))
            .child(menu_button("drawing-unlock-all", "drawing-unlock", tr("解锁全部", "Unlock all"), available)
                .on_click(cx.listener(|d, _, _, cx| { if d.editable() { d.selection.clear(); d.lock_selection(cx); } })))
            .child(menu_button("drawing-help", "keyboard", tr("快捷键", "Keyboard shortcuts"), available)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Help, cx))))
            .child(horizontal_line())
            .child(menu_button("drawing-reset", "trash-2", tr("重置画布", "Reset canvas"), available && self.doc.scene.non_deleted().next().is_some())
                .on_click(cx.listener(|d, _, _, cx| { if d.editable() && d.doc.scene.non_deleted().next().is_some() { d.reset_confirmation = true; cx.notify(); } })))
    }

    fn export_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let selection = if self.export_selection { self.selection.as_slice() } else { &[] };
        let opts = self.export_options; let mut cache = self.export_preview.borrow_mut();
        if cache.as_ref().is_none_or(|p| p.revision != self.revision || p.dark!=opts.dark || p.background!=opts.background || p.show_frames!=opts.show_frames || p.selection!=selection) {
            if let Some(p) = cache.take() { let _ = window.drop_image(p.image); }
            let doc = if selection.is_empty() { &self.doc } else { &self.selection_export_document() };
            *cache = doc.export_thumbnail(opts).map(|image| ExportPreview { image, selection: selection.to_vec(), revision:self.revision, dark:opts.dark, background:opts.background, show_frames:opts.show_frames });
        }
        let image = cache.as_ref().map(|p| p.image.clone());
        let mut panel = ui::popover().id("drawing-export-panel").occlude().w(px(MENU_W)).py(px(5.))
            .child(menu_button("drawing-export-back", "chevron-left", tr("导出绘图", "Export drawing"), true)
                .on_click(cx.listener(|d, _, _, cx| d.toggle_panel(Panel::Menu, cx)))).child(horizontal_line());
        if let Some(image) = image {
            panel = panel.child(canvas(|_,_,_| (), move |b,_,w,_| {
                let sz = image.size(0); let scale = (f32::from(b.size.width)/sz.width.0.max(1) as f32).min(f32::from(b.size.height)/sz.height.0.max(1) as f32);
                let sz = size(px(sz.width.0 as f32*scale),px(sz.height.0 as f32*scale));
                let _ = w.paint_image(Bounds::new(b.center()-point(sz.width/2.,sz.height/2.),sz),Corners::default(),image,0,false);
            }).w_full().h(px(78.)));
        }
        if !self.selection.is_empty() {
            panel = panel.child(menu_button("drawing-export-selection", "check", tr("图片仅导出选区", "Export selection only as image"), true)
                .when(self.export_selection, checked).on_click(cx.listener(|d, _, _, cx| { d.export_selection = !d.export_selection; cx.notify(); })));
        }
        for (n, label, enabled) in [(0, tr("包含背景", "Background"), self.export_options.background), (1, tr("深色模式", "Dark mode"), self.export_options.dark), (2, tr("嵌入可编辑图源", "Embed drawing source"), self.export_options.embed)] {
            panel = panel.child(menu_button(("drawing-export-option", n as usize), "check", label, true).when(enabled, checked)
                .on_click(cx.listener(move |d, _, _, cx| {
                    match n { 0 => d.export_options.background = !d.export_options.background, 1 => d.export_options.dark = !d.export_options.dark, _ => d.export_options.embed = !d.export_options.embed }
                    cx.notify();
                })));
        }
        panel = panel.child(div().px(px(12.)).py(px(7.)).flex().items_center().justify_between().text_size(px(12.))
            .child(tr("PNG 倍率", "PNG scale"))
            .children([1, 2, 3].map(|n| div().id(("drawing-export-scale", n as usize)).px(px(7.)).py(px(4.)).rounded(px(5.)).cursor_pointer()
                .bg(if self.export_options.scale == n as f32 { Theme::drawing_selection() } else { Theme::bg_overlay() }).child(format!("{n}×"))
                .on_click(cx.listener(move |d, _, _, cx| { d.export_options.scale = n as f32; cx.notify(); })))))
            .child(horizontal_line());
        for (kind, label) in [(ExportKind::Source, "Excalidraw (.excalidraw)"), (ExportKind::Png, "PNG"), (ExportKind::Svg, "SVG")] {
            panel = panel.child(menu_button(("drawing-export", kind as usize), "export", label, !self.busy)
                .on_click(cx.listener(move |d, _, w, cx| { d.panel = None; d.export(kind, w, cx); cx.notify(); })));
        }
        panel
    }

    fn properties(&self, max_h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let element = self.selected().map(|i| &self.doc.scene.elements[i]);
        let b = element.map(Element::base);
        let tool = element.map_or(self.tool, |e| match e {
            Element::Text(_) => Tool::Text, Element::Line(_) => Tool::Line, Element::Arrow(_) => Tool::Arrow,
            Element::Freedraw(_) => Tool::Pen, Element::Ellipse { .. } => Tool::Ellipse, Element::Image(_) => Tool::Image,
            Element::StickyNote(_) => Tool::Sticky, Element::Frame(_) | Element::MagicFrame(_) => Tool::Frame, _ => Tool::Rect,
        });
        let opts = &self.style;
        let text = element.and_then(|e| {
            let label = self.bound_text_index(e).map(|i| &self.doc.scene.elements[i]);
            match label.unwrap_or(e) { Element::Text(t) => Some(t), _ => None }
        });
        let mut body = ui::popover().id("drawing-props-panel").occlude().w(px(PROPERTIES_W)).max_h(px(max_h)).overflow_y_scroll().p(px(11.)).flex().flex_col().gap(px(9.));
        if !self.selection.is_empty() { body = body.child(self.geometry_controls(cx)); }
        if self.selection.len() > 1 { body = body.child(self.alignment_controls(cx)); }
        if tool == Tool::Pen {
            use scene::types::StrokeVariability::{Constant, Variable};
            let mode = if let Some(Element::Freedraw(f)) = element { f.stroke_options.variability } else { self.stroke_variability };
            body = body.child(options("pen-mode", tr("笔画", "Stroke"), [
                ("—", tr("固定宽度", "Constant width"), mode == Constant, StyleChoice::PenMode(Constant)),
                ("∿", tr("可变宽度", "Variable width"), mode == Variable, StyleChoice::PenMode(Variable)),
            ], false, cx));
        }
        if tool == Tool::Text || text.is_some() {
            let ink = text.map_or(self.text_color.as_str(), |t| &t.base.stroke_color);
            body = body.child(self.swatches(tr("文字颜色", "Text color"), Colors::Text, ink, &STROKES, cx));
        }
        if !matches!(tool, Tool::Image | Tool::Text) {
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
            body = body.child(section(tr("描边宽度", "Stroke width"), div().flex().flex_wrap().gap(px(4.)).children(width_buttons)
                .child(self.precision_button("custom-stroke-width", Property::StrokeWidth, format!("{width}"), cx))));
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
            let start = match element { Some(Element::Arrow(a)) => a.linear.start_arrowhead, _ => self.start_arrowhead };
            let end = match element { Some(Element::Arrow(a)) => a.linear.end_arrowhead, _ => self.arrowhead };
            body = body.child(self.arrowheads(true, start, cx)).child(self.arrowheads(false, end, cx));
            let kind = match element { Some(Element::Arrow(a)) => if a.elbowed { 2 } else { u8::from(a.linear.base.roundness.is_some()) }, _ => self.arrow_type };
            body = body.child(options("arrow-type", tr("箭头类型", "Arrow type"), [
                ("→", tr("直线", "Straight"), kind == 0, StyleChoice::ArrowType(0)),
                ("⌁", tr("曲线", "Curved"), kind == 1, StyleChoice::ArrowType(1)),
                ("⌝", tr("折线", "Elbow"), kind == 2, StyleChoice::ArrowType(2)),
            ], false, cx));
        }
        if tool == Tool::Text || text.is_some() {
            let family = text.map_or(self.text_font, |t| t.font_family);
            let font_size = text.map_or(self.text_size, |t| t.font_size);
            let align = text.map_or(self.text_align, |t| t.text_align);
            body = body.child(self.font_picker(family, cx));
            body = body.child(options("text-size", tr("字体大小", "Font size"), [("XS", 12.), ("S", 16.), ("M", 20.), ("L", 28.), ("XL", 36.)].map(|(label, v)| {
                (label, label, font_size == v, StyleChoice::TextSize(v))
            }), false, cx));
            body = body.child(self.precision_button("custom-font-size", Property::FontSize, format!("{font_size}"), cx));
            let mut alignment = div().flex().flex_col().gap(px(4.)).child(option_buttons("text-align", [
                ("align-left", tr("左对齐", "Left"), align == DrawingAlign::Left, StyleChoice::TextAlign(DrawingAlign::Left)),
                ("align-center", tr("居中", "Center"), align == DrawingAlign::Center, StyleChoice::TextAlign(DrawingAlign::Center)),
                ("align-right", tr("右对齐", "Right"), align == DrawingAlign::Right, StyleChoice::TextAlign(DrawingAlign::Right)),
            ], true, cx));
            if let Some(t) = text && let Some(container) = t.container_id.as_deref().and_then(|id| self.doc.scene.get(id))
                && !matches!(container, Element::Arrow(_) | Element::Line(_)) {
                let align = t.vertical_align;
                alignment = alignment.child(option_buttons("text-vertical", [
                    ("valign-top", tr("顶部对齐", "Align top"), align == VerticalAlign::Top, StyleChoice::TextVertical(VerticalAlign::Top)),
                    ("valign-middle", tr("垂直居中", "Align middle"), align == VerticalAlign::Middle, StyleChoice::TextVertical(VerticalAlign::Middle)),
                    ("valign-bottom", tr("底部对齐", "Align bottom"), align == VerticalAlign::Bottom, StyleChoice::TextVertical(VerticalAlign::Bottom)),
                ], true, cx));
            }
            body = body.child(section(tr("文本对齐", "Text alignment"), alignment));
        }
        body = body.child(self.opacity_slider(b.map_or(opts.opacity, |b| b.opacity), cx));
        if !self.selection.is_empty() {
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
            body = body.child(div().flex().flex_wrap().gap(px(4.))
                .child(menu_button("drawing-group", "drawing-rectangle", tr("编组", "Group selection"), self.selection.len() > 1)
                    .on_click(cx.listener(|d, _, _, cx| d.group_selection(false, cx))))
                .child(menu_button("drawing-ungroup", "drawing-unlock", tr("取消编组", "Ungroup selection"), true)
                    .on_click(cx.listener(|d, _, _, cx| d.group_selection(true, cx))))
                .child(menu_button("drawing-lock-selection", "drawing-lock", tr("锁定所选内容", "Lock selection"), true)
                    .on_click(cx.listener(|d, _, _, cx| d.lock_selection(cx))))
                .child(menu_button("drawing-flip-h", "drawing-arrow", tr("水平翻转", "Flip horizontal"), true)
                    .on_click(cx.listener(|d, _, _, cx| d.flip_selection(true, cx))))
                .child(menu_button("drawing-flip-v", "drawing-down", tr("垂直翻转", "Flip vertical"), true)
                    .on_click(cx.listener(|d, _, _, cx| d.flip_selection(false, cx)))));
            if element.is_some_and(|e| linear_points(e).is_some()) {
                body = body.child(menu_button("drawing-edit-points", "pen-line", tr("编辑线条节点", "Edit line points"), true)
                    .on_click(cx.listener(|d, _, w, cx| d.edit_points(w, cx))));
            }
            if matches!(element, Some(Element::Image(_))) {
                body = body.child(menu_button("drawing-crop", "drawing-corners", tr("裁剪图片", "Crop image"), true)
                    .on_click(cx.listener(|d, _, w, cx| d.edit_points(w, cx))))
                    .child(menu_button("drawing-reset-crop", "undo", tr("还原裁剪", "Reset crop"), true)
                        .on_click(cx.listener(|d, _, _, cx| d.reset_crop(cx))));
            }
            body = body.child(menu_button("drawing-add-library", "plus", tr("添加到素材库", "Add to library"), true)
                .on_click(cx.listener(|d, _, _, cx| d.add_to_library(cx))));
        }
        body
    }

    fn precision_button(&self, id: impl Into<ElementId>, property: Property, value: String, cx: &mut Context<Self>) -> Stateful<Div> {
        div().id(id).min_w(px(50.)).h(px(28.)).px(px(7.)).flex().items_center().rounded(px(5.)).bg(Theme::bg_overlay()).text_size(px(12.)).cursor_pointer()
            .child(value.clone()).tip(property.label(), None)
            .on_click(cx.listener(move |d, _, w, cx| d.start_property(property, value.clone(), w, cx)))
    }

    fn geometry_controls(&self, cx: &mut Context<Self>) -> Div {
        let Some(b) = selection_bounds(&self.doc.scene, &self.selection) else { return div() };
        let angle = self.selected().map_or(0., |i| self.doc.scene.elements[i].base().angle.to_degrees());
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (n, property, value) in [(0, Property::X, b.min_x), (1, Property::Y, b.min_y), (2, Property::Width, b.width()), (3, Property::Height, b.height()), (4, Property::Angle, angle)] {
            row = row.child(div().w(px(92.)).flex().items_center().justify_between().gap(px(3.))
                .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(property.label()))
                .child(self.precision_button(("drawing-geometry", n as usize), property, format!("{value:.1}"), cx)));
        }
        section(tr("位置与尺寸", "Position and size"), row)
    }

    fn arrowheads(&self, start: bool, current: Option<Arrowhead>, cx: &mut Context<Self>) -> Div {
        let choices = [("—", "None", None), ("→", "Arrow", Some(Arrowhead::Arrow)), ("▸", "Triangle", Some(Arrowhead::Triangle)),
            ("▷", "Triangle outline", Some(Arrowhead::TriangleOutline)), ("●", "Dot", Some(Arrowhead::Dot)), ("⬤", "Circle", Some(Arrowhead::Circle)),
            ("○", "Circle outline", Some(Arrowhead::CircleOutline)), ("◆", "Diamond", Some(Arrowhead::Diamond)), ("◇", "Diamond outline", Some(Arrowhead::DiamondOutline)),
            ("│", "Bar", Some(Arrowhead::Bar)), ("1", "One", Some(Arrowhead::CardinalityOne)), ("N", "Many", Some(Arrowhead::CardinalityMany)),
            ("1N", "One or many", Some(Arrowhead::CardinalityOneOrMany)), ("11", "Exactly one", Some(Arrowhead::CardinalityExactlyOne)),
            ("01", "Zero or one", Some(Arrowhead::CardinalityZeroOrOne)), ("0N", "Zero or many", Some(Arrowhead::CardinalityZeroOrMany)),
            ("⊣", "Crowfoot one", Some(Arrowhead::CrowfootOne)), ("≺", "Crowfoot many", Some(Arrowhead::CrowfootMany)), ("≼", "Crowfoot one or many", Some(Arrowhead::CrowfootOneOrMany))];
        section(if start { tr("箭头起点", "Arrow start") } else { tr("箭头末端", "Arrowhead") }, div().flex().flex_wrap().gap(px(4.))
            .children(choices.into_iter().enumerate().map(|(i, (glyph, label, head))| {
                property_button((if start { "arrow-start" } else { "arrowhead" }, i), label, current == head, cx,
                    if start { StyleChoice::StartArrowHead(head) } else { StyleChoice::ArrowHead(head) }).child(glyph)
            })))
    }

    fn alignment_controls(&self, cx: &mut Context<Self>) -> Div {
        let mut row = div().flex().flex_wrap().gap(px(3.));
        for (direction, glyph, label) in [
            (AlignSelection::Left, "align-left", tr("左对齐", "Left")), (AlignSelection::Center, "align-center", tr("水平居中", "Center horizontally")),
            (AlignSelection::Right, "align-right", tr("右对齐", "Right")), (AlignSelection::Top, "valign-top", tr("顶部对齐", "Align top")),
            (AlignSelection::Middle, "valign-middle", tr("垂直居中", "Align middle")), (AlignSelection::Bottom, "valign-bottom", tr("底部对齐", "Align bottom")),
            (AlignSelection::Horizontal, "drawing-rectangle", tr("水平分布", "Distribute horizontally")), (AlignSelection::Vertical, "drawing-corners", tr("垂直分布", "Distribute vertically")),
        ] {
            let enabled = self.selection.len() >= if matches!(direction, AlignSelection::Horizontal | AlignSelection::Vertical) { 3 } else { 2 };
            row = row.child(icon_button(("drawing-align", direction as usize), glyph, label, None, false, enabled).bg(Theme::bg_overlay())
                .on_click(cx.listener(move |d, _, _, cx| d.align_selection(direction, cx))));
        }
        section(tr("对齐与分布", "Align and distribute"), row)
    }

    fn library_panel(&self, max_h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut panel = ui::popover().id("drawing-library-panel").occlude().w(px(LIBRARY_W)).max_h(px(max_h)).overflow_y_scroll().p(px(9.)).flex().flex_col().gap(px(6.))
            .child(div().font_weight(FontWeight::MEDIUM).child(tr("素材库", "Library")))
            .child(menu_button("library-add", "plus", tr("添加所选内容", "Add selection"), !self.selection.is_empty())
                .on_click(cx.listener(|d, _, _, cx| d.add_to_library(cx))))
            .child(menu_button("library-import", "import", tr("导入素材库", "Import library"), true)
                .on_click(cx.listener(|d, _, w, cx| d.import_library(w, cx))))
            .child(menu_button("library-export", "export", tr("导出素材库", "Export library"), !self.library.is_empty())
                .on_click(cx.listener(|d, _, w, cx| d.export_library(w, cx))));
        for index in 0..self.library.len() {
            let preview = self.library_thumbnail(index);
            panel = panel.child(div().flex().gap(px(4.)).items_center()
                .child(div().id(("library-item", index)).flex_1().h(px(84.)).rounded(px(7.)).bg(Theme::bg_overlay()).cursor_pointer()
                    .child(canvas(|_, _, _| (), move |bounds, _, w, _| {
                        if let Some(image) = &preview {
                            let dimensions = image.size(0); let ratio = dimensions.width.0 as f32 / dimensions.height.0.max(1) as f32;
                            let h = f32::from(bounds.size.height).min(f32::from(bounds.size.width) / ratio);
                            let size = size(px(h * ratio), px(h)); let at = bounds.center() - point(size.width / 2., size.height / 2.);
                            let _ = w.paint_image(Bounds::new(at, size), Corners::default(), image.clone(), 0, false);
                        }
                    }).size_full())
                    .on_click(cx.listener(move |d, _, w, cx| {
                        if let Some(doc) = d.library.get(index).cloned() { let at = d.world(d.viewport.get().center()); d.insert_scene(doc, at, w, cx); }
                    })))
                .child(icon_button(("library-remove", index), "trash-2", tr("删除", "Delete"), None, false, true)
                    .on_click(cx.listener(move |d, _, w, cx| {
                        if d.editable() && index < d.library.len() {
                            d.library.remove(index);
                            for image in d.library_images.borrow_mut().drain().map(|(_, image)| image) { let _ = w.drop_image(image); }
                            d.persist_library(); cx.notify();
                        }
                    }))));
        }
        panel
    }

    fn conversion_menu(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        use scene::element::ConvertTarget::*;
        let props = |target| match target {
            Rectangle => ("drawing-rectangle",tr("矩形","Rectangle")), Diamond => ("drawing-diamond",tr("菱形","Diamond")),
            Ellipse => ("drawing-ellipse",tr("椭圆","Ellipse")), Line => ("minus",tr("直线","Line")),
            SharpArrow => ("drawing-arrow",tr("直箭头","Straight arrow")), CurvedArrow => ("drawing-arrow",tr("曲线箭头","Curved arrow")),
            ElbowArrow => ("drawing-arrow",tr("折线箭头","Elbow arrow")),
        };
        ui::popover().id("drawing-convert-menu").occlude().w(px(MENU_W)).py(px(5.))
            .children(self.shared_conversion_targets().iter().enumerate().map(|(i,&target)| {
                let (glyph,label) = props(target);
                let active = self.selection.iter().all(|&index| self.doc.scene.elements[index].convert_target() == Some(target));
                menu_button(("drawing-convert-target",i),glyph,label,true).when(active,checked)
                    .on_click(cx.listener(move |d,_,_,cx| d.convert_selection_to(target,cx)))
            }))
    }

    fn context_menu(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = !self.selection.is_empty();
        ui::popover().id("drawing-context").occlude().w(px(230.)).max_h(px(520.)).overflow_y_scroll().py(px(5.))
            .child(menu_button("drawing-context-copy", "copy", tr("复制", "Copy"), selected)
                .on_click(cx.listener(|d, _, _, cx| { d.copy_selection(false, cx); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-context-cut", "scissors", tr("剪切", "Cut"), selected)
                .on_click(cx.listener(|d, _, _, cx| { d.copy_selection(true, cx); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-context-paste", "copy", tr("粘贴", "Paste"), true)
                .on_click(cx.listener(|d, _, w, cx| { d.panel = None; d.paste_selection(w, cx); cx.notify(); })))
            .child(menu_button("drawing-context-duplicate", "copy", tr("创建副本", "Duplicate"), selected)
                .on_click(cx.listener(|d, _, _, cx| { d.duplicate(cx); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-context-all", "check", tr("全选", "Select all"), true)
                .on_click(cx.listener(|d, _, _, cx| d.select_all(cx))))
            .child(horizontal_line())
            .when(!self.shared_conversion_targets().is_empty(), |d| d
                .child(menu_button("drawing-switch-shape", "drawing-rectangle", tr("切换形状", "Shape switch"), true)
                    .on_click(cx.listener(|d,_,_,cx| { d.panel = Some(Panel::Convert); cx.notify(); }))))
            .when(self.selected_frame().is_some(), |d| d
                .child(menu_button("drawing-select-frame-contents", "check", tr("选择画框内所有元素", "Select all elements in frame"), true)
                    .on_click(cx.listener(|d, _, _, cx| d.frame_members(false, cx))))
                .child(menu_button("drawing-detach-frame-contents", "drawing-corners", tr("移出画框内所有元素", "Remove all elements from frame"), true)
                    .on_click(cx.listener(|d, _, _, cx| d.frame_members(true, cx)))))
            .when(self.selection.len() > 1, |d| d.child(div().px(px(9.)).py(px(5.)).child(self.alignment_controls(cx))))
            .child(menu_button("drawing-context-group", "drawing-rectangle", tr("编组", "Group selection"), self.selection.len() > 1)
                .on_click(cx.listener(|d, _, _, cx| { d.group_selection(false, cx); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-context-ungroup", "drawing-unlock", tr("取消编组", "Ungroup selection"), selected)
                .on_click(cx.listener(|d, _, _, cx| { d.group_selection(true, cx); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-context-lock", "drawing-lock", tr("锁定所选内容", "Lock selection"), selected)
                .on_click(cx.listener(|d, _, _, cx| { d.lock_selection(cx); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-context-unlock", "drawing-unlock", tr("解锁全部", "Unlock all"), true)
                .on_click(cx.listener(|d, _, _, cx| { d.selection.clear(); d.lock_selection(cx); d.panel = None; cx.notify(); })))
            .child(horizontal_line())
            .when(self.element_link_target().is_some(), |menu| menu.child(menu_button("drawing-copy-element-link","link",tr("复制元素链接","Copy element link"),true)
                .on_click(cx.listener(|d,_,_,cx| { if let Some(id) = d.element_link_target() {
                    let mut url = url::Url::parse("https://excalidraw.com/").unwrap(); url.query_pairs_mut().append_pair("element",id);
                    palette::write_clipboard_text(url.to_string(),cx);
                } d.panel = None; cx.notify(); }))))
            .when(self.selected().is_some_and(|i| self.doc.scene.elements[i].base().link.is_some()), |m| m.child(menu_button("drawing-follow-link","link",tr("打开链接","Open link"),true)
                .on_click(cx.listener(|d,_,_,cx| { d.follow_link(cx); d.panel = None; cx.notify(); }))))
            .child(menu_button("drawing-copy-svg","copy",tr("复制 SVG","Copy as SVG"),true)
                .on_click(cx.listener(|d,_,_,cx| { d.copy_formatted(true,cx); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-copy-text","type",tr("复制文字","Copy text"),selected)
                .on_click(cx.listener(|d,_,_,cx| { d.copy_formatted(false,cx); d.panel = None; cx.notify(); })))
            .child(menu_button("drawing-context-delete", "trash-2", tr("删除", "Delete"), selected)
                .on_click(cx.listener(|d, _, _, cx| { d.delete(cx); d.panel = None; cx.notify(); })))
    }

    fn help_menu(&self, max_h: f32) -> Stateful<Div> {
        let rows = [
            (tr("选择", "Selection"), "V / 1"), (tr("平移", "Hand"), "H / Space"), (tr("矩形", "Rectangle"), "R / 2"),
            (tr("菱形", "Diamond"), "D / 3"), (tr("椭圆", "Ellipse"), "O / 4"), (tr("箭头", "Arrow"), "A / 5"),
            (tr("直线", "Line"), "L / 6"), (tr("画笔", "Freehand"), "P / 7"), (tr("文字", "Text"), "T / 8"),
            (tr("图片", "Image"), "9"), (tr("橡皮擦", "Eraser"), "E / 0"), (tr("画框", "Frame"), "F"),
            (tr("激光笔", "Laser pointer"), "K"), (tr("取色", "Eyedropper"), "I"), (tr("填充颜色", "Bucket fill"), "B"),
            (tr("切换形状类型", "Convert element type"), "Tab / Shift+Tab"), (tr("绑定/解绑文字", "Bind / unbind text"), "W"), (tr("文字自动宽度", "Text auto resize"), "Alt+W"),
            (tr("素材库", "Library"), "Shift+L"), (tr("复制为 PNG", "Copy as PNG"), "Alt+Shift+C"),
            (tr("对象吸附", "Snap to objects"), "Alt+S"), (tr("只读浏览", "View mode"), "Alt+R"), (tr("专注模式", "Zen mode"), "Alt+Z"),
            (tr("全选", "Select all"), "⌘A"), (tr("复制", "Copy"), "⌘C"), (tr("剪切", "Cut"), "⌘X"), (tr("粘贴", "Paste"), "⌘V"),
            (tr("创建副本", "Duplicate"), "⌘D / Alt + drag"), (tr("编组", "Group selection"), "⌘G"), (tr("取消编组", "Ungroup selection"), "⌘⇧G"),
            (tr("编辑线条节点", "Edit line points"), "⌘Enter"), (tr("锁定所选内容", "Lock selection"), "⌘⇧L"),
            (tr("链接", "Link"), "⌘K"), (tr("查找画板文字", "Find text on canvas"), "⌘F"),
            (tr("缩放到全部内容", "Zoom to fit"), "Shift + 1"), (tr("在视口中显示所选内容", "Fit selection in viewport"), "Shift + 2"),
            (tr("缩放到所选内容", "Zoom to selection"), "Shift + 3"), (tr("按页平移画布", "Pan by page"), "PgUp / PgDn"),
            (tr("按页横向平移", "Pan horizontally by page"), "Shift + PgUp / PgDn"),
            (tr("网格模式", "Grid mode"), "Shift + G"), (tr("水平翻转", "Flip horizontal"), "Shift + H"), (tr("垂直翻转", "Flip vertical"), "Shift + V"),
        ];
        ui::popover().id("drawing-help-panel").occlude().w(px(310.)).max_h(px(max_h)).overflow_y_scroll().p(px(12.)).flex().flex_col().gap(px(7.))
            .child(div().font_weight(FontWeight::MEDIUM).child(tr("快捷键", "Keyboard shortcuts")))
            .children(rows.into_iter().map(|(label, key)| div().flex().justify_between().gap(px(8.)).text_size(px(12.)).child(label)
                .child(div().text_color(Theme::text_caption()).child(key.replace('⌘', if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" })))))
    }

    fn font_picker(&self, current: FontFamily, cx: &mut Context<Self>) -> Div {
        let mut row = div().flex().items_center().gap(px(4.));
        let picks = [
            (FontFamily::Excalifont, "pen-line", tr("手写", "Hand-drawn")), (FontFamily::Nunito, "type", tr("标准", "Normal")),
            (FontFamily::ComicShanns, "code", tr("等宽", "Code")), (FontFamily::Lilita, "bold", tr("醒目", "Display")),
        ];
        for (family, glyph, label) in picks {
            row = row.child(property_button(("text-font", family.code() as usize), label, current == family, cx, StyleChoice::TextFont(family))
                .child(icon(glyph, 16., Theme::text())));
        }
        row = row.child(div().w(px(1.)).h(px(18.)).bg(Theme::line_divider()))
            .child(icon_button("drawing-more-fonts", "chevron-down", tr("更多字体", "More fonts"), None, self.more_fonts || !picks.iter().any(|(family, ..)| *family == current), true)
                .on_click(cx.listener(|d, _, _, cx| { d.more_fonts = !d.more_fonts; d.colors = None; cx.notify(); })));
        section(tr("字体", "Font family"), div().flex().flex_col().gap(px(4.)).child(row).when(self.more_fonts, |d| {
            let list = quiet_excalidraw::fonts::BUNDLED.iter().map(|&(family, _)| {
                ui::menu_row(("drawing-font", family.code() as usize), "drawing-fonts".into())
                    .font_family(family.name()).child(div().flex_1().child(family.label()))
                    .when(current == family, checked)
                    .on_click(cx.listener(move |d, _, w, cx| {
                        d.more_fonts = false;
                        d.apply_style(StyleChoice::TextFont(family), true, w, cx);
                    }))
            });
            d.child(div().flex().flex_col().py(px(3.)).rounded(px(8.)).bg(Theme::bg_overlay()).children(list))
        }))
    }

    fn swatches(&self, label: &'static str, target: Colors, current: &str, colors: &[&'static str], cx: &mut Context<Self>) -> Div {
        let s = target as usize;
        let chooser = div().id(("drawing-color-more", s)).size(px(25.)).flex().items_center().justify_center().rounded(px(5.)).ml(px(4.)).border_1()
            .border_color(if self.colors == Some(target) { Theme::drawing_accent() } else { Theme::line_border() }).cursor_pointer().child(swatch_fill(current, 20.))
            .tip(tr("更多颜色", "More colors"), None)
            .on_click(cx.listener(move |d, _, _, cx| { d.colors = (d.colors != Some(target)).then_some(target); d.more_fonts = false; cx.notify(); }));
        let buttons = |prefix, palette: &[&'static str]| palette.iter().copied().enumerate().map(|(i, color)| {
            div().id((prefix, s * 100 + i)).size(px(25.)).flex().items_center().justify_center().rounded(px(5.)).border_1().cursor_pointer()
                .border_color(if current.eq_ignore_ascii_case(color) { Theme::drawing_accent() } else { gpui::transparent_black() }).child(swatch_fill(color, 21.)).tip(color, None)
                .on_click(cx.listener(move |d, _, w, cx| match target {
                    Colors::Stroke => d.apply_style(StyleChoice::Stroke(color), true, w, cx),
                    Colors::Fill => d.apply_style(StyleChoice::Fill(color), true, w, cx),
                    Colors::Paper => d.change_paper(color, cx),
                    Colors::Text => d.apply_style(StyleChoice::TextColor(color), true, w, cx),
                }))
        }).collect::<Vec<_>>();
        let common = buttons("drawing-color", colors); let extra = buttons("drawing-extra-color", &EXTRA);
        section(label, div().flex().flex_col().gap(px(4.))
            .child(div().flex().gap(px(3.)).children(common).child(chooser))
            .when(self.colors == Some(target), |d| d.child(div().w(px(180.)).flex().flex_wrap().gap(px(3.)).children(extra))
                .child(div().flex().gap(px(4.))
                    .child(self.precision_button(("drawing-custom-color", s), Property::Color(target), current.to_string(), cx))
                    .child(icon_button(("drawing-eyedropper", s), "eye", tr("取色", "Eyedropper"), Some("I"), false, true)
                        .on_click(cx.listener(move |d, _, w, cx| { d.eyedropper = target; d.tool = Tool::Eyedropper; d.colors = None; w.focus(&d.focus); cx.notify(); }))))))
    }

    fn opacity_slider(&self, opacity: f64, cx: &mut Context<Self>) -> Div {
        let bounds = self.opacity_bounds.clone();
        section(tr("透明度", "Opacity"), div().flex().flex_col().gap(px(3.))
            .child(div().id("drawing-opacity").relative().w_full().h(px(19.)).flex().items_center().cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(|d, ev: &MouseDownEvent, w, cx| {
                    d.opacity_dragging = Some(false); d.set_opacity(ev.position.x, w, cx); cx.stop_propagation();
                }))
                .on_mouse_move(cx.listener(|d, ev: &MouseMoveEvent, w, cx| {
                    if ev.pressed_button == Some(MouseButton::Left) { d.set_opacity(ev.position.x, w, cx); }
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

    fn set_opacity(&mut self, x: Pixels, window: &Window, cx: &mut Context<Self>) {
        let Some(has_checkpoint) = self.opacity_dragging else { return };
        let track = self.opacity_bounds.get();
        if track.size.width <= px(14.) { return; }
        let value = (100. * f32::from(x - track.left() - px(7.)) / f32::from(track.size.width - px(14.))).round().clamp(0., 100.) as f64;
        let current = self.selected().map_or(self.style.opacity, |i| self.doc.scene.elements[i].base().opacity);
        if current != value {
            self.apply_style(StyleChoice::Opacity(value), !has_checkpoint, window, cx);
            self.opacity_dragging = Some(true);
        }
    }

    fn field_panel(&self, cx: &mut Context<Self>, floating: bool) -> AnyElement {
        let label = if self.mermaid { "Mermaid" } else if let Some(property) = self.edit_property { property.label() } else if self.searching { tr("查找", "Find") } else if self.edit_frame.is_some() { tr("画框名称", "Frame name") } else if self.edit_link.is_some() { tr("链接", "Link") } else { tr("文字", "Text") };
        let view = self.viewport.get();
        if let Some((p, index)) = self.text_at {
            let t = index.and_then(|i| match &self.doc.scene.elements[i] { Element::Text(t) => Some(t), _ => None });
            let field = self.field.read(cx);
            let width = if let Some(t) = t.filter(|t| !t.auto_resize || t.container_id.is_some()) { t.base.width * self.zoom }
                else if let Some(width) = self.text_width { width * self.zoom }
                else if let Some(i) = self.text_container { (self.doc.scene.elements[i].base().width - 10.).max(10.) * self.zoom }
                else { scene::text::text_width_in(&field.text, f64::from(field.size), t.map_or(self.text_font, |t| t.font_family)) + f64::from(field.size) };
            let w = px((width as f32).max(30.).min(f32::from(view.size.width).max(30.)));
            let at = if let Some(t) = t { World::new(t.base.x, t.base.y) } else if self.text_container.is_some() {
                World::new(p.x - f64::from(f32::from(w)) / (2.*self.zoom), p.y - self.text_size*0.625)
            } else { p };
            let pos = self.offset + point(px((at.x*self.zoom) as f32), px((at.y*self.zoom) as f32));
            return div().id("drawing-canvas-text").absolute().occlude().left(pos.x.clamp(px(0.), (view.size.width-w).max(px(0.))))
                .top(pos.y.clamp(px(0.), (view.size.height-px(field.line_height)).max(px(0.)))).w(w).max_h(view.size.height).overflow_y_scroll()
                .child(self.field.clone()).into_any_element();
        }
        let (x,y) = (px(14.), px(62.));
        ui::popover().id("drawing-field").occlude().when(floating, |d| d.absolute().top(y).left(x)).w(px(FIELD_W)).p(px(6.)).flex().items_center().gap(px(5.))
            .child(div().id("drawing-field-scroll").flex_1().min_w_0().max_h(px(240.)).overflow_y_scroll().px(px(7.)).py(px(4.)).rounded(px(6.)).bg(Theme::bg_overlay()).child(self.field.clone()))
            .child(icon_button("drawing-text-done", "check", label, Some("Enter"), true, true)
                .on_click(cx.listener(|d, _, w, cx| d.finish_field(w, cx)))).into_any_element()
    }

    pub(super) fn confirmation(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let reset = self.reset_confirmation; let replace = self.pending_import.is_some();
        div().id("drawing-confirm-scrim").absolute().inset_0().occlude().bg(ui::scrim()).flex().justify_center().items_center()
            .child(ui::dialog("drawing-confirm").p(px(20.)).flex().flex_col().gap(px(16.))
                .child(if replace { tr("打开绘图并替换未保存的画板修改？", "Open drawing and replace unsaved changes?") } else if reset { tr("清空画布上的内容？", "Clear the canvas?") } else { tr("放弃未保存的画板修改？", "Discard unsaved drawing changes?") })
                .child(div().flex().justify_end().gap(px(8.))
                    .child(ui::secondary_button("drawing-keep", "取消").on_click(cx.listener(move |d, _, w, cx| {
                        if reset { d.pending_import = None; d.reset_confirmation = false; cx.notify(); } else { d.keep_editing(w, cx); }
                    })))
                    .child(ui::primary_button("drawing-confirm-action", if replace { "打开绘图" } else if reset { "重置画布" } else { "放弃修改" })
                        .on_click(cx.listener(move |d, _, w, cx| { if let Some(doc) = d.pending_import.take() { d.replace_scene(doc,w,cx); } else if reset { d.reset_canvas(cx); } else { d.close(w, cx); } })))))
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
    section(label, option_buttons(id, items, icons, cx))
}

fn option_buttons<const N: usize>(id: &'static str, items: [(&'static str, &'static str, bool, StyleChoice); N], icons: bool, cx: &mut Context<DrawingEditor>) -> Div {
    let buttons = items.into_iter().enumerate().map(|(i, (glyph, tip, selected, choice))| {
        property_button((id, i), tip, selected, cx, choice)
            .when(icons, |d| d.child(icon(glyph, 16., Theme::text())))
            .when(!icons, |d| d.child(glyph))
    });
    div().flex().gap(px(4.)).children(buttons)
}

fn property_button(id: impl Into<ElementId>, tip: &str, selected: bool, cx: &mut Context<DrawingEditor>, choice: StyleChoice) -> Stateful<Div> {
    div().id(id).size(px(32.)).flex_none().flex().items_center().justify_center().rounded(px(8.)).cursor_pointer().text_size(px(13.))
        .bg(if selected { Theme::drawing_selection() } else { Theme::bg_overlay() }).hover(|s| s.bg(Theme::fill_hover())).tip(tip, None)
        .on_click(cx.listener(move |d, _, w, cx| d.apply_style(choice, true, w, cx)))
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
