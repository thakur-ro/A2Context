//! Inspect a proposed call against the required fields in the tool definition sent to the model.
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;

struct Definition<'a> {
    schema: &'a Value,
    pointer: String,
}

/// Function names can repeat in a malformed catalog. Only a unique definition supplies evidence.
pub struct Catalog<'a>(BTreeMap<&'a str, Option<Definition<'a>>>);

#[derive(Debug, PartialEq, Serialize)]
pub struct FieldDeclaration {
    pub field: String,
    pub tool: String,
    pub pointer: String,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Assessment {
    MissingRequired {
        field: String,
        schema_pointer: String,
        argument_pointer: String,
        supplied_fields: Vec<String>,
        supplied_count: usize,
        other_declarations: Vec<FieldDeclaration>,
    },
    /// No missing field was found along the inspected object properties and array items.
    NoMissingFieldObserved,
    Unassessed {
        reason: &'static str,
    },
}

impl<'a> Catalog<'a> {
    /// Accepts chat-completion functions, flat Responses functions and Anthropic tool definitions.
    pub fn from_request(body: &'a Value) -> Self {
        let mut catalog = BTreeMap::new();
        for (i, tool) in body["tools"].as_array().into_iter().flatten().enumerate() {
            let (function, key, prefix) =
                if tool["type"] == "function" && tool["function"].is_object() {
                    (
                        &tool["function"],
                        "parameters",
                        format!("/tools/{i}/function"),
                    )
                } else if tool["type"] == "function" {
                    (tool, "parameters", format!("/tools/{i}"))
                } else if tool.get("input_schema").is_some() {
                    (tool, "input_schema", format!("/tools/{i}"))
                } else {
                    continue;
                };
            if let Some(name) = function["name"].as_str() {
                catalog
                    .entry(name)
                    .and_modify(|v| *v = None)
                    .or_insert(Some(Definition {
                        schema: &function[key],
                        pointer: format!("{prefix}/{key}"),
                    }));
            }
        }
        Self(catalog)
    }

    /// Return the unique declared schema and its JSON Pointer in the request.
    pub fn schema(&self, name: &str) -> Option<(&Value, &str)> {
        let def = self.0.get(name)?.as_ref()?;
        Some((def.schema, &def.pointer))
    }

    /// Accept parsed arguments or a provider's JSON-encoded arguments string. Returns the first witness.
    pub fn check(&self, name: &str, arguments: &Value) -> Assessment {
        let Some((schema, pointer)) = self.schema(name) else {
            return Assessment::Unassessed {
                reason: "No unique captured definition for this tool",
            };
        };
        let args = match arguments {
            Value::String(text) => match serde_json::from_str(text) {
                Ok(v) => Cow::Owned(v),
                Err(_) => {
                    return Assessment::Unassessed {
                        reason: "Arguments are not complete JSON",
                    }
                }
            },
            value => Cow::Borrowed(value),
        };
        if !args.is_object() || !schema.is_object() {
            return Assessment::Unassessed {
                reason: "Expected a captured argument object and schema object",
            };
        }
        if schema.get("$ref").is_some() {
            return Assessment::Unassessed {
                reason: "Referenced schemas are not resolved",
            };
        }
        let Some(mut issue) = inspect(schema, &args, pointer, "") else {
            return Assessment::NoMissingFieldObserved;
        };
        if let Assessment::MissingRequired {
            schema_pointer,
            supplied_fields,
            other_declarations,
            ..
        } = &mut issue
        {
            let root_requirement = schema_pointer.starts_with(&format!("{pointer}/required/"));
            if root_requirement {
                *other_declarations = self.other_declarations(name, schema, supplied_fields);
            }
        }
        issue
    }
    // A shared field name supplies a comparison to test, not a causal attribution.
    fn other_declarations(
        &self,
        target: &str,
        schema: &Value,
        fields: &[String],
    ) -> Vec<FieldDeclaration> {
        let mut matches = Vec::new();
        let Some(own) = schema["properties"].as_object() else {
            return matches;
        };
        for field in fields.iter().filter(|k| !own.contains_key(*k)) {
            for (&tool, def) in &self.0 {
                let Some(def) = def else { continue };
                if tool != target
                    && def.schema.get("$ref").is_none()
                    && def.schema["properties"].get(field).is_some()
                {
                    matches.push(FieldDeclaration {
                        field: field.clone(),
                        tool: tool.into(),
                        pointer: format!("{}/properties/{}", def.pointer, token(field)),
                    });
                    if matches.len() == 3 {
                        return matches;
                    }
                }
            }
        }
        matches
    }
}

fn token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn inspect(schema: &Value, value: &Value, schema_path: &str, path: &str) -> Option<Assessment> {
    // Older schema dialects can ignore siblings of $ref. Skip this branch without resolving references.
    if schema.get("$ref").is_some() {
        return None;
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema["required"]
            .as_array()
            .filter(|r| r.iter().all(Value::is_string))
        {
            for (i, key) in required.iter().filter_map(Value::as_str).enumerate() {
                if !object.contains_key(key) {
                    return Some(Assessment::MissingRequired {
                        field: key.into(),
                        schema_pointer: format!("{schema_path}/required/{i}"),
                        argument_pointer: format!("{path}/{}", token(key)),
                        supplied_fields: object.keys().take(8).cloned().collect(),
                        supplied_count: object.len(),
                        other_declarations: Vec::new(),
                    });
                }
            }
        }
        if let Some(properties) = schema["properties"].as_object() {
            for (k, v) in object {
                if let Some(child) = properties.get(k) {
                    let k = token(k);
                    if let Some(issue) = inspect(
                        child,
                        v,
                        &format!("{schema_path}/properties/{k}"),
                        &format!("{path}/{k}"),
                    ) {
                        return Some(issue);
                    }
                }
            }
        }
    } else if let Some(items) = value.as_array() {
        if schema["items"].is_object() && schema.get("prefixItems").is_none() {
            for (i, v) in items.iter().enumerate() {
                if let Some(issue) = inspect(
                    &schema["items"],
                    v,
                    &format!("{schema_path}/items"),
                    &format!("{path}/{i}"),
                ) {
                    return Some(issue);
                }
            }
        }
    }
    None
}
