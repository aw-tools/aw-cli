//! The house style: every engagement's remits, and the shape of `STATE.md`.
//!
//! These rules are the workspace's own conventions, beyond the contract and the
//! template's layout. They run only on a model read from the template's layout,
//! and their findings count only when the registry carries a `[state]` table.

mod remits;
mod state;

pub use remits::remits;

use super::model::Model;
use super::rule::Rule;
use super::rules::Finding;
use std::time::{SystemTime, UNIX_EPOCH};

/// The handover file the shape rules read.
const STATE: &str = "context/STATE.md";
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
    remits::check(layout, &mut found);
    if let Some(state) = model
        .artefacts
        .iter()
        .find(|artefact| artefact.path == STATE)
    {
        let caps = layout.state.unwrap_or_default();
        for (rule, message) in state::Shape::check(&state.body, model, layout, caps, today) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::layout;
    use crate::lint::model::Caps;
    use crate::lint::registry::RegistryFile;

    pub(super) const REGISTRY_TEXT: &str = "\
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
    pub(super) const CLEAN: &str = "\
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
    pub(super) fn findings_in(
        registry: &str,
        state: &str,
        prior: Option<&[&str]>,
    ) -> Vec<(Rule, String)> {
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

    pub(super) fn findings(state: &str) -> Vec<(Rule, String)> {
        findings_in(REGISTRY_TEXT, state, None)
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
}
