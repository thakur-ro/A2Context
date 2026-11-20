//! Screen captured text for reuse and possible semantic overlap or conflict.

pub mod patterns;

use crate::{preflight, replay::sha256, Availability, Call, Evidence, RequestFragment};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const VERSION: u32 = 1;
const MAX_PAIRS: usize = 256;
const SUGGESTION_SCORE: f64 = 0.8;

/// Select captured inputs and bound the number of compared pairs.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_ids: Vec<String>,
    #[serde(default = "default_limit")]
    pub pair_limit: usize,
}

fn default_limit() -> usize {
    64
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            version: VERSION,
            context_ids: Vec::new(),
            pair_limit: default_limit(),
        }
    }
}

/// One pair of exact captured texts submitted to a scorer.
#[derive(Serialize)]
pub struct TextPair<'a> {
    pub id: usize,
    pub left: &'a str,
    pub right: &'a str,
}

/// Batch of non-identical pairs. Score each pair in both directions.
#[derive(Serialize)]
pub struct ScoringRequest<'a> {
    pub version: u32,
    pub pairs: Vec<TextPair<'a>>,
}

/// The scorer's model and runtime identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scorer {
    pub model_id: String,
    pub revision: String,
    pub runtime: String,
}

impl Scorer {
    fn valid(&self) -> bool {
        [&self.model_id, &self.revision, &self.runtime]
            .iter()
            .all(|value| !value.trim().is_empty())
    }
}

/// A normalized three-way classification score, usually from a softmax.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scores {
    pub contradiction: f64,
    pub entailment: f64,
    pub neutral: f64,
}

impl Scores {
    fn valid(self) -> bool {
        let scores = [self.contradiction, self.entailment, self.neutral];
        scores.iter().all(|value| (0.0..=1.0).contains(value))
            && (scores.iter().sum::<f64>() - 1.0).abs() <= 1e-4
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unscored {
    NoScorer,
    TooLong,
    Unavailable,
}

/// The evidence available for a text pair.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Assessment {
    Exact,
    Scored {
        left_to_right: Scores,
        right_to_left: Scores,
    },
    Unscored {
        reason: Unscored,
    },
}

impl Assessment {
    fn valid(self, exact: bool, has_scorer: bool) -> bool {
        match self {
            Self::Exact => exact,
            Self::Scored {
                left_to_right,
                right_to_left,
            } => !exact && has_scorer && left_to_right.valid() && right_to_left.valid(),
            Self::Unscored {
                reason: Unscored::NoScorer,
            } => !exact && !has_scorer,
            Self::Unscored { .. } => !exact && has_scorer,
        }
    }

    fn suggestion(self) -> Option<Suggestion> {
        match self {
            Self::Exact => Some(Suggestion::ExactTextReuse),
            Self::Scored {
                left_to_right,
                right_to_left,
            } if left_to_right.contradiction.min(right_to_left.contradiction)
                >= SUGGESTION_SCORE =>
            {
                Some(Suggestion::PossibleConflict)
            }
            Self::Scored {
                left_to_right,
                right_to_left,
            } if left_to_right.entailment.min(right_to_left.entailment) >= SUGGESTION_SCORE => {
                Some(Suggestion::PossibleOverlap)
            }
            _ => None,
        }
    }
}

/// A scorer response reference. IDs must match the submitted rows in order.
#[derive(Debug, Deserialize)]
pub struct ScoredPair {
    pub id: usize,
    #[serde(flatten)]
    pub assessment: Assessment,
}

/// Results returned by an external scorer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoringResult {
    pub version: u32,
    pub scorer: Scorer,
    pub pairs: Vec<ScoredPair>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Suggestion {
    ExactTextReuse,
    PossibleConflict,
    PossibleOverlap,
}

/// A reference to a captured item. The digest covers the exact UTF-8 text.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub context_id: String,
    pub source_id: Option<String>,
    pub source_version: Option<String>,
    pub kind: String,
    pub role: Option<String>,
    pub request_fragment: Option<RequestFragment>,
    pub text_sha256: String,
}

/// One comparison between two indices in the report's source list.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Pair {
    pub sources: [usize; 2],
    pub suggestion: Option<Suggestion>,
    #[serde(flatten)]
    pub assessment: Assessment,
}

/// An assessment of a bounded selection of captured context text.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub version: u32,
    pub selection: Selection,
    pub scorer: Option<Scorer>,
    pub sources: Vec<Source>,
    pub total_pairs: usize,
    pub pairs: Vec<Pair>,
}

/// Run screening while assembling a request.
///
/// The callback receives exact text from nonidentical pairs. Return `Ok(None)`
/// to use exact matching only. Exact copies bypass the scorer.
///
/// ```
/// use nexusweave::context_screening::{screen_pending, Selection, Suggestion};
///
/// let request = serde_json::from_str(include_str!(concat!(
///     env!("CARGO_MANIFEST_DIR"),
///     "/examples/context-screening/request.json"
/// )))?;
/// let report = screen_pending(request, &Selection::default(), |_| Ok(None))?;
/// assert_eq!(report.total_pairs, 21);
/// let repeated = report.pairs.iter()
///     .find(|pair| pair.suggestion == Some(Suggestion::ExactTextReuse))
///     .expect("the request repeats one excerpt");
/// let [first, second] = repeated.sources.map(|index| &report.sources[index]);
/// assert_eq!(first.text_sha256, second.text_sha256);
/// assert_ne!(first.context_id, second.context_id);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn screen_pending(
    request: preflight::Request,
    selection: &Selection,
    score: impl FnOnce(&ScoringRequest<'_>) -> Result<Option<ScoringResult>, String>,
) -> Result<Report, String> {
    let (call, _, _) = preflight::pending_call(request)?;
    screen_call(&call, selection, score)
}

/// Screen the captured inputs of a normalized inference call.
///
/// This operation compares text. It leaves precedence and dispatch to the caller.
pub fn screen_call(
    call: &Call,
    selection: &Selection,
    score: impl FnOnce(&ScoringRequest<'_>) -> Result<Option<ScoringResult>, String>,
) -> Result<Report, String> {
    let plan = plan(call, selection)?;
    let batch = ScoringRequest {
        version: VERSION,
        pairs: plan
            .pairs
            .iter()
            .enumerate()
            .filter_map(|(id, &[left, right])| {
                let left = plan.text[left];
                let right = plan.text[right];
                (left != right).then_some(TextPair { id, left, right })
            })
            .collect(),
    };
    let scored = if batch.pairs.is_empty() {
        None
    } else {
        score(&batch)?
    };
    let (scorer, mut results) = if let Some(result) = scored {
        if result.version != VERSION
            || !result.scorer.valid()
            || result.pairs.len() != batch.pairs.len()
            || !result
                .pairs
                .iter()
                .zip(&batch.pairs)
                .all(|(result, expected)| {
                    result.id == expected.id && result.assessment.valid(false, true)
                })
        {
            return Err(
                "The scorer response does not match the requested pairs or score format.".into(),
            );
        }
        (Some(result.scorer), result.pairs.into_iter())
    } else {
        (None, Vec::new().into_iter())
    };
    let pairs = plan
        .pairs
        .into_iter()
        .map(|sources @ [left, right]| {
            let assessment = if plan.text[left] == plan.text[right] {
                Assessment::Exact
            } else if let Some(result) = results.next() {
                result.assessment
            } else {
                Assessment::Unscored {
                    reason: Unscored::NoScorer,
                }
            };
            Pair {
                sources,
                suggestion: assessment.suggestion(),
                assessment,
            }
        })
        .collect();
    Ok(Report {
        version: VERSION,
        selection: selection.clone(),
        scorer,
        sources: plan.sources,
        total_pairs: plan.total_pairs,
        pairs,
    })
}

struct Plan<'a> {
    sources: Vec<Source>,
    text: Vec<&'a str>,
    total_pairs: usize,
    pairs: Vec<[usize; 2]>,
}

fn plan<'a>(call: &'a Call, selection: &Selection) -> Result<Plan<'a>, String> {
    if selection.version != VERSION || !(1..=MAX_PAIRS).contains(&selection.pair_limit) {
        return Err(format!(
            "Use context screening version 1 and a pair limit from 1 to {MAX_PAIRS}."
        ));
    }
    let ids: BTreeSet<_> = selection.context_ids.iter().map(String::as_str).collect();
    if ids.len() != selection.context_ids.len() {
        return Err("Select each context ID at most once.".into());
    }
    let mut sources = Vec::new();
    let mut text = Vec::new();
    for item in &call.context {
        if item.availability != Availability::Included
            || (!ids.is_empty() && !ids.contains(item.id.as_str()))
        {
            continue;
        }
        let Some(body) = item.text.as_deref().filter(|text| !text.trim().is_empty()) else {
            continue;
        };
        sources.push(Source {
            context_id: item.id.clone(),
            source_id: item.source_id.clone(),
            source_version: item.source_version.clone(),
            kind: item.kind.clone(),
            role: item.role.clone(),
            request_fragment: item.request_fragment.clone(),
            text_sha256: sha256(body.as_bytes()),
        });
        text.push(body);
    }
    if !ids.is_empty() && ids.len() != text.len() {
        return Err("One or more selected context IDs have no captured text.".into());
    }
    let count = text.len();
    let total_pairs = count.saturating_mul(count.saturating_sub(1)) / 2;
    let pairs = (0..count)
        .flat_map(|left| ((left + 1)..count).map(move |right| [left, right]))
        .take(selection.pair_limit)
        .collect();
    Ok(Plan {
        sources,
        text,
        total_pairs,
        pairs,
    })
}

/// Bind a report to the exact captured selection and check its assessment format.
/// The scorer supplies the semantic scores.
pub fn validate_report(report: &Report, call: &Call) -> Result<(), String> {
    let plan = plan(call, &report.selection)?;
    if report.version != VERSION
        || report.sources != plan.sources
        || report.total_pairs != plan.total_pairs
        || report.pairs.len() != plan.pairs.len()
        || report.scorer.as_ref().is_some_and(|scorer| !scorer.valid())
        || (report.scorer.is_some()
            && report
                .pairs
                .iter()
                .all(|pair| pair.assessment == Assessment::Exact))
    {
        return Err("The screen does not match the recorded context selection.".into());
    }
    for (pair, &[left, right]) in report.pairs.iter().zip(&plan.pairs) {
        if pair.sources != [left, right]
            || !pair
                .assessment
                .valid(plan.text[left] == plan.text[right], report.scorer.is_some())
            || pair.suggestion != pair.assessment.suggestion()
        {
            return Err("A screened pair differs from the captured sources or scores.".into());
        }
    }
    Ok(())
}

/// A report attached to the same inference call as its captured inputs.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CapturedReport {
    pub evidence: Evidence,
    pub report: Option<Report>,
}

#[derive(Debug, Serialize)]
pub struct Recorded<'a> {
    pub run_id: &'a str,
    pub call_id: &'a str,
    pub evidence: &'a Evidence,
    #[serde(flatten)]
    pub verification: Verification<'a>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verification<'a> {
    Matched { report: &'a Report },
    Unverified { issue: String },
}

impl CapturedReport {
    /// Return the supplied report if it matches the context captured on this call.
    pub fn verified(&self, call: &Call) -> Result<&Report, String> {
        let report = self
            .report
            .as_ref()
            .ok_or_else(|| "The workflow supplied an unreadable context screen.".to_owned())?;
        validate_report(report, call)?;
        Ok(report)
    }
}

pub(crate) fn recorded<'a>(run_id: &'a str, call: &'a Call) -> Option<Recorded<'a>> {
    let captured = call.context_screening.as_ref()?;
    let result = captured.verified(call);
    Some(Recorded {
        run_id,
        call_id: &call.id,
        evidence: &captured.evidence,
        verification: match result {
            Ok(report) => Verification::Matched { report },
            Err(issue) => Verification::Unverified { issue },
        },
    })
}
