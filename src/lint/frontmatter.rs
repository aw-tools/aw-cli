//! Frontmatter: a block opened by a `---` first line and closed by the next
//! `---` line, holding flat `key: value` lines.

use super::model::Header;
use std::collections::BTreeMap;

const DELIMITER: &str = "---";

/// Split a file into its frontmatter and its body. An absent or unclosed block
/// leaves the whole text as the body.
pub fn parse(text: &str) -> (Header, &str) {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next().filter(|line| line_content(line) == DELIMITER) else {
        return (Header::Absent, text);
    };
    let mut fields = BTreeMap::new();
    let mut consumed = first.len();
    for line in lines {
        consumed += line.len();
        let content = line_content(line);
        if content == DELIMITER {
            return (Header::Fields(fields), &text[consumed..]);
        }
        if let Some((key, value)) = content.split_once(':') {
            fields
                .entry(key.trim().to_owned())
                .or_insert_with(|| value.trim().to_owned());
        }
    }
    (Header::Unclosed, text)
}

fn line_content(line: &str) -> &str {
    line.strip_suffix('\n').unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> Header {
        Header::Fields(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }

    #[test]
    fn reads_fields_and_the_body_after_the_block() {
        let (header, body) = parse("---\nkind: plan\nstatus:  open \n---\n\n# Title\n");
        assert_eq!(header, fields(&[("kind", "plan"), ("status", "open")]));
        assert_eq!(body, "\n# Title\n");
    }

    #[test]
    fn the_first_occurrence_of_a_key_wins() {
        let (header, _) = parse("---\nkind: plan\nkind: spec\n---\n");
        assert_eq!(header, fields(&[("kind", "plan")]));
    }

    #[test]
    fn a_value_keeps_its_own_colons() {
        let (header, _) = parse("---\nsuperseded_by: notes/a: b.md\n---\n");
        assert_eq!(header, fields(&[("superseded_by", "notes/a: b.md")]));
    }

    #[test]
    fn a_file_not_opening_with_the_delimiter_has_none() {
        for text in [
            "# Title\n---\nkind: plan\n---\n",
            "",
            " ---\nkind: plan\n---\n",
        ] {
            assert_eq!(parse(text), (Header::Absent, text));
        }
    }

    #[test]
    fn an_unclosed_block_is_told_apart_from_an_absent_one() {
        let text = "---\nkind: plan\nstatus: open\n";
        assert_eq!(parse(text), (Header::Unclosed, text));
    }

    #[test]
    fn a_closing_delimiter_at_end_of_file_closes_the_block() {
        assert_eq!(
            parse("---\nkind: plan\n---"),
            (fields(&[("kind", "plan")]), "")
        );
    }
}
