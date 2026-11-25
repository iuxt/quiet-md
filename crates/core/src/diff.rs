//! Version history uses a bounded LCS for readable line counts. Git previews
//! use libgit2 over complete files or sequences of canonical DocxXML blocks.

use anyhow::{Context as _, Result};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Same, Added, Removed }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
}

/// Bound the quadratic LCS table used by version history.
pub const MAX_LINES: usize = 1200;

/// What a reader sees: block text in document order, one line per paragraph.
pub fn readable(text: &str, markdown: bool) -> String {
    if markdown { return text.to_string() }
    crate::docxml::parse(text).map_or_else(|_| text.to_string(), |d| d.plain_text())
}

fn split(text: &str, markdown: bool) -> Vec<String> {
    let body = readable(text, markdown);
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES);
        lines.push(format!("… 仅显示前 {MAX_LINES} 行 / only the first {MAX_LINES} lines"));
    }
    lines
}

pub fn diff(old: &str, new: &str, markdown: bool) -> Vec<Line> {
    let a = split(old, markdown);
    let b = split(new, markdown);
    sequence(&a, &b).into_iter().map(|(kind, i)| Line { kind, text: if kind == Kind::Removed { a[i].clone() } else { b[i].clone() } }).collect()
}

fn sequence<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(Kind, usize)> {
    let (n, m) = (a.len(), b.len());
    let w = m + 1;
    let mut dp = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if a[i] == b[j] {
                dp[(i + 1) * w + j + 1] + 1
            } else {
                dp[(i + 1) * w + j].max(dp[i * w + j + 1])
            };
        }
    }
    let mut out = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((Kind::Same, j));
            i += 1; j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            out.push((Kind::Removed, i));
            i += 1;
        } else {
            out.push((Kind::Added, j));
            j += 1;
        }
    }
    while i < n { out.push((Kind::Removed, i)); i += 1 }
    while j < m { out.push((Kind::Added, j)); j += 1 }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub struct DiffChange<T> {
    pub added: bool,
    pub number: usize,
    pub content: T,
}

fn git_changes(old: &[u8], new: &[u8], mut visit: impl FnMut(bool, usize, &[u8]) -> Result<()>) -> Result<()> {
    if old == new { return Ok(()) }
    let mut options = git2::DiffOptions::new();
    options.context_lines(0).force_text(true);
    let patch = git2::Patch::from_buffers(old, None, new, None, Some(&mut options))?;
    for hunk in 0..patch.num_hunks() {
        for index in 0..patch.num_lines_in_hunk(hunk)? {
            let line = patch.line_in_hunk(hunk, index)?;
            let (added, number) = match line.origin() {
                '+' => (true, line.new_lineno()),
                '-' => (false, line.old_lineno()),
                _ => continue,
            };
            visit(added, number.context("Git diff line is missing its position")? as usize, line.content())?;
        }
    }
    Ok(())
}

pub(crate) fn is_blank_line(line: &str) -> bool { line.trim_ascii().is_empty() }

/// Cumulative offsets from runs of skipped lines or blocks. Each run is
/// keyed by the next position in the filtered sequence.
#[derive(Default)]
struct SkippedPositions(Vec<(usize, usize)>);

impl SkippedPositions {
    fn skip(&mut self, original: usize) {
        let offset = self.0.last().map_or(0, |&(_, offset)| offset);
        let next = original - offset;
        if let Some((position, offset)) = self.0.last_mut() && *position == next {
            *offset += 1;
        } else {
            self.0.push((next, offset + 1));
        }
    }

    fn original(&self, number: usize) -> usize {
        let end = self.0.partition_point(|&(position, _)| position <= number);
        number + self.0[..end].last().map_or(0, |&(_, offset)| offset)
    }
}

fn without_blank_lines(text: &str) -> (Cow<'_, str>, SkippedPositions) {
    let mut skipped = SkippedPositions::default();
    let mut filtered = String::new();
    let (mut start, mut offset) = (0, 0);
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let end = offset + line.len();
        if is_blank_line(line) {
            filtered.push_str(&text[start..offset]);
            start = end;
            skipped.skip(index + 1);
        }
        offset = end;
    }
    if start == 0 { return (Cow::Borrowed(text), skipped) }
    filtered.push_str(&text[start..]);
    (Cow::Owned(filtered), skipped)
}

pub(crate) fn git_lines(old: &str, new: &str) -> Result<Vec<DiffChange<String>>> {
    if old == new { return Ok(vec![]) }
    let (old, old_skipped) = without_blank_lines(old);
    let (new, new_skipped) = without_blank_lines(new);
    let mut rows = Vec::new();
    git_changes(old.as_bytes(), new.as_bytes(), |added, number, bytes| {
        let line = std::str::from_utf8(bytes)?;
        let content = line.strip_suffix('\n').map_or(line, |s| s.strip_suffix('\r').unwrap_or(s));
        let number = if added { &new_skipped } else { &old_skipped }.original(number);
        rows.push(DiffChange { added, number, content: content.to_string() });
        Ok(())
    })?;
    Ok(rows)
}

/// Retain both documents for equation numbering. Each edit indexes the
/// original block in its own version, including style-only changes.
pub struct FormattedDiff {
    pub old: crate::model::Document,
    pub new: crate::model::Document,
    rows: Vec<DiffChange<()>>,
}

impl FormattedDiff {
    pub fn into_rows(self) -> impl Iterator<Item = DiffChange<crate::model::Block>> {
        let mut old = self.old.blocks.into_iter().enumerate();
        let mut new = self.new.blocks.into_iter().enumerate();
        self.rows.into_iter().map(move |DiffChange { added, number, .. }| {
            let blocks = if added { &mut new } else { &mut old };
            let (_, content) = blocks.find(|(i, _)| i + 1 == number).expect("Git diff block position");
            DiffChange { added, number, content }
        })
    }
}

pub(crate) fn formatted(old: &str, new: &str) -> Result<Option<FormattedDiff>> {
    use crate::{docxml, model::{Block, BlockKind, Document}};
    use std::fmt::Write as _;
    let document = |xml: &str| -> Option<Document> {
        if xml.is_empty() { return Some(Document { blocks: vec![], ..Document::default() }) }
        let mut doc = docxml::parse(xml).ok()?;
        fn number_lists(blocks: &mut [Block]) {
            let mut number = 0;
            for block in blocks {
                if let BlockKind::Ordered { seq } = &mut block.kind {
                    number = seq.unwrap_or(number + 1); *seq = Some(number);
                } else { number = 0; }
                number_lists(&mut block.children);
                if let BlockKind::Table(table) = &mut block.kind {
                    for row in &mut table.rows { for cell in row { number_lists(&mut cell.blocks); } }
                }
            }
        }
        number_lists(&mut doc.blocks);
        if !doc.title.is_empty() {
            let title = std::mem::take(&mut doc.title);
            doc.blocks.insert(0, Block::new(BlockKind::Heading { level: 1, seq_auto: false }, title));
        }
        Some(doc)
    };
    let (Some(old), Some(new)) = (document(old), document(new)) else { return Ok(None) };
    let ((old_keys, old_skipped), (new_keys, new_skipped)) = {
        let mut ids = rustc_hash::FxHashMap::default();
        let mut encode = |doc: &Document| {
            let mut buf = String::new();
            let mut skipped = SkippedPositions::default();
            for (index, block) in doc.blocks.iter().enumerate() {
                if matches!(block.kind, BlockKind::Text) && block.children.is_empty() && block.bg.is_none()
                    && is_blank_line(&block.text.text) {
                    skipped.skip(index + 1);
                    continue;
                }
                let next = ids.len();
                let id = ids.entry(docxml::serialize_block(block)).or_insert(next);
                writeln!(buf, "{id}").unwrap();
            }
            (buf, skipped)
        };
        (encode(&old), encode(&new))
    };
    let mut rows = Vec::new();
    git_changes(old_keys.as_bytes(), new_keys.as_bytes(), |added, number, _| {
        let number = if added { &new_skipped } else { &old_skipped }.original(number);
        rows.push(DiffChange { added, number, content: () });
        Ok(())
    })?;
    Ok(Some(FormattedDiff { old, new, rows }))
}

pub fn counts(lines: &[Line]) -> (usize, usize) {
    (
        lines.iter().filter(|l| l.kind == Kind::Added).count(),
        lines.iter().filter(|l| l.kind == Kind::Removed).count(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_removed_and_unchanged_lines() {
        let d = diff("one\ntwo\nthree", "one\nTWO\nthree\nfour", true);
        assert_eq!(counts(&d), (2, 1));
        assert_eq!(d[0], Line { kind: Kind::Same, text: "one".into() });
        assert_eq!(d[1], Line { kind: Kind::Removed, text: "two".into() });
        assert_eq!(d[2], Line { kind: Kind::Added, text: "TWO".into() });
        assert_eq!(d[3], Line { kind: Kind::Same, text: "three".into() });
        assert_eq!(d[4], Line { kind: Kind::Added, text: "four".into() });
    }

    #[test]
    fn identical_documents_have_no_changes() {
        let d = diff("<p>a</p>", "<p>a</p>", false);
        assert_eq!(counts(&d), (0, 0));
        assert!(d.iter().all(|l| l.kind == Kind::Same));
    }

    #[test]
    fn xml_markup_is_not_compared() {
        let a = "<title>x</title><p>first</p>";
        let b = "<title>x</title><p>second</p>";
        let d = diff(a, b, false);
        assert_eq!(counts(&d), (1, 1));
        assert!(d.iter().any(|l| l.text == "first" && l.kind == Kind::Removed));
        assert!(d.iter().any(|l| l.text == "second" && l.kind == Kind::Added));
    }

    #[test]
    fn git_previews_ignore_blank_rows_and_keep_original_numbers() {
        let old = "\n \t\r\nBefore\n\nKeep\n\nOld tail\n";
        let new = "After\n\nKeep\n \n\nNew tail\n";
        let text = git_lines(old, new).unwrap();
        let positions = |rows: &[DiffChange<String>]| rows.iter().map(|r| (r.added, r.number)).collect::<Vec<_>>();
        let expected = [(false, 3), (true, 1), (false, 7), (true, 6)];
        assert_eq!(positions(&text), expected);
        assert!(git_lines("A\n\n\nB\n", "A\nB\n \t\n").unwrap().is_empty());
        assert_eq!(git_lines("item\n", "  item\n").unwrap().len(), 2);

        let old = "<p></p><p> </p><p>Before</p><p></p><p>Keep</p><p></p><p>Old tail</p>";
        let new = "<p>After</p><p></p><p>Keep</p><p> </p><p></p><p>New tail</p>";
        let rows = formatted(old, new).unwrap().unwrap().into_rows();
        assert_eq!(rows.map(|r| (r.added, r.number)).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn formatted_diff_detects_styles_without_text_changes() {
        let a = "<p>same text</p><p>unchanged</p>";
        let b = "<p><b>same text</b></p><p>unchanged</p>";
        assert_eq!(counts(&diff(a, b, false)), (0, 0));
        let rows = formatted(a, b).unwrap().unwrap().into_rows().collect::<Vec<_>>();
        assert_eq!(rows.iter().map(|r| (r.added, r.number)).collect::<Vec<_>>(), [(false, 1), (true, 1)]);
        assert!(!rows[0].content.text.runs[0].marks.bold);
        assert!(rows[1].content.text.runs[0].marks.bold);
    }

    #[test]
    fn formatted_diff_preserves_structure_and_ignores_parse_ids() {
        use crate::model::BlockKind;
        let xml = "<title>Title</title><h2>Heading</h2><blockquote><p>Quote</p></blockquote><ul><li>Item</li></ul>";
        assert!(formatted(xml, xml).unwrap().unwrap().into_rows().next().is_none());
        let added = formatted("", xml).unwrap().unwrap().into_rows().collect::<Vec<_>>();
        assert_eq!(added.len(), 4);
        assert!(added.iter().all(|row| row.added));
        assert!(matches!(added[1].content.kind, BlockKind::Heading { level: 2, .. }));
        assert!(matches!(added[2].content.kind, BlockKind::Quote));
        assert_eq!(added[2].content.children[0].text.text, "Quote");
        let removed = formatted(xml, "").unwrap().unwrap().into_rows().collect::<Vec<_>>();
        assert_eq!(removed.len(), 4);
        assert!(removed.iter().all(|row| !row.added));
        assert!(formatted("<p>broken", xml).unwrap().is_none());
        assert!(formatted("", "").unwrap().unwrap().into_rows().next().is_none());
    }

    #[test]
    fn formatted_diff_keeps_ordered_list_numbers_in_each_version() {
        use crate::model::BlockKind;
        let rich = formatted("<ol><li>One</li><li>Two</li></ol>", "<ol><li>One</li><li>New</li><li>Two</li></ol>").unwrap().unwrap();
        let numbers = rich.into_rows().map(|row| match row.content.kind {
            BlockKind::Ordered { seq } => seq.unwrap(), _ => panic!("ordered list expected"),
        }).collect::<Vec<_>>();
        assert_eq!(numbers, [2, 2, 3]);
    }

    #[test]
    fn formatted_diff_detects_heading_container_and_table_style_changes() {
        for (a, b) in [
            ("<p>Heading</p>", "<h1>Heading</h1>"),
            ("<blockquote><p>Quote</p></blockquote>", "<blockquote><p><i>Quote</i></p></blockquote>"),
            ("<table><tr><td><p>Cell</p></td></tr></table>", "<table><tr><td><p><b>Cell</b></p></td></tr></table>"),
        ] {
            let rich = formatted(a, b).unwrap().unwrap();
            assert_eq!(rich.into_rows().map(|r| r.added).collect::<Vec<_>>(), [false, true]);
        }
        let many = "<p>line</p>".repeat(MAX_LINES + 1);
        let rich = formatted("", &many).unwrap().unwrap();
        assert_eq!(rich.into_rows().count(), MAX_LINES + 1);
    }

    #[test]
    fn long_documents_are_truncated() {
        let many = (0..MAX_LINES + 100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let d = diff(&many, "", true);
        assert!(d.len() <= MAX_LINES + 1);
    }
}
