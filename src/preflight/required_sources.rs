//! Check workflow-declared alternatives against named fragments of the pending request.

use crate::{Availability, Call};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const ATTRIBUTE: &str = "nexusweave.required_sources";

/// A capture requirement is met when every source in at least one alternative is anchored.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredSources {
    name: String,
    any_of: Vec<Vec<String>>,
}

/// IDs that still need an anchor in each alternative, in the order supplied by the caller.
#[derive(Debug, Serialize)]
pub struct SourceCoverage {
    pub name: String,
    pub matched: bool,
    pub alternatives: Vec<Alternative>,
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
        Ok(())
    }

    /// Evaluate on a normalized pending call whose request fragments have been validated.
    pub fn evaluate(&self, call: &Call) -> SourceCoverage {
        let anchored: BTreeSet<_> = call
            .context
            .iter()
            .filter(|item| {
                item.availability == Availability::Included && item.request_fragment.is_some()
            })
            .filter_map(|item| item.source_id.as_ref())
            .collect();
        let alternatives: Vec<_> = self
            .any_of
            .iter()
            .map(|required| {
                let unconfirmed = required
                    .iter()
                    .filter(|id| !anchored.contains(*id))
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
        }
    }
}
