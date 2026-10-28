//! Entries and supersession marks in a binding artefact's body, in the form
//! the `supersession-marks` proposal's fixtures fix.
//!
//! An entry is an item of a numbered list at the top level of the body,
//! identified by the number written before it, its nested lists included. A
//! mark is `**Supersedes:**` or `**Superseded by:**`, then entry numbers
//! separated by commas and ending in a full stop. A legacy note is
//! `SUPERSEDED by`, then entry numbers separated by commas and any full stop
//! directly after the last. Both are read word by word so any white space may
//! wrap them, and either may begin or end inside a word, what is left of that
//! word staying a word. Code spans and fenced blocks hide marks and notes;
//! their words stay.

use std::ops::Range;

/// What a body records: its entries, and the words outside them.
pub struct Body<'a> {
    /// Every entry, in the order written, a repeated number kept.
    pub entries: Vec<Entry<'a>>,
    /// The words outside any entry, in order.
    pub outside: Vec<&'a str>,
}

pub struct Entry<'a> {
    pub number: u64,
    /// The words after its number, its marks and legacy notes left out.
    pub words: Vec<&'a str>,
    /// The entries its supersedes marks name.
    pub supersedes: Vec<u64>,
    /// The entries its superseded-by marks name.
    pub superseded_by: Vec<u64>,
    /// The entries its legacy notes name. Only clause 3.6 reads them: a note
    /// is not a superseded-by mark, so clauses 3.8 and 3.9 never see it.
    pub noted: Vec<u64>,
    /// The date its title carries.
    pub date: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Label {
    Supersedes,
    SupersededBy,
    Note,
}

/// Each label's words; the first may end a longer word.
const LABELS: [(Label, &[&str]); 3] = [
    (Label::Supersedes, &["**Supersedes:**"]),
    (Label::SupersededBy, &["**Superseded", "by:**"]),
    (Label::Note, &["SUPERSEDED", "by"]),
];

/// An entry being read: its number, and the byte range of each of its lines
/// in the body with whether the line is fenced code.
type Lines = (u64, Vec<(Range<usize>, bool)>);

pub fn parse(body: &str) -> Body<'_> {
    let mut entries: Vec<Lines> = Vec::new();
    let mut outside = Vec::new();
    // The column the content of the entry being read starts at.
    let mut item: Option<usize> = None;
    let mut after_blank = true;
    let mut paragraph = false;
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;
    for raw in body.split_inclusive('\n') {
        let start = offset;
        offset += raw.len();
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let mut text = line;
        let (indent, rest) = indentation(line, 0);
        let blank = line.trim().is_empty();
        let contained = item.is_some_and(|column| indent >= column);
        // A fenced block inside an entry takes no lazy lines: the first line
        // indented less than the entry's content ends both.
        if fence.is_some() && item.is_some() && !blank && !contained {
            fence = None;
            item = None;
        }
        let mut fenced = fence.is_some();
        if let Some(open) = fence {
            if closes(line, open) {
                fence = None;
            }
        } else if blank {
            after_blank = true;
            paragraph = false;
        } else {
            if !contained {
                match marker(rest, indent) {
                    // CommonMark lets only a list starting at 1 interrupt a
                    // paragraph.
                    Some((number, after, column))
                        if item.is_some() || !paragraph || number == 1 =>
                    {
                        entries.push((number, Vec::new()));
                        item = Some(column);
                        text = after;
                    }
                    _ if item.is_some() && (after_blank || (indent <= 3 && interrupts(rest))) => {
                        item = None;
                        paragraph = false;
                    }
                    _ => {}
                }
            }
            fence = opens(text);
            fenced = fence.is_some();
            paragraph = !fenced
                && !(indent <= 3
                    && (heading(rest) || thematic(rest) || (paragraph && underline(rest))));
            after_blank = false;
        }
        match entries.last_mut().filter(|_| item.is_some()) {
            Some((_, lines)) => {
                let from = start + line.len() - text.len();
                lines.push((from..start + line.len(), fenced));
            }
            None => outside.extend(text.split_whitespace()),
        }
    }
    Body {
        entries: entries
            .into_iter()
            .map(|(number, lines)| read_entry(body, number, &lines))
            .collect(),
        outside,
    }
}

/// The column the white space opening `text` reaches from column `from`, tabs
/// stopping every four columns, and the text after it.
fn indentation(text: &str, from: usize) -> (usize, &str) {
    let mut column = from;
    for (index, byte) in text.bytes().enumerate() {
        match byte {
            b' ' => column += 1,
            b'\t' => column += 4 - column % 4,
            _ => return (column, &text[index..]),
        }
    }
    (column, "")
}

/// A top-level list marker's number, the text after it, and the column the
/// item's content starts at, for a line whose indentation `indent` is removed
/// from `text`.
fn marker(text: &str, indent: usize) -> Option<(u64, &str, usize)> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let rest = text[digits..].strip_prefix('.')?;
    if indent > 3
        || !(1..=9).contains(&digits)
        || !(rest.is_empty() || rest.starts_with([' ', '\t']))
    {
        return None;
    }
    let width = indent + digits + 1;
    let (column, content) = indentation(rest, width);
    // Content after five or more columns is indented code inside the item.
    let column = if content.is_empty() || column - width > 4 {
        width + 1
    } else {
        column
    };
    Some((text[..digits].parse().ok()?, rest, column))
}

/// Whether a line, its indentation removed, starts a block that ends an entry
/// it is not indented into.
fn interrupts(text: &str) -> bool {
    heading(text)
        || thematic(text)
        || opens(text).is_some()
        || text.starts_with('>')
        || (text.starts_with(['-', '*', '+']) && text[1..].starts_with([' ', '\t']))
}

fn thematic(text: &str) -> bool {
    let marks: Vec<u8> = text
        .bytes()
        .filter(|byte| !matches!(byte, b' ' | b'\t'))
        .collect();
    marks.len() >= 3
        && [b'-', b'*', b'_'].contains(&marks[0])
        && marks.iter().all(|byte| *byte == marks[0])
}

/// Whether a line, its indentation removed, can underline a setext heading.
fn underline(text: &str) -> bool {
    let text = text.trim_end();
    !text.is_empty() && (text.bytes().all(|b| b == b'=') || text.bytes().all(|b| b == b'-'))
}

fn heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    let rest = &line[hashes..];
    (1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with([' ', '\t']))
}

/// The fence a line opens: its character and length.
fn opens(text: &str) -> Option<(u8, usize)> {
    let text = text.trim_start();
    let byte = *text
        .as_bytes()
        .first()
        .filter(|b| [b'`', b'~'].contains(b))?;
    let length = text.bytes().take_while(|b| *b == byte).count();
    (length >= 3 && (byte == b'~' || !text[length..].contains('`'))).then_some((byte, length))
}

fn closes(line: &str, (byte, length): (u8, usize)) -> bool {
    let line = line.trim_start();
    let run = line.bytes().take_while(|b| *b == byte).count();
    run >= length && line[run..].trim().is_empty()
}

/// An entry's words, marks and legacy notes, from its lines.
fn read_entry<'a>(body: &'a str, number: u64, lines: &[(Range<usize>, bool)]) -> Entry<'a> {
    // The byte ranges in `body` of each word, and of each code span and
    // fenced line.
    let mut words = Vec::new();
    let mut code = Vec::new();
    let mut paragraph: Option<Range<usize>> = None;
    for (range, fenced) in lines {
        words.extend(split(body, range.clone()));
        if *fenced || body[range.clone()].trim().is_empty() {
            code.extend(
                paragraph
                    .take()
                    .into_iter()
                    .flat_map(|open| spans(body, open)),
            );
            if *fenced {
                code.push(range.clone());
            }
        } else {
            paragraph = Some(paragraph.map_or(range.clone(), |open| open.start..range.end));
        }
    }
    code.extend(paragraph.into_iter().flat_map(|open| spans(body, open)));
    let plain = |range: Range<usize>| {
        !code
            .iter()
            .any(|span| span.start < range.end && range.start < span.end)
    };

    let mut entry = Entry {
        number,
        words: Vec::new(),
        supersedes: Vec::new(),
        superseded_by: Vec::new(),
        noted: Vec::new(),
        date: None,
    };
    let mut index = 0;
    while index < words.len() {
        let found = mark(body, &words[index..]).filter(|(_, _, start, _, end)| plain(*start..*end));
        let Some((label, numbers, start, last, end)) = found else {
            entry.words.push(&body[words[index].clone()]);
            index += 1;
            continue;
        };
        if words[index].start < start {
            entry.words.push(&body[words[index].start..start]);
        }
        match label {
            Label::Supersedes => entry.supersedes.extend(numbers),
            Label::SupersededBy => entry.superseded_by.extend(numbers),
            Label::Note => entry.noted.extend(numbers),
        }
        index += last;
        if end < words[index].end {
            words[index].start = end;
        } else {
            index += 1;
        }
    }
    entry.date = title_date(&entry.words.join(" ")).map(str::to_owned);
    entry
}

/// The first `YYYY-MM-DD` in the title of an entry's text, the bold run that
/// opens it.
fn title_date(text: &str) -> Option<&str> {
    let title = title(text)?;
    let at = title.as_bytes().windows(10).position(|window| {
        window.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
    })?;
    Some(&title[at..at + 10])
}

/// The bold run opening `text`, up to the first `**` outside code spans and
/// not preceded by white space. A `**` preceded by white space, met first,
/// opens a nested run and leaves no title.
fn title(text: &str) -> Option<&str> {
    text.strip_prefix("**")?;
    let code = code_spans(text);
    let mut index = 2;
    while index < text.len() {
        if let Some(span) = code.iter().find(|span| span.start == index) {
            index = span.end;
            continue;
        }
        if text.as_bytes()[index..].starts_with(b"**") {
            if text[..index].ends_with(char::is_whitespace) {
                return None;
            }
            return Some(&text[2..index]);
        }
        index += 1;
    }
    None
}

/// The mark or legacy note beginning in the first of `words`, given as byte
/// ranges in `body`: its label, the numbers it names, the byte it begins at,
/// and the index of the word it ends in with the byte it ends at.
fn mark(body: &str, words: &[Range<usize>]) -> Option<(Label, Vec<u64>, usize, usize, usize)> {
    LABELS.into_iter().find_map(|(label, parts)| {
        let before = body[words[0].clone()].strip_suffix(parts[0])?;
        let labelled = words
            .get(1..parts.len())?
            .iter()
            .zip(&parts[1..])
            .all(|(word, part)| body[word.clone()] == **part);
        if !labelled {
            return None;
        }
        let (numbers, last, end) = list(body, &words[parts.len()..], label == Label::Note)?;
        Some((
            label,
            numbers,
            words[0].start + before.len(),
            parts.len() + last,
            end,
        ))
    })
}

/// The numbers a list opening `words` names, the index of the word it ends in
/// and the byte it ends at: after the full stop that ends a mark's list, or
/// after a legacy note's last number and any full stop directly after it.
fn list(body: &str, words: &[Range<usize>], note: bool) -> Option<(Vec<u64>, usize, usize)> {
    let mut numbers: Vec<u64> = Vec::new();
    // Where a legacy note ends if no number follows.
    let mut last = None;
    'words: for (index, word) in words.iter().enumerate() {
        let mut at = word.start;
        loop {
            let digits = body[at..word.end]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            if digits == 0 {
                break 'words;
            }
            numbers.push(body[at..at + digits].parse().ok()?);
            at += digits;
            last = Some((index, at));
            match body.as_bytes()[at..word.end].first() {
                Some(b'.') => return Some((numbers, index, at + 1)),
                Some(b',') if at + 1 == word.end => break,
                Some(b',') => at += 1,
                _ if note => break 'words,
                _ => return None,
            }
        }
    }
    last.filter(|_| note)
        .map(|(index, at)| (numbers, index, at))
}

/// The byte ranges of the words in `range` of `body`.
fn split(body: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut start = None;
    let text = &body[range.clone()];
    for (index, character) in text.char_indices().chain([(text.len(), ' ')]) {
        match (character.is_whitespace(), start) {
            (true, Some(from)) => {
                words.push(range.start + from..range.start + index);
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    words
}

/// The byte ranges in `body` of the code spans of the paragraph at `range`.
fn spans(body: &str, range: Range<usize>) -> Vec<Range<usize>> {
    code_spans(&body[range.clone()])
        .into_iter()
        .map(|span| range.start + span.start..range.start + span.end)
        .collect()
}

/// The byte ranges of a paragraph's code spans, their backticks included: a
/// run of backticks, its first not escaped, up to the next run of the same
/// length.
fn code_spans(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let run = |from: usize| bytes[from..].iter().take_while(|b| **b == b'`').count();
    let mut spans = Vec::new();
    let mut index = 0;
    let mut escaped = false;
    while index < bytes.len() {
        if bytes[index] != b'`' || escaped {
            escaped = bytes[index] == b'\\' && !escaped;
            index += 1;
            continue;
        }
        let length = run(index);
        let mut search = index + length;
        let mut end = None;
        while let Some(found) = bytes[search..].iter().position(|b| *b == b'`') {
            let at = search + found;
            let closing = run(at);
            if closing == length {
                end = Some(at + closing);
                break;
            }
            search = at + closing;
        }
        match end {
            Some(end) => {
                spans.push(index..end);
                index = end;
            }
            None => index += length,
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbers(body: &Body) -> Vec<u64> {
        body.entries.iter().map(|entry| entry.number).collect()
    }

    #[test]
    fn paragraphs_between_entries_are_outside_them() {
        let body = parse(
            "# Register\n\nIntro text.\n\n1. **First.** One.\n\nA note between\nentries.\n\n\
             2. **Second.** Two.\n",
        );
        assert_eq!(numbers(&body), [1, 2]);
        assert_eq!(body.entries[0].words, ["**First.**", "One."]);
        assert_eq!(
            body.outside,
            [
                "#", "Register", "Intro", "text.", "A", "note", "between", "entries."
            ]
        );
    }

    #[test]
    fn an_indented_numbered_line_stays_inside_its_entry() {
        let body = parse("1. Keep the registry. It is read by step\n   2. of the bootstrap.\n");
        assert_eq!(numbers(&body), [1]);
        assert_eq!(body.entries[0].words.last(), Some(&"bootstrap."));
    }

    #[test]
    fn indented_content_after_a_blank_line_stays_inside_its_entry() {
        let body = parse("1. Ruling.\n\n    1. A sub-item.\n    - A bullet.\n2. Next.\n");
        assert_eq!(numbers(&body), [1, 2]);
        assert_eq!(
            body.entries[0].words,
            ["Ruling.", "1.", "A", "sub-item.", "-", "A", "bullet."]
        );
    }

    #[test]
    fn consecutive_entries_need_no_blank_line() {
        let body = parse("1. One.\n2. Two.\n3. Three.\n");
        assert_eq!(numbers(&body), [1, 2, 3]);
        assert!(body.outside.is_empty());
    }

    #[test]
    fn a_heading_ends_an_entry() {
        let body = parse("1. One.\n## Later\n");
        assert_eq!(body.entries[0].words, ["One."]);
        assert_eq!(body.outside, ["##", "Later"]);
    }

    #[test]
    fn only_a_1_interrupts_a_paragraph_outside_entries() {
        let body = parse("Read by step\n2. of the bootstrap.\n");
        assert!(body.entries.is_empty());
        let body = parse("A list follows:\n1. One.\n");
        assert_eq!(numbers(&body), [1]);
        assert_eq!(body.outside, ["A", "list", "follows:"]);
    }

    #[test]
    fn a_repeated_number_is_kept_in_order() {
        let body = parse("1. One.\n2. Two.\n1. Again.\n");
        assert_eq!(numbers(&body), [1, 2, 1]);
    }

    #[test]
    fn marks_leave_the_words_and_name_their_entries() {
        let body = parse(
            "3. **Title (2026-10-17).** **Supersedes:** 1, 2. Ruling. **Superseded by:** 4.\n",
        );
        let entry = &body.entries[0];
        assert_eq!(entry.words, ["**Title", "(2026-10-17).**", "Ruling."]);
        assert_eq!(entry.supersedes, [1, 2]);
        assert_eq!(entry.superseded_by, [4]);
    }

    #[test]
    fn a_mark_wraps_across_lines_and_any_white_space() {
        let body = parse(
            "332. **A stamp.** **Superseded\n     by:**\t324,\n     330. **Supersedes:**\u{a0}5.\n",
        );
        let entry = &body.entries[0];
        assert_eq!(numbers(&body), [332]);
        assert_eq!(entry.superseded_by, [324, 330]);
        assert_eq!(entry.supersedes, [5]);
        assert_eq!(entry.words, ["**A", "stamp.**"]);
    }

    #[test]
    fn several_marks_of_one_kind_add_up() {
        let body = parse("9. **Supersedes:** 1. Text. **Supersedes:** 2, 3.\n");
        assert_eq!(body.entries[0].supersedes, [1, 2, 3]);
        assert_eq!(body.entries[0].words, ["Text."]);
    }

    #[test]
    fn a_label_without_a_number_list_is_prose() {
        for text in [
            "5. **Supersedes:** N; text.\n",
            "5. **Supersedes:** 1, text.\n",
            "5. **Supersedes:** 1\n",
            "5. **Supersedes decision** 1.\n",
        ] {
            let body = parse(text);
            let entry = &body.entries[0];
            assert!(entry.supersedes.is_empty(), "{text}");
            assert_eq!(entry.words.join(" "), text[3..].trim_end(), "{text}");
        }
    }

    #[test]
    fn a_label_outside_any_entry_is_prose() {
        let body = parse("Intro. **Supersedes:** 1.\n");
        assert!(body.entries.is_empty());
        assert_eq!(body.outside, ["Intro.", "**Supersedes:**", "1."]);
    }

    #[test]
    fn code_spans_hide_marks_and_keep_their_words() {
        let body = parse(
            "5. A newer entry carries `**Supersedes:** 3.` and the older\n   \
             `**Superseded\n   by:** 5.` stamp; ``a ` **Supersedes:** 1.`` too.\n",
        );
        let entry = &body.entries[0];
        assert!(entry.supersedes.is_empty());
        assert!(entry.superseded_by.is_empty());
        assert_eq!(entry.words.len(), 18);
    }

    #[test]
    fn an_unclosed_backtick_hides_nothing() {
        let body = parse("5. A stray ` then **Supersedes:** 3.\n");
        assert_eq!(body.entries[0].supersedes, [3]);
    }

    #[test]
    fn a_fenced_block_hides_marks_and_keeps_its_words() {
        let body = parse(
            "5. Ruling:\n\n   ```text\n   2. **Supersedes:** 3.\n\n   7. not an entry\n   ```\n\n   \
             After. **Supersedes:** 4.\n6. Next.\n",
        );
        assert_eq!(numbers(&body), [5, 6]);
        let entry = &body.entries[0];
        assert_eq!(entry.supersedes, [4]);
        assert_eq!(
            entry.words,
            [
                "Ruling:",
                "```text",
                "2.",
                "**Supersedes:**",
                "3.",
                "7.",
                "not",
                "an",
                "entry",
                "```",
                "After."
            ]
        );
    }

    #[test]
    fn a_line_opening_with_a_multibyte_character_opens_no_fence() {
        let body = parse("1. Ruling.\n   ✅ Implemented. **Supersedes:** 0.\n");
        assert_eq!(body.entries[0].supersedes, [0]);
    }

    #[test]
    fn a_comma_list_without_spaces_is_a_mark() {
        let body = parse(
            "1. Use TOML for the registry. **Superseded by:** 3.\n\
             2. Keep the registry in one file. **Superseded by:** 3.\n\
             3. Use one JSON file per kind. **Supersedes:** 1,2.\n",
        );
        let entry = &body.entries[2];
        assert_eq!(entry.supersedes, [1, 2]);
        assert_eq!(entry.words, ["Use", "one", "JSON", "file", "per", "kind."]);
        assert_eq!(
            parse("4. **Supersedes:** 1,2, 3.\n").entries[0].supersedes,
            [1, 2, 3]
        );
        for text in ["4. **Supersedes:** 1,,2.\n", "4. **Supersedes:** 1,.\n"] {
            assert!(parse(text).entries[0].supersedes.is_empty(), "{text}");
        }
    }

    #[test]
    fn a_block_at_column_0_under_an_entry_line_ends_the_entry() {
        for (block, outside) in [
            (
                "- one file per kind",
                vec!["-", "one", "file", "per", "kind"],
            ),
            (
                "> **Superseded by:** 2.",
                vec![">", "**Superseded", "by:**", "2."],
            ),
            (
                "```\n**Superseded by:** 2.\n```",
                vec!["```", "**Superseded", "by:**", "2.", "```"],
            ),
            ("***", vec!["***"]),
        ] {
            let text = format!("1. Use TOML for the registry:\n{block}\n");
            let body = parse(&text);
            let entry = &body.entries[0];
            assert_eq!(
                entry.words,
                ["Use", "TOML", "for", "the", "registry:"],
                "{block}"
            );
            assert!(entry.superseded_by.is_empty(), "{block}");
            assert_eq!(body.outside, outside, "{block}");
        }
    }

    #[test]
    fn an_unindented_line_continuing_the_entry_text_stays_inside_it() {
        let body = parse("1. Use TOML for the\nregistry.\n");
        assert_eq!(
            body.entries[0].words,
            ["Use", "TOML", "for", "the", "registry."]
        );
    }

    #[test]
    fn an_entry_may_be_indented_up_to_three_spaces() {
        for text in [
            "1. Use TOML. **Superseded by:** 2.\n 2. Use JSON. **Supersedes:** 1.\n",
            " 1. Use TOML. **Superseded by:** 2.\n 2. Use JSON. **Supersedes:** 1.\n",
            "   1. Use TOML. **Superseded by:** 2.\n2. Use JSON. **Supersedes:** 1.\n",
        ] {
            let body = parse(text);
            assert_eq!(numbers(&body), [1, 2], "{text}");
            assert_eq!(body.entries[0].superseded_by, [2], "{text}");
            assert_eq!(body.entries[1].supersedes, [1], "{text}");
        }
        assert!(parse("    1. Indented code.\n").entries.is_empty());
    }

    #[test]
    fn a_line_indented_short_of_the_content_ends_the_entry_and_its_fence() {
        let body = parse(
            "1. Use TOML. **Superseded by:** 2.\n   ```\n   x\n2. Use JSON. **Supersedes:** 1.\n",
        );
        assert_eq!(numbers(&body), [1, 2]);
        assert_eq!(body.entries[0].superseded_by, [2]);
        assert_eq!(body.entries[1].supersedes, [1]);
    }

    #[test]
    fn a_paragraph_indented_short_of_the_content_after_a_blank_line_ends_the_entry() {
        let body = parse("10. Ruling.\n\n   **Superseded by:** 12.\n");
        assert!(body.entries[0].superseded_by.is_empty());
        assert_eq!(body.outside, ["**Superseded", "by:**", "12."]);
        let body = parse("10. Ruling.\n\n    **Superseded by:** 12.\n");
        assert_eq!(body.entries[0].superseded_by, [12]);
    }

    #[test]
    fn a_thematic_break_underline_or_closed_fence_outside_entries_is_not_a_paragraph() {
        for between in [
            "\n---\n",
            "Intro\n---\n",
            "Intro\n===\n",
            "```\ncode\n```\n",
        ] {
            let text = format!(
                "1. Use TOML. **Superseded by:** 2.\n\n{between}2. Use JSON. **Supersedes:** 1.\n"
            );
            let body = parse(&text);
            assert_eq!(numbers(&body), [1, 2], "{between}");
            assert_eq!(body.entries[1].supersedes, [1], "{between}");
        }
        assert!(parse("Intro\n===\n===\n2. Text.\n").entries.is_empty());
    }

    #[test]
    fn an_escaped_backtick_opens_no_code_span() {
        let body = parse(
            "1. Use TOML. **Superseded by:** 2.\n\
             2. Write \\` for a backtick. **Supersedes:** 1. Then `x`.\n",
        );
        let entry = &body.entries[1];
        assert_eq!(entry.supersedes, [1]);
        assert_eq!(
            entry.words,
            ["Write", "\\`", "for", "a", "backtick.", "Then", "`x`."]
        );
        assert!(
            parse("2. A \\\\`**Supersedes:** 1.` span.\n").entries[0]
                .supersedes
                .is_empty()
        );
    }

    #[test]
    fn the_title_date_is_the_first_date_in_the_bold_run_opening_the_text() {
        assert_eq!(
            title_date("**Ruling (2026-10-26, after 2026-10-25).** Text."),
            Some("2026-10-26")
        );
        assert_eq!(
            title_date("**Lint reads `**Supersedes:**` marks (2026-10-26).** Text."),
            Some("2026-10-26")
        );
        for text in [
            "Text **2026-10-26** more.",
            "**Ruling (2026-10-26) over **TOML**.** Text.",
            "**Ruling (2026-10-26). Text, as **ruled**.",
            "**Ruling.** Recorded 2026-10-26.",
            "Ruling recorded 2026-10-26.",
            "**Ruling (2026-10-26).",
            "**Ruling (2026-1-26).**",
            "**Ruling (20261026).**",
            "****",
            "",
        ] {
            assert_eq!(title_date(text), None, "{text}");
        }
    }

    #[test]
    fn an_entry_takes_its_date_from_a_title_wrapped_across_lines() {
        let body = parse("5. **A ruling wrapped\n   over two lines (2026-10-26).** Text.\n");
        assert_eq!(body.entries[0].date.as_deref(), Some("2026-10-26"));
    }

    #[test]
    fn a_title_with_a_nested_bold_run_carries_no_date() {
        let body = parse("5. **A ruling **and** its date (2026-10-26).** Text.\n");
        assert_eq!(body.entries[0].date, None);
    }

    #[test]
    fn a_mark_before_the_title_is_not_the_title() {
        let body = parse("5. **Supersedes:** 3. **A ruling (2026-10-26).** Text.\n");
        assert_eq!(body.entries[0].date.as_deref(), Some("2026-10-26"));
    }

    #[test]
    fn a_legacy_note_names_entries_apart_from_the_marks() {
        let body = parse("1. One. SUPERSEDED by 2,\n   3. **Superseded by:** 4.\n");
        let entry = &body.entries[0];
        assert_eq!(entry.noted, [2, 3]);
        assert_eq!(entry.superseded_by, [4]);
        assert_eq!(entry.words, ["One."]);
    }

    #[test]
    fn a_legacy_note_takes_a_full_stop_directly_after_its_last_number() {
        for (text, words) in [
            ("1. One. SUPERSEDED by 2. Two.\n", vec!["One.", "Two."]),
            (
                "1. One. SUPERSEDED by 2 (in part).\n",
                vec!["One.", "(in", "part)."],
            ),
            (
                "1. One. SUPERSEDED by 2, then 3.\n",
                vec!["One.", ",", "then", "3."],
            ),
        ] {
            let entry = &parse(text).entries[0];
            assert_eq!(entry.noted, [2], "{text}");
            assert_eq!(entry.words, words, "{text}");
        }
        assert!(
            parse("1. SUPERSEDED by none.\n").entries[0]
                .noted
                .is_empty()
        );
    }

    #[test]
    fn a_mark_or_note_inside_a_word_leaves_the_rest_of_it_a_word() {
        let body = parse(
            "1. One.\n   - _SUPERSEDED by 2 (this bullet only)._\n   \
             - _**Superseded by:** 3.)_ (**Supersedes:** 0.)\n",
        );
        let entry = &body.entries[0];
        assert_eq!(entry.noted, [2]);
        assert_eq!(entry.superseded_by, [3]);
        assert_eq!(entry.supersedes, [0]);
        assert_eq!(
            entry.words,
            [
                "One.", "-", "_", "(this", "bullet", "only)._", "-", "_", ")_", "(", ")"
            ]
        );
    }

    #[test]
    fn code_hides_a_legacy_note() {
        for text in [
            "1. One. `SUPERSEDED by 2.`\n",
            "1. One. ``x SUPERSEDED\n   by 2. y``\n",
            "1. One.\n\n   ```\n   SUPERSEDED by 2.\n   ```\n",
        ] {
            assert!(parse(text).entries[0].noted.is_empty(), "{text}");
        }
    }

    #[test]
    fn words_split_on_unicode_white_space() {
        let body = parse("1. Use\u{a0}TOML\u{2003}now.\n");
        assert_eq!(body.entries[0].words, ["Use", "TOML", "now."]);
    }
}
