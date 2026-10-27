//! Entries and supersession marks in a binding artefact's body, in the form
//! the `supersession-marks` proposal's fixtures fix.
//!
//! An entry is an item of a numbered list at the top level of the body,
//! identified by the number written before it. A mark is `**Supersedes:**` or
//! `**Superseded by:**`, then entry numbers separated by commas and ending in a
//! full stop, read word by word so any white space may wrap it. Code spans and
//! fenced blocks hide marks; their words stay.

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
    /// The words after its number, its marks left out.
    pub words: Vec<&'a str>,
    /// The entries its supersedes marks name.
    pub supersedes: Vec<u64>,
    /// The entries its superseded-by marks name.
    pub superseded_by: Vec<u64>,
}

const SUPERSEDES: &[&str] = &["**Supersedes:**"];
const SUPERSEDED_BY: &[&str] = &["**Superseded", "by:**"];

/// An entry being read: its number, and the byte range of each of its lines
/// in the body with whether the line is fenced code.
type Lines = (u64, Vec<(Range<usize>, bool)>);

pub fn parse(body: &str) -> Body<'_> {
    let mut entries: Vec<Lines> = Vec::new();
    let mut outside = Vec::new();
    let mut in_entry = false;
    let mut after_blank = true;
    let mut paragraph = false;
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;
    for raw in body.split_inclusive('\n') {
        let start = offset;
        offset += raw.len();
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let mut text = line;
        let mut fenced = fence.is_some();
        if let Some(open) = fence {
            if closes(line, open) {
                fence = None;
            }
        } else if line.trim().is_empty() {
            after_blank = true;
            paragraph = false;
        } else {
            if !line.starts_with([' ', '\t']) {
                match marker(line) {
                    // CommonMark lets only a list starting at 1 interrupt a
                    // paragraph.
                    Some((number, rest)) if in_entry || !paragraph || number == 1 => {
                        entries.push((number, Vec::new()));
                        in_entry = true;
                        text = rest;
                    }
                    _ if after_blank || heading(line) => in_entry = false,
                    _ => {}
                }
            }
            paragraph = !in_entry && !heading(line);
            after_blank = false;
            fence = opens(text);
            fenced = fence.is_some();
        }
        match entries.last_mut().filter(|_| in_entry) {
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

/// A column-0 list marker's number and the text after it.
fn marker(line: &str) -> Option<(u64, &str)> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let rest = line[digits..].strip_prefix('.')?;
    if !(1..=9).contains(&digits) || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    Some((line[..digits].parse().ok()?, rest))
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

/// An entry's words and marks, from its lines.
fn read_entry<'a>(body: &'a str, number: u64, lines: &[(Range<usize>, bool)]) -> Entry<'a> {
    // Each word, and whether it lies outside code.
    let mut words: Vec<(&str, bool)> = Vec::new();
    let mut paragraph: Option<Range<usize>> = None;
    for (range, fenced) in lines {
        let text = &body[range.clone()];
        if *fenced || text.trim().is_empty() {
            if let Some(range) = paragraph.take() {
                words.extend(paragraph_words(&body[range]));
            }
            words.extend(text.split_whitespace().map(|word| (word, false)));
        } else {
            paragraph = Some(paragraph.map_or(range.clone(), |open| open.start..range.end));
        }
    }
    if let Some(range) = paragraph {
        words.extend(paragraph_words(&body[range]));
    }

    let mut entry = Entry {
        number,
        words: Vec::new(),
        supersedes: Vec::new(),
        superseded_by: Vec::new(),
    };
    let mut index = 0;
    while index < words.len() {
        let rest = &words[index..];
        let mark = [SUPERSEDES, SUPERSEDED_BY]
            .into_iter()
            .find(|label| labelled(rest, label))
            .and_then(|label| {
                let (numbers, count) = numbers(&rest[label.len()..])?;
                Some((label, numbers, label.len() + count))
            });
        if let Some((label, numbers, length)) = mark {
            if label == SUPERSEDES {
                entry.supersedes.extend(numbers);
            } else {
                entry.superseded_by.extend(numbers);
            }
            index += length;
        } else {
            entry.words.push(rest[0].0);
            index += 1;
        }
    }
    entry
}

fn labelled(words: &[(&str, bool)], label: &[&str]) -> bool {
    words.len() >= label.len()
        && words
            .iter()
            .zip(label)
            .all(|((word, plain), part)| *plain && word == part)
}

/// The numbers a mark's list names, and how many words the list takes.
fn numbers(words: &[(&str, bool)]) -> Option<(Vec<u64>, usize)> {
    let mut numbers = Vec::new();
    for (word, plain) in words {
        if !plain {
            return None;
        }
        if let Some(number) = word.strip_suffix(',').and_then(number) {
            numbers.push(number);
            continue;
        }
        numbers.push(word.strip_suffix('.').and_then(number)?);
        let count = numbers.len();
        return Some((numbers, count));
    }
    None
}

fn number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The words of a paragraph, each with whether it lies wholly outside code
/// spans.
fn paragraph_words(text: &str) -> Vec<(&str, bool)> {
    let code = code_spans(text);
    let mut words = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices().chain([(text.len(), ' ')]) {
        match (character.is_whitespace(), start) {
            (true, Some(from)) => {
                let plain = !code
                    .iter()
                    .any(|span| span.start < index && from < span.end);
                words.push((&text[from..index], plain));
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    words
}

/// The byte ranges of a paragraph's code spans, their backticks included: a
/// run of backticks up to the next run of the same length.
fn code_spans(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let run = |from: usize| bytes[from..].iter().take_while(|b| **b == b'`').count();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'`' {
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
            "5. Ruling:\n\n   ```text\n   2. **Supersedes:** 3.\n\n7. not an entry\n   ```\n\n   \
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
    fn words_split_on_unicode_white_space() {
        let body = parse("1. Use\u{a0}TOML\u{2003}now.\n");
        assert_eq!(body.entries[0].words, ["Use", "TOML", "now."]);
    }
}
