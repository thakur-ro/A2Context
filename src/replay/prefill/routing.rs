//! Cross one instruction with one tool inventory while replaying the same user-source edit.
use super::{body_with_edit, digest, excerpt, Probe, ATTRIBUTE};
use crate::replay::{Choice, Intervention, Trial};
use crate::{preflight, Call};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Design {
    /// Verified user-message edit to replay in every setup.
    pub context_id: String,
    /// Position of the system or developer message to vary.
    pub instruction_index: usize,
    /// Text for that message in the alternate condition.
    pub alternate_instruction: String,
    /// Complete tool declarations for the alternate condition.
    pub alternate_tools: Vec<Value>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Report {
    version: u8,
    runner_id: String,
    design: Design,
    cells: [[Arm; 2]; 4],
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Arm {
    request_sha256: String,
    trial: Trial,
}

fn tool_names(tools: &Value) -> Result<Vec<&str>, String> {
    tools
        .as_array()
        .ok_or("Expected an array of tool definitions.")?
        .iter()
        .map(|t| {
            t.pointer("/function/name")
                .or_else(|| t.get("name"))
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| "Each tool needs a name.".into())
        })
        .collect()
}

impl Design {
    fn conditions<'a>(
        &'a self,
        probe: &Probe,
        call: &Call,
        body: &'a Value,
    ) -> Result<impl Fn(usize, usize) -> Value + 'a, String> {
        let edit: &Intervention = probe
            .interventions
            .iter()
            .find(|edit| edit.context_id == self.context_id)
            .ok_or("Choose an edit from the verified first-token report.")?;
        let messages = body["messages"]
            .as_array()
            .ok_or("Expected chat messages.")?;
        let index = self.instruction_index;
        let instruction = messages.get(index).ok_or("Missing instruction message.")?;
        let Some(original) = instruction["content"].as_str() else {
            return Err("Choose a text instruction.".into());
        };
        if !matches!(instruction["role"].as_str(), Some("system" | "developer"))
            || self.alternate_instruction == original
        {
            return Err(
                "Choose a system or developer message and different replacement text.".into(),
            );
        }
        if !messages.iter().enumerate().any(|(i, m)| {
            m["role"] == "user"
                && (edit.pointer == format!("/messages/{i}/content")
                    || edit.pointer.starts_with(&format!("/messages/{i}/content/")))
        }) {
            return Err("The replay edit must target a separate user message's content.".into());
        }
        let alternate_tools = json!(self.alternate_tools);
        tool_names(&body["tools"])?;
        tool_names(&alternate_tools)?;
        if body["tools"] == alternate_tools {
            return Err("Supply a different tool inventory.".into());
        }
        let edited = body_with_edit(call, body, edit)?;
        if body == &edited {
            return Err("The tested source edit leaves the request unchanged.".into());
        }
        Ok(move |cell, arm| {
            let mut request = if arm == 0 {
                body.clone()
            } else {
                edited.clone()
            };
            if cell >= 2 {
                request["messages"][index]["content"] = json!(self.alternate_instruction);
            }
            if cell % 2 == 1 {
                request["tools"] = alternate_tools.clone();
            }
            request
        })
    }
}

pub(super) fn checked(
    raw: &Value,
    probe: &Probe,
    call: &Call,
    body: &Value,
    excerpts: bool,
) -> Result<Value, String> {
    let report: Report =
        serde_json::from_value(raw.clone()).map_err(|_| "Unreadable routing comparison.")?;
    if report.version != 1 || report.runner_id.trim().is_empty() {
        return Err("Invalid routing comparison version or runner.".into());
    }
    let condition = report.design.conditions(probe, call, body)?;
    let mut rows = Vec::new();
    for (cell, pair) in report.cells.iter().enumerate() {
        let mut trials = Vec::new();
        for (index, arm) in pair.iter().enumerate() {
            let request = condition(cell, index);
            if digest(&request) != arm.request_sha256 {
                return Err("Routing trial differs from the reconstructed request.".into());
            }
            let mut view = json!({"request_sha256":arm.request_sha256,"status":"failed"});
            if let Trial::Completed { choice } = &arm.trial {
                if !matches!(choice.kind.as_str(), "tool_call" | "model_output")
                    || choice.label.trim().is_empty()
                {
                    return Err("Unrecognized routing trial choice.".into());
                }
                view["status"] = json!("completed");
                view["kind"] = json!(choice.kind);
                if choice.kind == "tool_call" {
                    view["listed"] =
                        json!(tool_names(&request["tools"])?.contains(&choice.label.as_str()));
                }
                if excerpts {
                    view["label"] = json!(excerpt(&choice.label));
                    view["outcome"] = json!(excerpt(&choice.outcome.to_string()));
                }
            }
            trials.push(view);
        }
        let changed = match (&pair[0].trial, &pair[1].trial) {
            (Trial::Completed { choice: a }, Trial::Completed { choice: b }) => Some(a != b),
            _ => None,
        };
        rows.push(json!({"changed":changed,"trials":trials}));
    }
    let mut view =
        json!({"runner":report.runner_id,"context_id":report.design.context_id,"cells":rows});
    let shifts: Vec<_> = rows.iter().map(|cell| cell["changed"].as_bool()).collect();
    view["pattern"] = json!(match shifts.as_slice() {
        [Some(a), Some(b), Some(c), Some(d)] if a == c && b == d && a != b => "tools",
        [Some(a), Some(b), Some(c), Some(d)] if a == b && c == d && a != c => "instruction",
        [Some(a), Some(b), Some(c), Some(d)] if a == b && a == c && a == d => "constant",
        [Some(_), Some(_), Some(_), Some(_)] => "combined",
        _ => "incomplete",
    });
    if excerpts {
        view["instructions"] = json!([
            excerpt(
                body["messages"][report.design.instruction_index]["content"]
                    .as_str()
                    .unwrap()
            ),
            excerpt(&report.design.alternate_instruction)
        ]);
        view["inventories"] = json!([
            tool_names(&body["tools"])?,
            tool_names(&json!(report.design.alternate_tools))?
        ]);
    }
    Ok(view)
}

/// Run eight proposal-only trials on a previously scored request with an explicit adapter.
pub fn measure(
    mut input: preflight::Request,
    design: Design,
    runner: &str,
    mut propose: impl FnMut(&Value) -> Option<Choice>,
) -> Result<preflight::Request, String> {
    if runner.trim().is_empty() {
        return Err("Supply a runner identifier.".into());
    }
    let (call, body, _) = preflight::pending_call(input.clone())?;
    let body = body.ok_or("A routing comparison requires the captured request body.")?;
    let mut probe = call
        .prefill_probe
        .as_ref()
        .and_then(|c| c.report.clone())
        .ok_or("Supply a request with a readable first-token report.")?;
    probe.checked(&call, &body, false)?;
    let cells = {
        let condition = design.conditions(&probe, &call, &body)?;
        std::array::from_fn(|cell| {
            std::array::from_fn(|arm| {
                let request = condition(cell, arm);
                Arm {
                    request_sha256: digest(&request),
                    trial: propose(&request)
                        .map_or(Trial::Failed, |choice| Trial::Completed { choice }),
                }
            })
        })
    };
    let raw = json!(Report {
        version: 1,
        runner_id: runner.into(),
        design,
        cells
    });
    checked(&raw, &probe, &call, &body, false)?;
    probe.routing_test = Some(raw);
    input.attributes.insert(ATTRIBUTE.into(), json!(probe));
    Ok(input)
}
