# 画板

在 DocxXML 笔记里从斜杠菜单选「画板」，或点击已有画板预览。当前分支在 Quiet.md 窗口内使用 [gpui-excalidraw](https://github.com/LittleGuest/gpui-excalidraw) 的工具栏、画布和属性面板。顶部「保存」或 `⌘S`（Windows 和 Linux 使用 `Ctrl+S`）将编辑结果写入笔记。返回笔记时会确认是否放弃草稿。

左上角菜单可打开 `.excalidraw` 文件和嵌入图源的 PNG/SVG，也可导出图源或图片。画板保存到笔记后，正文显示可再次打开的预览。工具、快捷键和临时素材面板沿用上游编辑器。上游的快捷键对话框可通过 `?` 打开。

上游版的素材面板随当前画板会话使用。原画板的小地图、图片裁剪和折线路由等操作与上游有差异。

构建 `cargo build --release --features devtools` 后，用 `python3 tools/harness/drawing.py` 在临时文档库验证编辑、存盘及重开画板。
