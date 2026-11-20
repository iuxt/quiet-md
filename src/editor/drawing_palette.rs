//! The palette dispatches the same commands as the toolbar and shortcuts.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Command {
    Tool(Tool), Fit, ZoomSelected, Minimap, Stats, Export, Import, Find, Library, Help, Dark, Grid, Snap, Zen, View,
    Undo, Redo, Duplicate, Delete, Wrap, Group, Ungroup, Lock, Copy, Paste, Svg, Text, Link, Mermaid, FrameOnly, DetachFrame, FrameRendering, ShapeSwitch, Midpoints,
}

impl DrawingEditor {
    pub(super) fn open_palette(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.discard || self.reset_confirmation || self.gesture.is_some() { return; }
        if self.palette.take().is_none() {
            self.finish_field(w, cx);
            if self.mermaid { return; }
            self.panel = None; self.palette = Some(0);
            self.field.update(cx, |f,cx| { f.multiline = false; f.set_text("",cx); f.set_placeholder(tr("搜索命令", "Search commands"),cx); });
            w.focus(&self.field.read(cx).focus_handle(cx));
        } else { w.focus(&self.focus); }
        cx.notify();
    }

    pub(super) fn palette_items(&self, cx: &App) -> Vec<(Command, &'static str)> {
        use Command::*;
        let query = self.field.read(cx).text.trim().to_lowercase();
        let mut list = Vec::new();
        let mut add = |cmd, zh: &'static str, en: &'static str, enabled| {
            if enabled && (zh.contains(&query) || en.to_lowercase().contains(&query)) { list.push((cmd, tr(zh,en))); }
        };
        let edit = !self.view_only; let selected = !self.selection.is_empty();
        for &(tool,zh,en,_,_) in TOOLS { add(Tool(tool),zh,en,edit); }
        for (cmd, zh, en, enabled) in [
            (Fit,"缩放到全部内容","Zoom to fit",true),(ZoomSelected,"缩放到选区","Zoom to selection",selected),
            (Find,"查找画板文字","Find text on canvas",true),(Export,"导出绘图","Export drawing",true),
            (Import,"打开绘图","Open drawing",edit),(Library,"素材库","Library",edit),(Mermaid,"插入 Mermaid","Insert Mermaid",edit),
            (Minimap,"小地图","Minimap",true),(Stats,"画板统计","Canvas statistics",true),(Grid,"网格模式","Grid mode",true),
            (Snap,"对象吸附","Snap to objects",edit),(Dark,"深色画布","Dark canvas",true),(Zen,"专注模式","Zen mode",true),
            (View,"只读浏览","View mode",true),(Help,"快捷键","Keyboard shortcuts",true),(Undo,"撤销","Undo",edit && !self.undo.is_empty()),
            (Redo,"重做","Redo",edit && !self.redo.is_empty()),(Duplicate,"创建副本","Duplicate",edit && selected),
            (Delete,"删除","Delete",edit && selected),(Wrap,"包裹或解除包裹","Wrap or unwrap",edit && selected),(Lock,"锁定或解锁","Lock or unlock",edit && selected),
            (Group,"编组","Group selection",edit && self.selection.len()>1),(Ungroup,"取消编组","Ungroup selection",edit && selected),
            (FrameOnly,"选择画框内所有元素","Select all elements in frame",edit && self.selected_frame().is_some()),
            (DetachFrame,"移出画框内所有元素","Remove all elements from frame",edit && self.selected_frame().is_some()),
            (FrameRendering,"显示画框","Show frames",true),
            (ShapeSwitch,"切换形状","Shape switch",edit && !self.shared_conversion_targets().is_empty()),
            (Midpoints,"箭头吸附到边中点","Snap arrows to midpoints",edit),
            (Copy,"复制","Copy",selected),(Paste,"粘贴","Paste",edit),(Svg,"复制 SVG","Copy as SVG",true),
            (Text,"复制文字","Copy text",selected),(Link,"编辑链接","Edit link",edit && self.selected().is_some()),
        ] { add(cmd,zh,en,enabled); }
        list
    }

    pub(super) fn palette_step(&mut self, down: bool, cx: &mut Context<Self>) {
        let count = self.palette_items(cx).len(); if count == 0 { return; }
        let index = self.palette.unwrap_or(0); self.palette = Some((index + if down { 1 } else { count-1 }) % count); cx.notify();
    }

    pub(super) fn run_command(&mut self, cmd: Command, w: &mut Window, cx: &mut Context<Self>) {
        use Command::*;
        self.palette = None; w.focus(&self.focus);
        match cmd {
            Tool(t) => self.tool(t,w,cx), Fit => self.fit_view(), ZoomSelected => self.zoom_selection(cx),
            Minimap => self.minimap = !self.minimap, Stats => self.stats = !self.stats, Export => self.open_export(cx),
            Import => self.import(w,cx), Find => self.start_find(w,cx), Library => self.panel = Some(Panel::Library), Help => self.panel = Some(Panel::Help),
            Dark => self.dark_canvas = !self.dark_canvas, Grid => self.toggle_grid(cx), Snap => self.objects_snap = !self.objects_snap,
            Zen => self.zen = !self.zen, View => { self.view_only = !self.view_only; self.selection.clear(); },
            Undo => self.history(false,cx), Redo => self.history(true,cx), Duplicate => self.duplicate(cx), Delete => self.delete(cx),
            Wrap => self.toggle_text_binding(w,cx), Group => self.group_selection(false,cx), Ungroup => self.group_selection(true,cx),
            Lock => self.lock_selection(cx), Copy => self.copy_selection(false,cx), Paste => self.paste_selection(w,cx),
            Svg => self.copy_formatted(true,cx), Text => self.copy_formatted(false,cx), Link => self.open_link_editor(w,cx),
            FrameOnly => self.frame_members(false,cx), DetachFrame => self.frame_members(true,cx),
            Midpoints => self.snap_arrow_midpoints = !self.snap_arrow_midpoints,
            ShapeSwitch => self.panel = Some(Panel::Convert),
            FrameRendering => self.export_options.show_frames = !self.export_options.show_frames,
            Mermaid => { self.mermaid = true; self.field.update(cx, |f,cx| { f.multiline = true; f.set_text("",cx); f.set_placeholder("flowchart LR",cx); }); w.focus(&self.field.read(cx).focus_handle(cx)); },
        }
        cx.notify();
    }

    pub(super) fn copy_formatted(&self, svg: bool, cx: &mut App) {
        let doc = if self.selection.is_empty() { &self.doc } else { &self.selection_export_document() };
        let value = if svg { let Ok(svg) = doc.export_svg(self.export_options) else { return }; svg } else {
            doc.scene.non_deleted().filter_map(|e| match e { Element::Text(t) => Some(text_to_edit(t)), _ => None }).collect::<Vec<_>>().join("\n")
        };
        if !value.is_empty() { write_clipboard_text(value,cx); }
    }
}

pub(super) fn write_clipboard_text(text: String, cx: &mut App) {
    let mut fake = crate::clipboard::FAKE_CLIPBOARD.lock().unwrap();
    if let Some(clip) = fake.as_mut() { clip.text = text; clip.html = None; clip.images.clear(); }
    else { cx.write_to_clipboard(ClipboardItem::new_string(text)); }
}
