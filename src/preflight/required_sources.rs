//! Check workflow-declared alternatives against named fragments of the pending request.

use crate::{Availability, Call};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const ATTRIBUTE: &str = "nexusweave.required_sources";

/// One alternative must contain every required anchor and match its optional expected text.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredSources {
    name: String,
    any_of: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    expected_text_sha256: BTreeMap<String, String>,
}

/// How each configured alternative compares with the captured request.
#[derive(Debug, Serialize)]
pub struct SourceCoverage {
    pub name: String,
    pub matched: bool,
    pub alternatives: Vec<Alternative>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub expected_text_sha256: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub text_mismatches: BTreeSet<String>,
}

/// The original required IDs and the subset without a matching named request fragment.
#[derive(Debug, Serialize)]
pub struct Alternative {
    pub required: Vec<String>,
    pub unconfirmed: Vec<String>,
}

impl RequiredSources {
    /// Remove the optional check from the surrounding attributes before input normalization.
    pub fn take(attributes: &mut Map<String, Value>) -> Result<Option<Self>, String> {
        let Some(raw) = attributes.remove(ATTRIBUTE) else {
            return Ok(None);
        };
        let parsed = match raw {
            Value::String(text) => serde_json::from_str(&text),
            value => serde_json::from_value(value),
        };
        let check: Self = parsed.map_err(|e| format!("Invalid {ATTRIBUTE}: {e}"))?;
        check.validate()?;
        Ok(Some(check))
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.any_of.is_empty() {
            return Err(format!(
                "{ATTRIBUTE} requires a name and at least one alternative"
            ));
        }
        for group in &self.any_of {
            if group.is_empty() || group.iter().any(|id| id.trim().is_empty()) {
                return Err(format!(
                    "Every {ATTRIBUTE} alternative needs nonempty source IDs"
                ));
            }
        }
        if self.expected_text_sha256.is_empty() {
            return Ok(());
        }
        let required: BTreeSet<_> = self.any_of.iter().flatten().collect();
        for (id, digest) in &self.expected_text_sha256 {
            if !required.contains(id) {
                return Err(format!("Expected text supplied for undeclared source {id}"));
            }
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(format!("Expected lowercase SHA-256 digest for source {id}"));
            }
        }
        Ok(())
    }

    pub(super) fn receipt_version(&self) -> u32 {
        if self.expected_text_sha256.is_empty() {
            2
        } else {
            3
        }
    }

    /// Evaluate on a normalized pending call whose request fragments have been validated.
    pub fn evaluate(&self, call: &Call) -> SourceCoverage {
        let mut text_mismatches = BTreeSet::new();
        let mut confirmed = BTreeSet::new();
        for item in &call.context {
            if item.availability != Availability::Included || item.request_fragment.is_none() {
                continue;
            }
            let Some(id) = &item.source_id else { continue };
            if confirmed.contains(id) {
                continue;
            }
            let matches = self.expected_text_sha256.get(id).is_none_or(|expected| {
                item.text
                    .as_ref()
                    .is_some_and(|text| crate::replay::sha256(text.as_bytes()) == *expected)
            });
            if matches {
                confirmed.insert(id);
                text_mismatches.remove(id);
            } else {
                text_mismatches.insert(id.clone());
            }
        }
        let alternatives: Vec<_> = self
            .any_of
            .iter()
            .map(|required| {
                let unconfirmed = required
                    .iter()
                    .filter(|id| !confirmed.contains(*id))
                    .cloned()
                    .collect();
                Alternative {
                    required: required.clone(),
                    unconfirmed,
                }
            })
            .collect();
        SourceCoverage {
            name: self.name.clone(),
            matched: alternatives.iter().any(|row| row.unconfirmed.is_empty()),
            alternatives,
            expected_text_sha256: self.expected_text_sha256.clone(),
            text_mismatches,
        }
    }
}
