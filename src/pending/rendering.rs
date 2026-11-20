//! Test whether removing a selected JSON field changes the renderer's ordered input tokens.
use crate::{preflight, replay};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const ATTRIBUTE: &str = "nexusweave.render_check";

/// Adapter output in request order. A null entry means the renderer rejected that input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rendered {
    pub renderer: String,
    pub renderer_sha256: String,
    pub tokens: Vec<Option<Vec<u32>>>,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Fingerprint {
    count: usize,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Removal {
    pointer: String,
    request_sha256: String,
    tokens: Option<Fingerprint>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    version: u8,
    renderer: String,
    renderer_sha256: String,
    request_sha256: String,
    baseline: Fingerprint,
    removals: Vec<Removal>,
}

fn fingerprint(ids: &[u32]) -> Fingerprint {
    Fingerprint {
        count: ids.len(),
        sha256: replay::sha256(&serde_json::to_vec(ids).expect("integer array serializes")),
    }
}

fn digest(value: &Value) -> String {
    replay::sha256(value.to_string().as_bytes())
}

fn removed(body: &Value, pointer: &str) -> Result<Value, String> {
    if !replay::valid_pointer(pointer) || body.pointer(pointer).is_none() {
        return Err(format!("No field at JSON pointer {pointer}"));
    }
    let (parent, key) = pointer.rsplit_once('/').expect("validated JSON pointer");
    let mut edited = body.clone();
    match edited.pointer_mut(parent) {
        Some(Value::Object(map)) => {
            // Keep every surviving field in captured order for the chat template.
            map.shift_remove(&key.replace("~1", "/").replace("~0", "~"));
        }
        Some(Value::Array(array)) => {
            let index: usize = key.parse().map_err(|_| "Invalid array index")?;
            array.remove(index);
        }
        _ => return Err(format!("Cannot remove field at {pointer}")),
    }
    Ok(edited)
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn pointers_valid<'a>(pointers: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    for pointer in pointers {
        if seen.len() == 32 || !seen.insert(pointer) {
            return false;
        }
    }
    !seen.is_empty()
}

/// Render the unchanged body and independent one-field omissions. Only the explicit
/// adapter sees the bodies. The saved measurement contains counts and fingerprints.
pub fn measure(
    mut input: preflight::Request,
    pointers: Vec<String>,
    render: impl FnOnce(&Value) -> Result<Rendered, String>,
) -> Result<preflight::Request, String> {
    let (_, body, _) = preflight::pending_call(input.clone())?;
    let body = body.ok_or("A rendering check requires a captured request body")?;
    if !pointers_valid(pointers.iter().map(String::as_str)) {
        return Err("Choose 1 to 32 distinct field pointers".into());
    }
    let mut bodies = vec![body];
    for pointer in &pointers {
        bodies.push(removed(&bodies[0], pointer)?);
    }
    let rendered = render(&json!({"version":1,"requests":bodies}))?;
    if rendered.tokens.len() != bodies.len() {
        return Err("Renderer returned a different number of token sequences".into());
    }
    let mut tokens = rendered.tokens.into_iter();
    let baseline = tokens
        .next()
        .flatten()
        .ok_or("Renderer rejected the original request")?;
    let report = Report {
        version: 1,
        renderer: rendered.renderer,
        renderer_sha256: rendered.renderer_sha256,
        request_sha256: digest(&bodies[0]),
        baseline: fingerprint(&baseline),
        removals: pointers
            .into_iter()
            .zip(&bodies[1..])
            .zip(tokens)
            .map(|((pointer, changed), ids)| Removal {
                pointer,
                request_sha256: digest(changed),
                tokens: ids.as_deref().map(fingerprint),
            })
            .collect(),
    };
    report.inspect(&bodies[0], false)?;
    input.attributes.insert(
        ATTRIBUTE.into(),
        serde_json::to_value(report).map_err(|e| e.to_string())?,
    );
    Ok(input)
}

impl Report {
    fn inspect(&self, body: &Value, details: bool) -> Result<Value, String> {
        if self.version != 1
            || self.renderer.trim().is_empty()
            || !valid_hash(&self.renderer_sha256)
            || !valid_hash(&self.baseline.sha256)
            || self.request_sha256 != digest(body)
            || !pointers_valid(self.removals.iter().map(|r| r.pointer.as_str()))
        {
            return Err("Invalid renderer receipt or captured request mismatch".into());
        }
        let mut rows = Vec::new();
        let (mut unchanged, mut unrenderable) = (0, 0);
        for removal in &self.removals {
            if digest(&removed(body, &removal.pointer)?) != removal.request_sha256 {
                return Err("Renderer receipt differs from the tested field removal".into());
            }
            let state = match &removal.tokens {
                None => {
                    unrenderable += 1;
                    "unrenderable"
                }
                Some(t)
                    if !valid_hash(&t.sha256)
                        || (t.sha256 == self.baseline.sha256 && t.count != self.baseline.count) =>
                {
                    return Err("Invalid token fingerprint".into())
                }
                Some(t) if t == &self.baseline => {
                    unchanged += 1;
                    "unchanged"
                }
                Some(_) => "changed",
            };
            if details {
                rows.push(json!({"pointer":removal.pointer,"status":state,"input_tokens":removal.tokens.as_ref().map(|t| t.count)}));
            }
        }
        let mut view =
            json!({"tested":self.removals.len(),"unchanged":unchanged,"unrenderable":unrenderable});
        if details {
            view["fields"] = json!(rows);
            view["renderer"] = json!(self.renderer);
            view["renderer_sha256"] = json!(self.renderer_sha256);
            view["baseline_tokens"] = json!(self.baseline.count);
        }
        Ok(view)
    }
}

pub(super) fn read(
    raw: Option<Value>,
    body: Option<&Value>,
    details: bool,
) -> Result<Option<Value>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let report: Report = match raw {
        Value::String(text) => serde_json::from_str(&text),
        value => serde_json::from_value(value),
    }
    .map_err(|e| format!("Invalid renderer receipt: {e}"))?;
    report
        .inspect(
            body.ok_or("Renderer receipt requires a captured request body")?,
            details,
        )
        .map(Some)
}
