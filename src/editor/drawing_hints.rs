use super::*;

const COMMAND: &str = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
const COMMAND_ENTER: &str = if cfg!(target_os = "macos") { "Cmd + Enter" } else { "Ctrl + Enter" };
const COMMAND_D: &str = if cfg!(target_os = "macos") { "Cmd + D" } else { "Ctrl + D" };
const COMMAND_ARROWS: &str = if cfg!(target_os = "macos") { "Cmd + ↑↓←→" } else { "Ctrl + ↑↓←→" };

impl DrawingEditor {
    pub(super) fn canvas_hint(&self) -> Option<Div> {
        if self.busy || self.discard || self.reset_confirmation || matches!(self.panel, Some(Panel::More | Panel::Context | Panel::Help))
            || self.edit_link.is_some() || self.edit_property.is_some() { return None; }
        if self.searching {
            return Some(hint(tr("按{}查找下一处，按{}查找上一处，按{}关闭查找", "{} for next match, {} for previous match, {} to close search"), &["Enter", "Shift + Enter", "Esc"]));
        }
        if self.text_at.is_some() {
            return Some(hint(tr("按{}换行，按{}或{}完成编辑", "{} for a new line, {} or {} to finish editing"), &["Enter", COMMAND_ENTER, "Esc"]));
        }
        if self.edit_frame.is_some() {
            return Some(hint(tr("按{}确认画框名称，按{}取消", "{} to confirm the frame name, {} to cancel"), &["Enter", "Esc"]));
        }
        if self.crop.is_some() {
            return Some(hint(tr("拖动四角裁剪，点击图片外或按{}或{}完成", "Drag the corners to crop. Click outside the image or press {} or {} to finish"), &["Enter", "Esc"]));
        }
        if self.pending_image.is_some() {
            return Some(hint(tr("点击画布放置图片", "Click the canvas to place the image"), &[]));
        }
        if self.view_only || self.tool == Tool::Pan || self.space || matches!(self.gesture, Some(Gesture::Pan { .. })) {
            return Some(hint(tr("拖动画布以平移", "Drag to pan the canvas"), &[]));
        }
        if !self.polyline.is_empty() {
            return Some(hint(tr("点击添加点，再点一次最后一个点，或按{}或{}结束绘制",
                "Click to add points. Click the last point again, or press {} or {} to finish"), &["Esc", "Enter"]));
        }

        let selected = self.selected().and_then(|i| self.doc.scene.elements.get(i));
        let gesture_hint = match &self.gesture {
            Some(Gesture::Rotate { .. }) => Some(hint(tr("按住{}，每次旋转 45°", "Hold {} to rotate in 45° steps"), &["Shift"])),
            Some(Gesture::Resize { .. }) if matches!(selected, Some(Element::Image(_))) => Some(hint(
                tr("按住{}自由缩放，按住{}从中心缩放", "Hold {} to resize freely, or {} to resize from the center"), &["Shift", "Alt"])),
            Some(Gesture::Resize { .. }) => Some(hint(
                tr("按住{}保持宽高比例，按住{}从中心缩放", "Hold {} to keep the aspect ratio, or {} to resize from the center"), &["Shift", "Alt"])),
            Some(Gesture::Box { .. }) => Some(hint(tr("按住{}再拖框，可选中编组内的元素", "Hold {} before dragging a box to select elements inside groups"), &[COMMAND])),
            Some(Gesture::Move { delta, .. }) if (self.grid || self.objects_snap) && delta.length() > MOVE_THRESHOLD => Some(hint(
                tr("按住{}暂时关闭吸附", "Hold {} to temporarily disable snapping"), &[COMMAND])),
            Some(Gesture::Shape { .. } | Gesture::Endpoint { .. }) if self.tool == Tool::Arrow || matches!(selected, Some(Element::Arrow(_))) => Some(hint(
                tr("按住{}约束角度，按住{}停用箭头绑定", "Hold {} to constrain the angle, or {} to disable arrow binding"), &["Shift", COMMAND])),
            Some(Gesture::Endpoint { .. }) | Some(Gesture::Shape { .. }) if self.tool == Tool::Line || matches!(selected, Some(Element::Line(_))) => Some(hint(
                tr("按住{}约束角度", "Hold {} to constrain the angle"), &["Shift"])),
            Some(Gesture::Shape { .. }) if matches!(self.tool, Tool::Rect | Tool::Diamond | Tool::Ellipse | Tool::Sticky | Tool::Frame) => Some(hint(
                tr("按住{}统一宽高，按住{}从中心绘制", "Hold {} for equal width and height, or {} to draw from the center"), &["Shift", "Alt"])),
            _ => None,
        };
        if gesture_hint.is_some() { return gesture_hint; }

        Some(match self.tool {
            Tool::Arrow => hint(tr("点击添加多个点，拖动绘制直线箭头。按{}切换箭头类型",
                "Click to add points or drag for a straight arrow. Press {} to switch arrow types"), &["A"]),
            Tool::AutoShape => hint(tr("拖动绘制形状、箭头或直线", "Drag to draw a shape, arrow, or line"), &[]),
            Tool::Lasso => hint(tr("拖动套索选择元素", "Drag a lasso around the elements to select"), &[]),
            Tool::Line => hint(tr("点击添加多个点，拖动绘制直线", "Click to add points or drag to draw a straight line"), &[]),
            Tool::Pen => hint(tr("按住并拖动绘制，松开完成", "Press and drag to draw, then release to finish"), &[]),
            Tool::Text => hint(tr("点击添加文字。也可用选择工具双击画布", "Click to add text. You can also double-click the canvas with the selection tool"), &[]),
            Tool::Sticky => hint(tr("点击放置便签，或拖动指定大小", "Click to place a sticky note or drag to size it"), &[]),
            Tool::Eraser => hint(tr("拖动擦除，按住{}扫过可恢复本次待擦除的元素", "Drag to erase. Hold {} and sweep over marked elements to restore them"), &["Alt"]),
            Tool::Frame => hint(tr("拖出画框，将其中的元素放入画框", "Drag to draw a frame around its contents"), &[]),
            Tool::Laser => hint(tr("拖动留下临时激光轨迹", "Drag to leave a temporary laser trail"), &[]),
            Tool::Eyedropper => hint(tr("点击元素或画布背景取色", "Click an element or the canvas background to sample its color"), &[]),
            Tool::Bucket => hint(tr("点击图形或多段线围出的区域填色", "Click a shape or an area enclosed by a polyline to fill it"), &[]),
            Tool::Select if matches!(self.gesture, Some(Gesture::Move { delta, .. }) if delta.length() > MOVE_THRESHOLD) => return None,
            Tool::Select => match selected {
                None if self.selection.is_empty() => hint(tr("按住{}或{}拖动画布，也可以用抓手工具",
                    "Hold {} or {} and drag to pan, or use the hand tool"), &[tr("鼠标中键", "Middle mouse button"), "Space"]),
                Some(e) if self.line_edit.is_some() && linear_points(e).is_some() => self.points_hint(e),
                Some(Element::Arrow(_)) => hint(tr("按住{}双击，或按{}编辑点", "Hold {} and double-click, or press {} to edit points"), &[COMMAND, COMMAND_ENTER]),
                Some(Element::Line(_)) => hint(tr("双击直线或按{}编辑点", "Double-click the line or press {} to edit points"), &[COMMAND_ENTER]),
                Some(Element::Text(t)) if !t.auto_resize && t.container_id.is_none() => hint(
                    tr("双击或按{}编辑文字，按{}切换自动宽度", "Double-click or press {} to edit text, or press {} to toggle automatic width"), &["Enter", "Alt + W"]),
                Some(Element::Text(_)) => hint(tr("双击或按{}编辑文字", "Double-click or press {} to edit text"), &["Enter"]),
                Some(Element::Image(_)) => hint(tr("双击图片或按{}开始裁剪", "Double-click the image or press {} to start cropping"), &["Enter"]),
                Some(Element::Frame(_) | Element::MagicFrame(_)) => hint(tr("按{}编辑画框名称", "Press {} to edit the frame name"), &["Enter"]),
                Some(Element::Rectangle { .. } | Element::Diamond { .. } | Element::Ellipse { .. } | Element::StickyNote(_)) => hint(
                    tr("按{}编辑图形文字，按{}添加相邻流程图节点", "Press {} to edit the label, or {} to add a connected flowchart node"), &["Enter", COMMAND_ARROWS]),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn points_hint(&self, element: &Element) -> Div {
        let elbow = matches!(element, Element::Arrow(a) if a.elbowed);
        let binding_endpoint = matches!(element, Element::Arrow(_))
            && matches!(&self.gesture, Some(Gesture::Vertex { indices, insert: None, .. })
                if linear_points(element).is_some_and(|p| indices.contains(&0) || indices.contains(&(p.len() - 1))));
        if binding_endpoint {
            hint(tr("按住{}停用箭头绑定", "Hold {} to disable arrow binding"), &[COMMAND])
        } else if self.selected_points.is_empty() {
            if elbow {
                hint(tr("点击选择端点，按住{}多选，拖动中点调整折线", "Click endpoints to select them, hold {} to select multiple, or drag a midpoint to adjust the bend"), &["Shift"])
            } else {
                hint(tr("点击选择点，按住{}多选，按住{}点击或拖动中点可加点",
                    "Click to select a point, hold {} to select multiple, or {} and click to add a point. You can also drag a midpoint"), &["Shift", "Alt"])
            }
        } else if elbow {
            hint(tr("拖拽移动点，按{}移除选中的点", "Drag to move points, or press {} to remove selected points"), &["Delete"])
        } else {
            hint(tr("按{}移除点，按{}复制点，拖拽移动点", "Press {} to remove points, press {} to duplicate them, or drag to move them"), &["Delete", COMMAND_D])
        }
    }
}

fn hint(text: &'static str, keys: &[&'static str]) -> Div {
    let mut row = div().min_w_0().max_w_full().flex().flex_wrap().justify_center().items_center().gap_x(px(2.)).gap_y(px(3.))
        .px(px(6.)).py(px(2.)).rounded(px(5.)).bg(Theme::bg_body()).font_family(theme::UI_FONT)
        .text_size(px(13.)).line_height(px(19.)).text_color(Theme::text()).text_align(TextAlign::Center).whitespace_normal();
    for (i, part) in text.split("{}").enumerate() {
        if i > 0 && let Some(key) = keys.get(i - 1) {
            row = row.child(div().flex_none().px(px(4.)).border_1().border_color(Theme::line_sheet()).rounded(px(4.))
                .font_family(theme::MONO_FONT).text_size(px(11.)).child(*key));
        }
        if !part.is_empty() { row = row.child(div().min_w_0().max_w_full().child(part)); }
    }
    row
}
