use super::Choice;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{borrow::Cow, collections::BTreeSet};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MatchRule {
    #[default]
    Exact,
    KindAndLabel,
    OutcomeFields {
        pointers: Vec<String>,
    },
    TextCapture {
        pattern: String,
    },
}

impl MatchRule {
    pub(crate) fn is_exact(&self) -> bool {
        matches!(self, Self::Exact)
    }

    pub(crate) fn normalized(&self) -> Self {
        let mut rule = self.clone();
        if let Self::OutcomeFields { pointers } = &mut rule {
            pointers.sort_unstable();
        }
        rule
    }
}

pub(super) struct Matcher<'a> {
    recorded: &'a Choice,
    comparison: Comparison<'a>,
}

enum Comparison<'a> {
    Exact,
    KindAndLabel,
    OutcomeFields {
        pointers: &'a [String],
        expected: Vec<Value>,
    },
    TextCapture {
        regex: Regex,
        expected: &'a str,
    },
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) enum Projection<'a> {
    Label,
    Exact(&'a Value),
    Fields(Vec<Value>),
    Capture(&'a str),
}

impl<'a> Matcher<'a> {
    pub(super) fn new(rule: &'a MatchRule, recorded: &'a Choice) -> Result<Self, String> {
        let comparison = match rule {
            MatchRule::Exact => Comparison::Exact,
            MatchRule::KindAndLabel => Comparison::KindAndLabel,
            MatchRule::OutcomeFields { pointers } => {
                let mut seen = BTreeSet::new();
                if pointers.is_empty()
                    || !pointers
                        .iter()
                        .all(|pointer| valid_pointer(pointer) && seen.insert(pointer))
                {
                    return Err(
                        "outcome_fields requires distinct JSON Pointers to recorded fields".into(),
                    );
                }
                let outcome = structured_outcome(&recorded.outcome)
                    .ok_or("outcome_fields requires a recorded JSON object or array")?;
                let expected = pointers
                    .iter()
                    .map(|pointer| {
                        outcome
                            .pointer(pointer)
                            .cloned()
                            .ok_or_else(|| format!("recorded outcome has no field {pointer}"))
                    })
                    .collect::<Result<_, _>>()?;
                Comparison::OutcomeFields { pointers, expected }
            }
            MatchRule::TextCapture { pattern } => {
                let regex = Regex::new(pattern)
                    .map_err(|error| format!("invalid text_capture pattern: {error}"))?;
                if regex.captures_len() != 2 {
                    return Err("text_capture requires exactly one capture group".into());
                }
                let expected = recorded.outcome.as_str().and_then(|text| unique_capture(&regex, text))
                    .ok_or("text_capture must extract exactly one nonempty value from the recorded outcome")?;
                Comparison::TextCapture { regex, expected }
            }
        };
        Ok(Self {
            recorded,
            comparison,
        })
    }

    pub(super) fn compare(&self, choice: &Choice) -> Option<bool> {
        if choice.kind != self.recorded.kind || choice.label != self.recorded.label {
            return Some(false);
        }
        match &self.comparison {
            Comparison::Exact => Some(choice.outcome == self.recorded.outcome),
            Comparison::KindAndLabel => Some(true),
            Comparison::OutcomeFields { pointers, expected } => {
                let outcome = structured_outcome(&choice.outcome)?;
                let mut matches = true;
                for (pointer, expected) in pointers.iter().zip(expected) {
                    matches &= outcome.pointer(pointer)? == expected;
                }
                Some(matches)
            }
            Comparison::TextCapture { regex, expected } => {
                Some(unique_capture(regex, choice.outcome.as_str()?)? == *expected)
            }
        }
    }

    pub(super) fn project<'choice>(&self, choice: &'choice Choice) -> Option<Projection<'choice>> {
        let same_label = choice.kind == self.recorded.kind && choice.label == self.recorded.label;
        match &self.comparison {
            Comparison::Exact => Some(Projection::Exact(&choice.outcome)),
            Comparison::OutcomeFields { pointers, .. } if same_label => {
                let outcome = structured_outcome(&choice.outcome)?;
                let fields = pointers
                    .iter()
                    .map(|pointer| outcome.pointer(pointer).cloned());
                Some(Projection::Fields(fields.collect::<Option<_>>()?))
            }
            Comparison::TextCapture { regex, .. } if same_label => Some(Projection::Capture(
                unique_capture(regex, choice.outcome.as_str()?)?,
            )),
            _ => Some(Projection::Label),
        }
    }
}

pub(crate) fn valid_pointer(pointer: &str) -> bool {
    pointer.starts_with('/')
        && pointer
            .split('~')
            .skip(1)
            .all(|suffix| suffix.starts_with(['0', '1']))
}

fn structured_outcome(value: &Value) -> Option<Cow<'_, Value>> {
    if value.is_object() || value.is_array() {
        return Some(Cow::Borrowed(value));
    }
    let decoded = serde_json::from_str::<Value>(value.as_str()?).ok()?;
    (decoded.is_object() || decoded.is_array()).then_some(Cow::Owned(decoded))
}

fn unique_capture<'a>(regex: &Regex, text: &'a str) -> Option<&'a str> {
    let mut matches = regex.captures_iter(text);
    let captured = matches.next()?.get(1)?.as_str();
    (!captured.is_empty() && matches.next().is_none()).then_some(captured)
}
