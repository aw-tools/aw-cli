//! The shape of `STATE.md`, read line by line as the script's awk reads it.

use super::day_number;
use crate::lint::model::{Caps, Layout, Model};
use crate::lint::rule::Rule;
use crate::lint::rules::OPEN;
use std::collections::BTreeSet;

const NEXT: &str = "Next";
const OPEN_ITEMS: &str = "Open items";
const DORMANT_ENGAGEMENTS: &str = "Dormant engagements";
/// The slug prefix marking an item whose subject has no engagement.
const NO_TOPIC: &str = "item-";

/// Which `STATE.md` section a line sits in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Other,
    Next,
    Items,
    Dormant,
}

/// The item field a continuation line adds to.
#[derive(Clone, Copy)]
enum Field {
    Now,
    Next,
    Blocked,
    Ledger,
}

/// One item under `## Open items`, as read so far.
struct Item<'a> {
    slug: &'a str,
    date: String,
    field: Option<Field>,
    now: usize,
    next: usize,
    blocked: usize,
    ledger: usize,
    no_topic: bool,
}

/// A finding in file order, or a `## Next` entry to resolve once every item
/// slug is known.
enum Event<'a> {
    Found(Rule, String),
    NextRef(&'a str),
}

/// One pass over `STATE.md`, line by line as the script's awk reads it.
pub(super) struct Shape<'a> {
    model: &'a Model,
    layout: &'a Layout,
    caps: Caps,
    today: i64,
    section: Section,
    item: Option<Item<'a>>,
    rollup: Option<(&'a str, usize)>,
    next_entries: usize,
    item_count: usize,
    events: Vec<Event<'a>>,
    items: BTreeSet<&'a str>,
    /// The engagement each topic-bearing item names, before any `/thread`.
    engagements: BTreeSet<&'a str>,
    rollups: BTreeSet<&'a str>,
}

impl<'a> Shape<'a> {
    pub(super) fn check(
        text: &'a str,
        model: &'a Model,
        layout: &'a Layout,
        caps: Caps,
        today: i64,
    ) -> Vec<(Rule, String)> {
        let mut shape = Shape {
            model,
            layout,
            caps,
            today,
            section: Section::Other,
            item: None,
            rollup: None,
            next_entries: 0,
            item_count: 0,
            events: Vec::new(),
            items: BTreeSet::new(),
            engagements: BTreeSet::new(),
            rollups: BTreeSet::new(),
        };
        let mut findings = Vec::new();
        if text.lines().any(|line| {
            line.trim_start_matches(|c: char| c.is_ascii_whitespace())
                .starts_with("```")
        }) {
            findings.push((
                Rule::StateFenced,
                "fenced code block; verbatim output belongs in the owning ledger".to_owned(),
            ));
        }
        // The shape rules run only inside the sections they parse, so a renamed
        // heading would silently switch them off.
        for title in [OPEN_ITEMS, NEXT] {
            if !text.lines().any(|line| heading(line) == Some(title)) {
                findings.push((
                    Rule::StateSectionMissing,
                    format!("no '## {title}' section, so its shape rules did not run"),
                ));
            }
        }
        for line in text.lines() {
            shape.line(line);
        }
        shape.flush_item();
        shape.flush_rollup();
        shape.finish(&mut findings);
        findings
    }

    fn line(&mut self, line: &'a str) {
        if let Some(title) = heading(line) {
            self.flush_item();
            self.flush_rollup();
            self.section = match title {
                NEXT => Section::Next,
                OPEN_ITEMS => Section::Items,
                DORMANT_ENGAGEMENTS => Section::Dormant,
                _ => Section::Other,
            };
            return;
        }
        match self.section {
            Section::Other => {}
            Section::Next => self.next_line(line),
            Section::Dormant => self.dormant_line(line),
            Section::Items => self.item_line(line),
        }
    }

    fn next_line(&mut self, line: &'a str) {
        if numbered(line) {
            self.next_entries += 1;
            self.events
                .push(Event::NextRef(tokens(line).nth(1).unwrap_or_default()));
        } else if !(continuation(line) && self.next_entries > 0 || blank(line)) {
            self.found(
                Rule::NextFreeText,
                "free text in '## Next' outside the ordered list".to_owned(),
            );
        }
    }

    fn dormant_line(&mut self, line: &'a str) {
        if rollup_bullet(line) {
            self.flush_rollup();
            let slug = tokens(line).nth(1).unwrap_or_default();
            let slug = slug.strip_suffix(':').unwrap_or(slug);
            self.rollup = Some((slug, tokens(line).count() - 1));
        } else if let (true, Some((_, count))) = (continuation(line), &mut self.rollup) {
            *count += tokens(line).count();
        } else if !blank(line) {
            self.found(
                Rule::RollupMalformed,
                "line under '## Dormant engagements' is not a roll-up bullet".to_owned(),
            );
        }
    }

    fn item_line(&mut self, line: &'a str) {
        if line.starts_with("### ") {
            self.flush_item();
            let mut words = tokens(line).skip(1);
            let slug = words.next().unwrap_or_default();
            let date = words.next().unwrap_or_default().replace(['(', ')'], "");
            self.item = (!slug.is_empty()).then_some(Item {
                slug,
                date,
                field: None,
                now: 0,
                next: 0,
                blocked: 0,
                ledger: 0,
                no_topic: false,
            });
            return;
        }
        let Some(item) = &mut self.item else {
            return;
        };
        let count = tokens(line).count();
        let field = [
            ("- Now:", Field::Now),
            ("- Next:", Field::Next),
            ("- Blocked:", Field::Blocked),
            ("- Ledger:", Field::Ledger),
        ]
        .into_iter()
        .find(|(prefix, _)| line.starts_with(prefix));
        let (field, words) = if let Some((prefix, field)) = field {
            item.field = Some(field);
            if let Field::Ledger = field {
                item.no_topic |=
                    line[prefix.len()..].trim_matches(|c: char| c.is_ascii_whitespace()) == "—";
            }
            (field, count.saturating_sub(2))
        } else if let (true, Some(field)) = (continuation(line), item.field) {
            (field, count)
        } else {
            if !blank(line) {
                let slug = item.slug;
                self.found(
                    Rule::ItemFreeText,
                    format!("item '{slug}' holds free text outside the four fields"),
                );
            }
            return;
        };
        match field {
            Field::Now => item.now += words,
            Field::Next => item.next += words,
            Field::Blocked => item.blocked += words,
            Field::Ledger => item.ledger += 1,
        }
    }

    fn flush_item(&mut self) {
        let Some(item) = self.item.take() else {
            return;
        };
        let slug = item.slug;
        if [item.now, item.next, item.blocked, item.ledger].contains(&0) {
            self.found(
                Rule::ItemFieldMissing,
                format!("item '{slug}' misses a field; it needs Now, Next, Blocked and Ledger"),
            );
        }
        let cap = self.caps.field_tokens;
        for (name, count) in [
            ("Now", item.now),
            ("Next", item.next),
            ("Blocked", item.blocked),
        ] {
            if count > cap {
                self.found(
                    Rule::ItemFieldLong,
                    format!("item '{slug}': {name} exceeds {cap} tokens ({count})"),
                );
            }
        }
        if item.ledger > 1 {
            self.found(
                Rule::ItemFieldLong,
                format!("item '{slug}': Ledger exceeds 1 line"),
            );
        }
        match day_number(&item.date) {
            None => self.found(
                Rule::ItemUndated,
                format!("item '{slug}' carries no (YYYY-MM-DD) last-rewritten date in its heading"),
            ),
            Some(day) if self.today - day > self.caps.stale_days => {
                let age = self.today - day;
                self.found(
                    Rule::ItemStale,
                    format!(
                        "item '{slug}' was last rewritten {age} days ago; re-read it against \
                         reality, then rewrite or confirm it"
                    ),
                );
            }
            Some(_) => {}
        }
        if slug.starts_with(NO_TOPIC) {
            if !item.no_topic {
                self.found(
                    Rule::ItemPrefix,
                    format!("item '{slug}' takes the {NO_TOPIC} prefix without 'Ledger: —'"),
                );
            }
        } else if item.no_topic {
            self.found(
                Rule::ItemPrefix,
                format!(
                    "item '{slug}' has 'Ledger: —' on an unprefixed slug; a subject with no \
                     engagement takes the {NO_TOPIC} prefix"
                ),
            );
        } else {
            let engagement = slug.split('/').next().unwrap_or(slug);
            self.engagements.insert(engagement);
            match self.status(engagement) {
                Some(OPEN) if self.layout.dormant.contains(engagement) => self.found(
                    Rule::ItemEngagement,
                    format!(
                        "item '{engagement}' names a dormant engagement, which takes one roll-up \
                         line under '## {DORMANT_ENGAGEMENTS}' instead"
                    ),
                ),
                Some(OPEN) => {}
                status => self.found(
                    Rule::ItemEngagement,
                    format!(
                        "item '{engagement}' names an engagement that is not open (status: {}); \
                         closing an engagement removes its item",
                        status.unwrap_or("unregistered")
                    ),
                ),
            }
        }
        self.items.insert(slug);
        self.item_count += 1;
    }

    fn flush_rollup(&mut self) {
        let Some((slug, count)) = self.rollup.take() else {
            return;
        };
        let cap = self.caps.rollup_tokens;
        if count > cap {
            self.found(
                Rule::RollupLong,
                format!("roll-up '{slug}' exceeds {cap} tokens ({count})"),
            );
        }
        match self.status(slug) {
            Some(OPEN) if self.layout.dormant.contains(slug) => {}
            Some(OPEN) => self.found(
                Rule::RollupEngagement,
                format!(
                    "roll-up '{slug}' names an engagement the registry does not declare dormant; \
                     an active engagement holds an item"
                ),
            ),
            status => self.found(
                Rule::RollupEngagement,
                format!(
                    "roll-up '{slug}' names an engagement that is not open (status: {}); closing \
                     an engagement removes its line",
                    status.unwrap_or("unregistered")
                ),
            ),
        }
        self.rollups.insert(slug);
    }

    /// The file-wide caps, each `## Next` entry against the item slugs, and
    /// every open engagement against the items and roll-up lines.
    fn finish(self, findings: &mut Vec<(Rule, String)>) {
        let item_count = self.item_count;
        for event in self.events {
            findings.push(match event {
                Event::Found(rule, message) => (rule, message),
                Event::NextRef(slug) if self.items.contains(slug) => continue,
                Event::NextRef(slug) => (
                    Rule::NextDangling,
                    format!("'## {NEXT}' names '{slug}', which is no item slug"),
                ),
            });
        }
        if item_count > self.caps.max_items {
            findings.push((
                Rule::ItemsOverCap,
                format!(
                    "{item_count} items exceed the max_items cap ({})",
                    self.caps.max_items
                ),
            ));
        }
        if self.next_entries > self.caps.next_entries {
            findings.push((
                Rule::NextOverCap,
                format!(
                    "'## {NEXT}' holds {} entries, over the next_entries cap ({})",
                    self.next_entries, self.caps.next_entries
                ),
            ));
        }
        for unit in self.model.units.iter().filter(|unit| unit.state == OPEN) {
            let name = unit.name.as_str();
            if self.layout.dormant.contains(name) {
                if !self.rollups.contains(name) {
                    findings.push((
                        Rule::EngagementUnlisted,
                        format!(
                            "dormant open engagement '{name}' has no roll-up line under \
                             '## {DORMANT_ENGAGEMENTS}'"
                        ),
                    ));
                }
            } else if !self.engagements.contains(name) {
                findings.push((
                    Rule::EngagementUnlisted,
                    format!("open engagement '{name}' has no item under '## {OPEN_ITEMS}'"),
                ));
            }
        }
    }

    fn found(&mut self, rule: Rule, message: String) {
        self.events.push(Event::Found(rule, message));
    }

    fn status(&self, engagement: &str) -> Option<&'a str> {
        let model: &'a Model = self.model;
        model
            .units
            .iter()
            .find(|unit| unit.name == engagement)
            .map(|unit| unit.state.as_str())
    }
}

/// A `## ` heading's title, less any `<n>. ` numbering.
fn heading(line: &str) -> Option<&str> {
    let title = line.strip_prefix("## ")?;
    let numbered = title
        .split_once(". ")
        .filter(|(number, _)| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()));
    Some(numbered.map_or(title, |(_, rest)| rest))
}

/// An entry of an ordered list: digits, a dot and a space.
fn numbered(line: &str) -> bool {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(". ")
}

/// A roll-up bullet: `- <slug>:`, the slug lowercase, digits and hyphens.
fn rollup_bullet(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("- ") else {
        return false;
    };
    let slug = rest
        .bytes()
        .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        .count();
    slug > 0 && rest[slug..].starts_with(':')
}

/// An indented line with something on it.
fn continuation(line: &str) -> bool {
    line.starts_with(|c: char| c.is_ascii_whitespace()) && !blank(line)
}

fn blank(line: &str) -> bool {
    line.bytes().all(|b| b.is_ascii_whitespace())
}

/// A line's tokens: its fields split on spaces and tabs, as awk splits them.
fn tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split([' ', '\t']).filter(|word| !word.is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{CLEAN, REGISTRY_TEXT, findings, findings_in};
    use super::*;
    use crate::lint::rule::Severity;

    fn rules(state: &str) -> Vec<Rule> {
        findings(state).into_iter().map(|(rule, _)| rule).collect()
    }

    /// `CLEAN` with the first `from` replaced by `to`.
    fn edited(from: &str, to: &str) -> String {
        assert!(CLEAN.contains(from), "{from}");
        CLEAN.replacen(from, to, 1)
    }

    /// `CLEAN` with one more item before the dormant section.
    fn with_item(slug: &str) -> String {
        edited(
            "## 3. Dormant",
            &format!(
                "### {slug} (2026-10-01)\n\n- Now: a\n- Next: b\n- Blocked: c\n- Ledger: d\n\n\
                 ## 3. Dormant"
            ),
        )
    }

    #[test]
    fn the_state_file_holds_no_fence() {
        assert_eq!(
            rules(&edited("# State\n", "# State\n\n  ```sh\n")),
            [Rule::StateFenced]
        );
    }

    #[test]
    fn the_next_and_open_items_sections_exist() {
        assert_eq!(
            rules(&edited("## 1. Next", "## Priorities")),
            [Rule::StateSectionMissing]
        );
        assert_eq!(rules(&edited("## 2. Open items", "## Open items")), []);
    }

    #[test]
    fn next_holds_only_its_ordered_list() {
        assert_eq!(
            rules(&edited(
                "## 1. Next\n",
                "## 1. Next\n\nPreamble.\n  indented\n"
            )),
            [Rule::NextFreeText, Rule::NextFreeText]
        );
    }

    #[test]
    fn next_is_capped() {
        assert_eq!(
            findings(&edited("   carried on\n", "   carried on\n3. alpha\n")),
            [(
                Rule::NextOverCap,
                "'## Next' holds 3 entries, over the next_entries cap (2)".to_owned()
            )]
        );
    }

    #[test]
    fn next_names_an_item_slug() {
        assert_eq!(
            findings(&edited("1. alpha", "1. alpha/thread")),
            [(
                Rule::NextDangling,
                "'## Next' names 'alpha/thread', which is no item slug".to_owned()
            )]
        );
    }

    #[test]
    fn dormant_engagements_holds_only_roll_up_bullets() {
        // The continuation line has no bullet to continue, so it fails too.
        assert_eq!(
            rules(&edited("- gamma:", "* gamma:")),
            [
                Rule::RollupMalformed,
                Rule::RollupMalformed,
                Rule::EngagementUnlisted
            ]
        );
    }

    #[test]
    fn a_roll_up_line_is_capped() {
        // The marker is not a token; the slug is.
        assert_eq!(
            findings(&edited("the spring.", &"word ".repeat(23))),
            [(
                Rule::RollupLong,
                "roll-up 'gamma' exceeds 25 tokens (26)".to_owned()
            )]
        );
    }

    #[test]
    fn a_roll_up_names_a_dormant_open_engagement() {
        for slug in ["alpha", "beta", "omega"] {
            assert_eq!(
                rules(&edited(
                    "- gamma: resting until\n  the spring.\n",
                    &format!("- gamma: resting.\n- {slug}: resting.\n"),
                )),
                [Rule::RollupEngagement],
                "{slug}"
            );
        }
    }

    #[test]
    fn an_item_holds_only_its_four_fields() {
        assert_eq!(
            rules(&edited("- Next: four\n", "- Next: four\nA stray line.\n")),
            [Rule::ItemFreeText]
        );
        assert_eq!(
            rules(&edited(
                "### alpha (2026-09-02)\n",
                "### alpha (2026-09-02)\n\n  indented first\n"
            )),
            [Rule::ItemFreeText]
        );
    }

    #[test]
    fn an_item_holds_every_field() {
        assert_eq!(
            findings(&edited("- Blocked: z\n", "- Blocked:\n")),
            [(
                Rule::ItemFieldMissing,
                "item 'item-misc' misses a field; it needs Now, Next, Blocked and Ledger"
                    .to_owned()
            )]
        );
    }

    #[test]
    fn an_item_field_is_capped() {
        // The marker and the label are not tokens; continuation lines are.
        let now = format!("- Now: {}\n  {}\n", "w ".repeat(40), "w ".repeat(11));
        assert_eq!(
            findings(&edited("- Now: one two\n  three\n", &now)),
            [(
                Rule::ItemFieldLong,
                "item 'alpha': Now exceeds 50 tokens (51)".to_owned()
            )]
        );
        assert_eq!(
            findings(&edited(
                "- Ledger: ledger-a.md\n",
                "- Ledger: ledger-a.md\n  and more\n"
            )),
            [(
                Rule::ItemFieldLong,
                "item 'alpha': Ledger exceeds 1 line".to_owned()
            )]
        );
    }

    #[test]
    fn an_item_heading_is_dated() {
        for heading in ["### alpha", "### alpha 2026-9-02", "### alpha (26-09-02)"] {
            assert_eq!(
                rules(&edited("### alpha (2026-09-02)", heading)),
                [Rule::ItemUndated],
                "{heading}"
            );
        }
        // The script strips the brackets, not checks them.
        assert_eq!(
            rules(&edited("### alpha (2026-09-02)", "### alpha 2026-09-02)")),
            []
        );
    }

    #[test]
    fn an_item_past_the_stale_age_warns() {
        assert_eq!(
            findings(&edited("(2026-09-02)", "(2026-09-01)")),
            [(
                Rule::ItemStale,
                "item 'alpha' was last rewritten 31 days ago; re-read it against reality, then \
                 rewrite or confirm it"
                    .to_owned()
            )]
        );
        assert_eq!(Rule::ItemStale.severity(), Severity::Warning);
    }

    #[test]
    fn the_item_prefix_goes_with_a_dash_ledger() {
        assert_eq!(
            rules(&edited("- Ledger: —\n", "- Ledger: none\n")),
            [Rule::ItemPrefix]
        );
        // An unprefixed slug with a dash ledger names no engagement.
        assert_eq!(
            rules(&edited("- Ledger: ledger-a.md", "- Ledger:  — ")),
            [Rule::ItemPrefix, Rule::EngagementUnlisted]
        );
    }

    #[test]
    fn an_item_names_an_active_open_engagement() {
        assert_eq!(rules(&with_item("alpha/thread")), []);
        for slug in ["beta", "omega/x", "gamma"] {
            assert_eq!(rules(&with_item(slug)), [Rule::ItemEngagement], "{slug}");
        }
    }

    #[test]
    fn the_items_are_capped() {
        let state = with_item("alpha/one").replacen(
            "## 3. Dormant",
            "### alpha/two (2026-10-01)\n\n- Now: a\n- Next: b\n- Blocked: c\n- Ledger: d\n\n\
             ## 3. Dormant",
            1,
        );
        assert_eq!(
            findings(&state),
            [(
                Rule::ItemsOverCap,
                "4 items exceed the max_items cap (3)".to_owned()
            )]
        );
    }

    #[test]
    fn every_open_engagement_is_listed() {
        let registry = format!(
            "{REGISTRY_TEXT}\n[engagements.delta]\nstatus = \"open\"\nremits = []\n\n\
             [engagements.epsilon]\nstatus = \"open\"\nactivity = \"dormant\"\nremits = []\n"
        );
        assert_eq!(
            findings_in(&registry, CLEAN, None),
            [
                (
                    Rule::EngagementUnlisted,
                    "open engagement 'delta' has no item under '## Open items'".to_owned()
                ),
                (
                    Rule::EngagementUnlisted,
                    "dormant open engagement 'epsilon' has no roll-up line under \
                     '## Dormant engagements'"
                        .to_owned()
                ),
            ]
        );
    }
}
