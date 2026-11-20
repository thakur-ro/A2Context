//! Compare a workflow's permitted function names with its assembled tool catalog.
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

/// A function present in the request outside the declared workflow scope.
#[derive(Serialize)]
pub struct OutsideTool {
    pub name: String,
    /// Index in the request body's `tools` array.
    pub index: usize,
}

/// A missing catalog and an explicitly empty catalog have different meanings.
#[derive(Serialize)]
pub struct ToolScope {
    pub captured: Option<usize>,
    pub outside: Vec<OutsideTool>,
    /// Entries whose function names could not be established.
    pub unclassified: Vec<usize>,
}

pub(super) fn check(body: Option<&Value>, allowed: &[String]) -> Result<ToolScope, String> {
    if allowed.iter().any(|name| name.trim().is_empty()) {
        return Err("Each allowed tool needs a nonempty function name.".into());
    }
    let allowed: BTreeSet<_> = allowed.iter().map(String::as_str).collect();
    let catalog = body
        .and_then(|body| body.get("tools"))
        .and_then(Value::as_array);
    let mut check = ToolScope {
        captured: catalog.map(Vec::len),
        outside: Vec::new(),
        unclassified: Vec::new(),
    };
    for (index, tool) in catalog.into_iter().flatten().enumerate() {
        // Vendor-hosted tool types can carry names with different dispatch semantics.
        let name = if tool.get("type").is_none_or(|kind| kind == "function") {
            match tool.get("function") {
                Some(function) => function
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty()),
                None => crate::genai::tool_name(tool),
            }
        } else {
            None
        };
        match name {
            None => check.unclassified.push(index),
            Some(name) if !allowed.contains(name) => check.outside.push(OutsideTool {
                name: name.to_owned(),
                index,
            }),
            Some(_) => {}
        }
    }
    Ok(check)
}
