//! Line-level diff between two document versions. DocxXML is turned into
//! plain text first, so the view compares what the reader sees rather than
//! markup; Git also offers formatted block comparisons below. Markdown files
//! are compared as written. No diff crate: the LCS
//! table is bounded, which keeps a million-character note from allocating
//! hundreds of megabytes during a preview.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Same, Added, Removed }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
}

/// Beyond this many lines the preview is truncated; the table is O(n·m).
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

/// Read-only block comparisons retain styles and containers, including changes
/// that leave the plain text identical. Canonical XML ignores per-parse ids.
pub struct FormattedDiff {
    pub rows: Vec<(Kind, crate::model::Block)>,
    pub truncated: bool,
}

pub fn formatted(old: &str, new: &str) -> Option<FormattedDiff> {
    use crate::{docxml, model::{Block, BlockKind, Document}};
    let blocks = |xml: &str| -> Option<Vec<Block>> {
        if xml.is_empty() { return Some(vec![]) }
        let doc = docxml::parse(xml).ok()?;
        let mut blocks = doc.blocks;
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
        number_lists(&mut blocks);
        if !doc.title.is_empty() { blocks.insert(0, Block::new(BlockKind::Heading { level: 1, seq_auto: false }, doc.title)); }
        Some(blocks)
    };
    let mut a = blocks(old)?; let mut b = blocks(new)?;
    let truncated = a.len() > MAX_LINES || b.len() > MAX_LINES;
    a.truncate(MAX_LINES); b.truncate(MAX_LINES);
    let keys = |blocks: &[Block]| blocks.iter().map(|block| {
        docxml::serialize(&Document { blocks: vec![block.clone()], ..Document::default() })
    }).collect::<Vec<_>>();
    let rows = sequence(&keys(&a), &keys(&b)).into_iter().map(|(kind, i)| {
        (kind, if kind == Kind::Removed { a[i].clone() } else { b[i].clone() })
    }).collect();
    Some(FormattedDiff { rows, truncated })
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
    fn formatted_diff_detects_styles_without_text_changes() {
        let a = "<p>same text</p><p>unchanged</p>";
        let b = "<p><b>same text</b></p><p>unchanged</p>";
        assert_eq!(counts(&diff(a, b, false)), (0, 0));
        let rich = formatted(a, b).unwrap();
        assert_eq!(rich.rows.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec![Kind::Removed, Kind::Added, Kind::Same]);
        assert!(!rich.rows[0].1.text.runs[0].marks.bold);
        assert!(rich.rows[1].1.text.runs[0].marks.bold);
        assert!(!rich.truncated);
    }

    #[test]
    fn formatted_diff_preserves_structure_and_ignores_parse_ids() {
        use crate::model::BlockKind;
        let xml = "<title>Title</title><h2>Heading</h2><blockquote><p>Quote</p></blockquote><ul><li>Item</li></ul>";
        let rich = formatted(xml, xml).unwrap();
        assert!(rich.rows.iter().all(|(k, _)| *k == Kind::Same));
        assert!(matches!(rich.rows[1].1.kind, BlockKind::Heading { level: 2, .. }));
        assert!(matches!(rich.rows[2].1.kind, BlockKind::Quote));
        assert_eq!(rich.rows[2].1.children[0].text.text, "Quote");
        let added = formatted("", xml).unwrap();
        assert!(added.rows.iter().all(|(k, _)| *k == Kind::Added));
        let removed = formatted(xml, "").unwrap();
        assert!(removed.rows.iter().all(|(k, _)| *k == Kind::Removed));
        assert!(formatted("<p>broken", xml).is_none());
        assert!(formatted("", "").unwrap().rows.is_empty());
    }

    #[test]
    fn formatted_diff_keeps_ordered_list_numbers_in_each_version() {
        use crate::model::BlockKind;
        let rich = formatted("<ol><li>One</li><li>Two</li></ol>", "<ol><li>One</li><li>New</li><li>Two</li></ol>").unwrap();
        let numbers = rich.rows.iter().filter(|(k, _)| *k != Kind::Removed).map(|(_, b)| match b.kind {
            BlockKind::Ordered { seq } => seq.unwrap(), _ => panic!("ordered list expected"),
        }).collect::<Vec<_>>();
        assert_eq!(numbers, vec![1, 2, 3]);
    }

    #[test]
    fn formatted_diff_detects_heading_container_and_table_style_changes() {
        for (a, b) in [
            ("<p>Heading</p>", "<h1>Heading</h1>"),
            ("<blockquote><p>Quote</p></blockquote>", "<blockquote><p><i>Quote</i></p></blockquote>"),
            ("<table><tr><td><p>Cell</p></td></tr></table>", "<table><tr><td><p><b>Cell</b></p></td></tr></table>"),
        ] {
            let rich = formatted(a, b).unwrap();
            assert_eq!(rich.rows.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec![Kind::Removed, Kind::Added]);
        }
        let many = "<p>line</p>".repeat(MAX_LINES + 1);
        let rich = formatted("", &many).unwrap();
        assert!(rich.truncated);
        assert_eq!(rich.rows.len(), MAX_LINES);
    }

    #[test]
    fn long_documents_are_truncated() {
        let many = (0..MAX_LINES + 100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let d = diff(&many, "", true);
        assert!(d.len() <= MAX_LINES + 1);
    }
}
