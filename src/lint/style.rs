//! The house style: every engagement's remits, and the shape of `STATE.md`.
//!
//! These rules are the workspace's own conventions, beyond the contract and the
//! template's layout. They run only on a model read from the template's layout,
//! and their findings count only when the registry carries a `[state]` table.

use super::layout::{ARCHIVE, ENGAGEMENTS, REGISTRY};
use super::model::{Caps, Layout, Model};
use super::registry::RegistryFile;
use super::rule::Rule;
use super::rules::{Finding, OPEN};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

/// The handover file the shape rules read.
const STATE: &str = "context/STATE.md";
const NEXT: &str = "Next";
const OPEN_ITEMS: &str = "Open items";
const DORMANT_ENGAGEMENTS: &str = "Dormant engagements";
/// The slug prefix marking an item whose subject has no engagement.
const NO_TOPIC: &str = "item-";
/// The day number of 1970-01-01, the epoch `today` counts from.
const EPOCH_DAY: i64 = 2_440_588;

/// Every house-style finding in the model, or none when it was not read from
/// the template's layout. `today` is a day number, as [`today`] gives it.
pub fn check(model: &Model, today: i64) -> Vec<Finding> {
    let Some(layout) = &model.layout else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    let mut found = |path: &str, rule: Rule, message: String| {
        findings.push(Finding {
            path: path.to_owned(),
            message,
            rule,
        });
    };
    check_remits(layout, &mut found);
    if let Some(state) = model
        .artefacts
        .iter()
        .find(|artefact| artefact.path == STATE)
    {
        let caps = layout.state.unwrap_or_default();
        for (rule, message) in Shape::check(&state.body, model, layout, caps, today) {
            found(STATE, rule, message);
        }
    }
    findings
}

/// Today's day number in UTC.
pub fn today() -> i64 {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / 86_400);
    EPOCH_DAY + i64::try_from(days).unwrap_or(0)
}

/// Each engagement lists its remits, each kebab-case and once; and, in a
/// commit, a remit the register has not held before is pointed out.
fn check_remits(layout: &Layout, found: &mut impl FnMut(&str, Rule, String)) {
    for (name, remits) in &layout.remits {
        let Some(remits) = remits else {
            found(
                REGISTRY,
                Rule::RemitsMissing,
                format!("engagement '{name}' has no remits key (required; [] means none)"),
            );
            continue;
        };
        let mut seen = BTreeMap::new();
        for remit in remits {
            if !kebab_case(remit) {
                found(
                    REGISTRY,
                    Rule::RemitMalformed,
                    format!("engagement '{name}' remit '{remit}' is not kebab-case"),
                );
            }
            let count = seen.entry(remit).or_insert(0);
            *count += 1;
            // Once per remit, however often it repeats, as the script does.
            if *count == 2 {
                found(
                    REGISTRY,
                    Rule::RemitRepeated,
                    format!("engagement '{name}' lists remit '{remit}' twice"),
                );
            }
        }
    }
    let Some(prior) = &layout.prior_remits else {
        return;
    };
    let existing = if prior.is_empty() {
        "none".to_owned()
    } else {
        prior
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let current: BTreeSet<&String> = layout.remits.values().flatten().flatten().collect();
    for remit in current.into_iter().filter(|remit| !prior.contains(*remit)) {
        found(
            REGISTRY,
            Rule::RemitNew,
            format!(
                "remit '{remit}' appears for the first time in the register (existing: {existing})"
            ),
        );
    }
}

fn kebab_case(value: &str) -> bool {
    value.split('-').all(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    })
}

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
struct Shape<'a> {
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
    fn check(
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

/// The day number of a `YYYY-MM-DD` date, by the script's arithmetic; `None`
/// for any other shape.
fn day_number(date: &str) -> Option<i64> {
    let bytes = date.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    if !shaped {
        return None;
    }
    let part = |range: std::ops::Range<usize>| date[range].parse::<i64>().ok();
    let (year, month, day) = (part(0..4)?, part(5..7)?, part(8..10)?);
    let a = (14 - month) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    Some(day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32_045)
}

/// `aw lint --remits`: one tab-separated line per remit an engagement lists,
/// giving the remit, the engagement, its status and activity, and its directory
/// if it sits where its status puts it, else `-`. An engagement listing no
/// remit appears under `-`; one with no `remits` key not at all. Sorted
/// bytewise; with `filter`, only that remit's lines.
pub fn remits(
    registry: &RegistryFile,
    is_dir: impl Fn(&str) -> bool,
    filter: Option<&str>,
) -> Vec<String> {
    let none = ["-".to_owned()];
    let mut lines = Vec::new();
    for (name, entry) in &registry.engagements {
        let Some(remits) = &entry.remits else {
            continue;
        };
        let status = entry.status.as_str();
        let activity = entry.activity.as_deref().unwrap_or("active");
        let tree = if status == OPEN { ENGAGEMENTS } else { ARCHIVE };
        let directory = format!("{tree}/{name}");
        let directory = if is_dir(&directory) {
            directory.as_str()
        } else {
            "-"
        };
        let remits = if remits.is_empty() { &none[..] } else { remits };
        for remit in remits {
            lines.push(format!(
                "{remit}\t{name}\t{status}\t{activity}\t{directory}"
            ));
        }
    }
    lines.sort();
    if let Some(filter) = filter.filter(|filter| !filter.is_empty()) {
        lines.retain(|line| line.split('\t').next() == Some(filter));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::layout;
    use crate::lint::rule::Severity;

    const REGISTRY_TEXT: &str = "\
[class.standing]
statuses = [\"live\", \"retired\"]

[kind.state]
class = \"standing\"

[state]
max_items = 3
next_entries = 2

[engagements.alpha]
status = \"open\"
remits = [\"build\"]

[engagements.beta]
status = \"closed\"
remits = []

[engagements.gamma]
status = \"open\"
activity = \"dormant\"
remits = [\"operations\"]
";

    /// A `STATE.md` clean under every house-style rule on `today`.
    const CLEAN: &str = "\
# State

## 1. Next

1. alpha
2. item-misc — with a note
   carried on

## 2. Open items

### alpha (2026-09-02)

- Now: one two
  three
- Next: four
- Blocked: —
- Ledger: ledger-a.md

### item-misc (2026-10-01)

- Now: x
- Next: y
- Blocked: z
- Ledger: —

## 3. Dormant engagements

- gamma: resting until
  the spring.
";

    fn today() -> i64 {
        day_number("2026-10-02").unwrap()
    }

    /// The house-style findings for `state` as `STATE.md` under `registry`,
    /// with `prior` as `HEAD`'s remits.
    fn findings_in(registry: &str, state: &str, prior: Option<&[&str]>) -> Vec<(Rule, String)> {
        let registry = RegistryFile::parse(registry, "registry").unwrap();
        let mut model = layout::read(
            &registry,
            vec![(
                STATE.to_owned(),
                format!("---\nkind: state\nstatus: live\n---\n{state}"),
            )],
        );
        let mut layout = layout::declared(&registry);
        layout.prior_remits =
            prior.map(|remits| remits.iter().map(|remit| (*remit).to_owned()).collect());
        model.layout = Some(layout);
        check(&model, today())
            .into_iter()
            .map(|finding| (finding.rule, finding.message))
            .collect()
    }

    fn findings(state: &str) -> Vec<(Rule, String)> {
        findings_in(REGISTRY_TEXT, state, None)
    }

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
    fn the_house_style_is_clean() {
        assert_eq!(findings(CLEAN), []);
    }

    #[test]
    fn a_model_read_from_no_layout_gets_no_house_style_rules() {
        let registry = RegistryFile::parse(REGISTRY_TEXT, "registry").unwrap();
        let model = layout::read(&registry, vec![(STATE.to_owned(), "```\n".to_owned())]);
        assert!(check(&model, today()).is_empty());
    }

    #[test]
    fn without_a_state_file_only_the_remits_are_checked() {
        let registry =
            RegistryFile::parse(&REGISTRY_TEXT.replace("remits = []\n", ""), "registry").unwrap();
        let mut model = layout::read(&registry, Vec::new());
        model.layout = Some(layout::declared(&registry));
        let rules: Vec<Rule> = check(&model, today()).iter().map(|f| f.rule).collect();
        assert_eq!(rules, [Rule::RemitsMissing]);
    }

    #[test]
    fn every_engagement_lists_its_remits() {
        let registry = REGISTRY_TEXT.replace("remits = []\n", "");
        assert_eq!(
            findings_in(&registry, CLEAN, None),
            [(
                Rule::RemitsMissing,
                "engagement 'beta' has no remits key (required; [] means none)".to_owned()
            )]
        );
    }

    #[test]
    fn a_remit_is_kebab_case() {
        let registry = REGISTRY_TEXT.replace(
            "remits = [\"build\"]",
            "remits = [\"build-2\", \"Build\", \"a--b\", \"-a\", \"a_b\"]",
        );
        let found = findings_in(&registry, CLEAN, None);
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(
            found
                .iter()
                .all(|(rule, message)| *rule == Rule::RemitMalformed
                    && !message.contains("'build-2'"))
        );
    }

    #[test]
    fn a_repeated_remit_is_reported_once() {
        let registry = REGISTRY_TEXT.replace(
            "remits = [\"build\"]",
            "remits = [\"build\", \"build\", \"build\"]",
        );
        assert_eq!(
            findings_in(&registry, CLEAN, None),
            [(
                Rule::RemitRepeated,
                "engagement 'alpha' lists remit 'build' twice".to_owned()
            )]
        );
    }

    #[test]
    fn a_commit_adding_a_remit_is_told_so() {
        assert_eq!(
            findings_in(REGISTRY_TEXT, CLEAN, Some(&["build", "publication"])),
            [(
                Rule::RemitNew,
                "remit 'operations' appears for the first time in the register (existing: \
                 build, publication)"
                    .to_owned()
            )]
        );
        let found = findings_in(REGISTRY_TEXT, CLEAN, Some(&[]));
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(
            |(rule, message)| *rule == Rule::RemitNew && message.ends_with("(existing: none)")
        ));
        assert_eq!(Rule::RemitNew.severity(), Severity::Warning);
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

    #[test]
    fn a_missing_state_key_takes_the_scripts_default() {
        let registry = RegistryFile::parse("[state]\nmax_items = 15\n", "registry").unwrap();
        assert_eq!(
            registry.state,
            Some(Caps {
                max_items: 15,
                ..Caps::default()
            })
        );
        assert_eq!(
            Caps::default(),
            Caps {
                max_items: 12,
                field_tokens: 50,
                rollup_tokens: 25,
                next_entries: 8,
                stale_days: 30,
            }
        );
    }

    #[test]
    fn day_numbers_count_from_the_epoch() {
        assert_eq!(day_number("1970-01-01"), Some(EPOCH_DAY));
        assert_eq!(
            day_number("2024-03-01").unwrap() - day_number("2024-02-28").unwrap(),
            2
        );
        assert!(super::today() > day_number("2026-01-01").unwrap());
    }

    #[test]
    fn remits_lists_each_engagement_under_each_remit() {
        let registry = RegistryFile::parse(
            &format!("{REGISTRY_TEXT}\n[engagements.delta]\nstatus = \"open\"\n"),
            "registry",
        )
        .unwrap();
        let on_disk = [
            "context/engagements/alpha",
            "context/archive/beta",
            "context/archive/gamma",
        ];
        let is_dir = |dir: &str| on_disk.contains(&dir);
        assert_eq!(
            remits(&registry, is_dir, None),
            [
                "-\tbeta\tclosed\tactive\tcontext/archive/beta",
                "build\talpha\topen\tactive\tcontext/engagements/alpha",
                "operations\tgamma\topen\tdormant\t-",
            ]
        );
        assert_eq!(
            remits(&registry, is_dir, Some("build")),
            ["build\talpha\topen\tactive\tcontext/engagements/alpha"]
        );
        assert_eq!(remits(&registry, is_dir, Some("")).len(), 3);
    }
}
