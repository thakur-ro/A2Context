use nexusweave::{inspect, langsmith, otlp, replay, Availability, Basis, FindingKind, Run};
use serde_json::{json, Value};

const SEMANTICS: &str = "nexusweave.context.semantics";
const BODY: &str = r#"{ "messages": [{"role":"system","content":"Use 30 days."},{"role":"user","content":"Use 60 days."}], "seed": 9223372036854775807 }"#;

fn declarations(first: Value, second: Value, decision: Value) -> Value {
    json!({
        "version": 1,
        "context": [
            {"anchor": first, "source_id": "retail/return-policy", "source_version": "v12",
             "rule": {"key": "return_window_days", "scope": "standard", "value": 30}},
            {"anchor": second, "source_id": "retail/shopper-preference", "source_version": "v2",
             "authority": "Shopper preference",
             "rule": {"key": "return_window_days", "scope": "standard", "value": 60}}
        ],
        "decisions": [{"anchor": decision, "applies_to": {"key": "return_window_days", "scope": "standard"}}],
        "relationships": [{"context": first, "decision": decision, "relation": "applies to"}]
    })
}

fn otlp_export(metadata: &Value) -> Value {
    json!({"resourceSpans":[{"scopeSpans":[{"spans":[{
        "traceId":"11111111111111111111111111111111",
        "spanId":"2222222222222222",
        "attributes":[
            {"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},
            {"key":"gen_ai.conversation.id","value":{"stringValue":"otel-thread"}},
            {"key":"nexusweave.organization.id","value":{"stringValue":"retail"}},
            {"key":"nexusweave.user.id","value":{"stringValue":"shopper"}},
            {"key":"gen_ai.input.messages","value":{"stringValue":json!([
                {"role":"system","parts":[{"type":"text","content":"Use 30 days."}]},
                {"role":"user","parts":[{"type":"text","content":"Use 60 days."}]}
            ]).to_string()}},
            {"key":"gen_ai.output.messages","value":{"stringValue":json!([
                {"role":"assistant","parts":[{"type":"text","content":"30 days."}]}
            ]).to_string()}},
            {"key":SEMANTICS,"value":{"stringValue":metadata.to_string()}},
            {"key":"nexusweave.request.body","value":{"stringValue":BODY}}
        ]
    }]}]}]})
}

fn set_otlp_attribute(export: &mut Value, key: &str, value: String) {
    let attributes = export["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array_mut()
        .unwrap();
    let attribute = attributes
        .iter_mut()
        .find(|attribute| attribute["key"] == key)
        .unwrap();
    attribute["value"]["stringValue"] = json!(value);
}

fn otlp_declarations() -> Value {
    declarations(
        json!({"field":"gen_ai.input.messages","message_index":0,"part_index":0}),
        json!({"field":"gen_ai.input.messages","message_index":1,"part_index":0}),
        json!({"field":"gen_ai.output.messages","message_index":0,"part_index":0}),
    )
}

fn langsmith_export() -> Value {
    let mut metadata = json!({
        "organization_id":"retail", "user_id":"shopper", "thread_id":"langsmith-thread",
        "nexusweave.span.id":"ls-call",
        "nexusweave.request.body":BODY
    });
    metadata[SEMANTICS] = declarations(
        json!({"field":"inputs.messages[0].content"}),
        json!({"field":"inputs.messages[1].content"}),
        json!({"field":"outputs.output"}),
    );
    json!({
        "id":"ls-call", "run_type":"llm",
        "inputs":{"messages":[
            {"role":"system","content":"Use 30 days."}, {"role":"user","content":"Use 60 days."}
        ]},
        "outputs":{"output":"30 days."},
        "extra":{"metadata":metadata}
    })
}

#[test]
fn binds_request_fragments_in_otlp_and_langsmith() {
    let prefix = "Policy 🧭\n";
    let policy = "Use 30 days.";
    let body =
        json!({"messages":[{"content":format!("{prefix}{policy}\nOther inputs")}]}).to_string();
    let fragment = json!({
        "pointer":"/messages/0/content",
        "text_range":{"start_byte":prefix.len(),"end_byte":prefix.len() + policy.len()}
    });
    let mut metadata = otlp_declarations();
    metadata["context"][0]["request_fragment"] = fragment.clone();
    let mut export = otlp_export(&metadata);
    set_otlp_attribute(&mut export, "nexusweave.request.body", body.clone());
    let mut runs = otlp::import(&export).unwrap();
    let mut export = langsmith_export();
    let metadata = &mut export["extra"]["metadata"];
    metadata[SEMANTICS]["context"][0]["request_fragment"] = fragment.clone();
    metadata["nexusweave.request.body"] = json!(body);
    runs.extend(langsmith::import(&export).unwrap());
    let inspection = inspect(&runs).unwrap();
    for (run, capture) in runs.iter().zip(&inspection.capture_profiles) {
        let call = &run.calls[0];
        assert!(call.context_metadata.as_ref().unwrap().issue.is_none());
        assert_eq!(
            serde_json::to_value(&call.context[0].request_fragment).unwrap(),
            fragment
        );
        assert_eq!(capture.items[0].matching_fields, Some(1));
        assert_eq!(capture.items[1].matching_fields, Some(0));
    }
}

#[test]
fn projects_source_text_from_one_captured_message_and_links_it_to_decisions() {
    let prefix = "Policy 🧭\n";
    let policy = "Use 30 days.";
    let preference = "Use 60 days.";
    let prompt = format!("{prefix}{policy}\n{preference}");
    let body = json!({"messages":[{"role":"system","content":prompt}]}).to_string();
    let declaration = |anchor: Value| {
        let source = |id: &str, text: &str, start: usize, value: u64| {
            json!({
                "id": id, "source_id": format!("retail/{id}"), "source_version": "v1",
                "role": "system", "kind": "instruction",
                "request_fragment": {"pointer":"/messages/0/content",
                    "text_range":{"start_byte":start,"end_byte":start + text.len()}},
                "rule":{"key":"return_window_days","scope":"standard","value":value}
            })
        };
        json!({
            "version":1,
            "request_sources":[source("policy", policy, prefix.len(), 30),
                source("preference", preference, prefix.len() + policy.len() + 1, 60)],
            "decisions":[{"anchor":anchor,"applies_to":{"key":"return_window_days","scope":"standard"}}],
            "relationships":[{"context":{"request_source_id":"policy"},"decision":anchor,"relation":"applies to"}]
        })
    };
    let mut export = otlp_export(&declaration(
        json!({"field":"gen_ai.output.messages","message_index":0,"part_index":0}),
    ));
    set_otlp_attribute(&mut export, "nexusweave.request.body", body.clone());
    set_otlp_attribute(
        &mut export,
        "gen_ai.input.messages",
        json!([{"role":"system","parts":[{"type":"text","content":prompt}]}]).to_string(),
    );
    let mut runs = otlp::import(&export).unwrap();
    let mut export = langsmith_export();
    export["inputs"]["messages"] = json!([{"role":"system","content":prompt}]);
    export["extra"]["metadata"][SEMANTICS] = declaration(json!({"field":"outputs.output"}));
    export["extra"]["metadata"]["nexusweave.request.body"] = json!(body);
    runs.extend(langsmith::import(&export).unwrap());
    let view = inspect(&runs).unwrap();
    assert_eq!(view.findings.len(), 2);
    for (run, capture) in runs.iter().zip(&view.capture_profiles) {
        let call = &run.calls[0];
        assert_eq!(call.context.len(), 3);
        assert_eq!(call.context[0].text.as_deref(), Some(prompt.as_str()));
        assert!(call.context_metadata.as_ref().unwrap().issue.is_none());
        let item = &call.context[1];
        assert_eq!(item.id, "request-source:policy");
        assert_eq!(item.text.as_deref(), Some(policy));
        assert_eq!(
            item.evidence,
            call.recorded_request.as_ref().unwrap().evidence
        );
        assert_eq!(call.relationships[0].context_id, item.id);
        assert_eq!(call.relationships[0].basis, Basis::Declared);
        assert_eq!(capture.items[1].matching_fields, Some(1));
        let case: replay::Case = serde_json::from_value(json!({
            "version":1,"id":"without-policy","runner_id":"synthetic",
            "location":{"run_id":run.id,"call_id":call.id,"decision_id":call.decisions[0].id},
            "request":serde_json::from_str::<Value>(&body).unwrap(),"samples":1,
            "interventions":[{
                "context_id": item.id, "pointer":"/messages/0/content","operation":"replace_text_fragment",
                "text_range":item.request_fragment.as_ref().unwrap().text_range,"replacement":""
            }]
        })).unwrap();
        let mut messages = Vec::new();
        replay::execute(&case, &runs, |request| {
            messages.push(
                request["messages"][0]["content"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
            Some(replay::Choice {
                kind: call.decisions[0].kind.clone(),
                label: call.decisions[0].label.clone(),
                outcome: call.decisions[0].outcome.clone(),
            })
        })
        .unwrap();
        assert_eq!(
            messages,
            [prompt.clone(), format!("{prefix}\n{preference}")]
        );
    }
    let profile = view
        .source_profiles
        .iter()
        .find(|profile| profile.source_id == "retail/policy")
        .unwrap();
    assert_eq!(profile.occurrences.len(), 2);
    assert!(profile
        .occurrences
        .iter()
        .all(|item| item.linked_decision_ids.len() == 1));
}

#[test]
fn leaves_original_evidence_usable_when_request_source_declarations_fail() {
    let make_export = || {
        let mut export = langsmith_export();
        let metadata = &mut export["extra"]["metadata"];
        metadata[SEMANTICS]["request_sources"] = json!([{
            "id":"policy","source_id":"projected-policy",
            "request_fragment":{"pointer":"/messages/0/content","text_range":{"start_byte":0,"end_byte":12}}
        }]);
        export
    };
    let mut missing = make_export();
    missing["extra"]["metadata"]["nexusweave.request.body"] = json!("{");
    let mut invalid = make_export();
    invalid["extra"]["metadata"][SEMANTICS]["request_sources"][0]["request_fragment"]
        ["text_range"]["end_byte"] = json!(300);
    let mut duplicate = make_export();
    let sources = duplicate["extra"]["metadata"][SEMANTICS]["request_sources"]
        .as_array_mut()
        .unwrap();
    sources.push(sources[0].clone());
    let mut dangling = make_export();
    dangling["extra"]["metadata"][SEMANTICS]["relationships"][0]["context"] =
        json!({"request_source_id":"missing"});
    for (export, diagnostic) in [
        (missing, "captured request body"),
        (invalid, "UTF-8 text range"),
        (duplicate, "Duplicate request source ID"),
        (dangling, "Unknown request source ID"),
    ] {
        let runs = langsmith::import(&export).unwrap();
        let call = &runs[0].calls[0];
        assert_eq!(call.context.len(), 2);
        assert_eq!(call.context[0].text.as_deref(), Some("Use 30 days."));
        assert!(call.context[0].source_id.is_none());
        assert!(call.relationships.is_empty());
        assert!(call
            .context_metadata
            .as_ref()
            .unwrap()
            .issue
            .as_deref()
            .unwrap()
            .contains(diagnostic));
        inspect(&runs).unwrap();
    }
}

#[test]
fn does_not_use_metadata_inherited_from_a_different_langsmith_run() {
    let parent = langsmith_export();
    let mut child = parent.clone();
    child["id"] = json!("ls-child");
    child["trace_id"] = json!("ls-call");
    child["parent_run_id"] = json!("ls-call");
    let runs = langsmith::import(&json!([parent, child])).unwrap();
    let run = &runs[0];
    assert_eq!(run.calls.len(), 2);
    assert!(run.calls[0].context_metadata.is_some());
    assert!(run.calls[1].context_metadata.is_none());
    assert!(run.calls[1].recorded_request.is_none());
    assert!(run.calls[1].decisions[0].applies_to.is_none());
    assert_eq!(run.calls[1].decisions[0].outcome, json!("30 days."));
}

#[test]
fn combines_versioned_rule_findings_across_importers_and_verifies_captured_replays() {
    let mut runs = otlp::import(&otlp_export(&otlp_declarations())).unwrap();
    runs.extend(langsmith::import(&langsmith_export()).unwrap());

    for run in &runs {
        let call = &run.calls[0];
        assert_eq!(
            call.context[0].source_id.as_deref(),
            Some("retail/return-policy")
        );
        assert_eq!(call.context[1].source_version.as_deref(), Some("v2"));
        assert_eq!(
            call.context[1].authority.as_deref(),
            Some("Shopper preference")
        );
        assert_eq!(call.context[1].text.as_deref(), Some("Use 60 days."));
        let binding = call.context_metadata.as_ref().unwrap();
        assert!(binding.issue.is_none());
        assert_eq!(call.relationships[0].basis, Basis::Declared);
        assert_eq!(
            call.relationships[0].evidence.as_ref(),
            Some(&binding.evidence)
        );
        let capture = call.recorded_request.as_ref().unwrap();
        assert_eq!(capture.body, BODY);
        assert_eq!(capture.evidence.span_id, call.span_id);
        assert!(capture.evidence.field.contains("nexusweave.request.body"));

        let mut case: replay::Case = serde_json::from_value(json!({
            "version":1, "id":"withhold-policy", "runner_id":"synthetic",
            "location":{"run_id":run.id,"call_id":call.id,"decision_id":call.decisions[0].id},
            "samples":1, "request":serde_json::from_str::<Value>(BODY).unwrap(),
            "interventions":[{"context_id":call.context[0].id,"pointer":"/messages/0/content",
                              "replacement":"[context omitted]"}]
        }))
        .unwrap();
        let report = replay::execute(&case, &runs, |request| {
            Some(replay::Choice {
                kind: "model_output".into(),
                label: "Model answer".into(),
                outcome: if request["messages"][0]["content"] == "[context omitted]" {
                    json!("60 days.")
                } else {
                    json!("30 days.")
                },
            })
        })
        .unwrap();
        assert_eq!(report.variants[0].difference_pp, Some(-100.0));
        case.request["seed"] = json!(0);
        assert!(replay::execute(&case, &runs, |_| panic!(
            "baseline must be verified before sampling"
        ))
        .is_err());
    }

    let report = inspect(&runs).unwrap();
    assert_eq!(report.findings.len(), 2);
    assert!(report
        .findings
        .iter()
        .all(|finding| finding.kind == FindingKind::Conflict));
    let shopper = report
        .patterns
        .iter()
        .find(|group| group.user_id.as_deref() == Some("shopper"))
        .unwrap();
    assert_eq!(shopper.affected_conversations, Some(2));
    assert_eq!(shopper.affected_runs, 2);
    let policy = report
        .source_profiles
        .iter()
        .find(|profile| profile.source_id == "retail/return-policy")
        .unwrap();
    assert_eq!(policy.occurrences.len(), 2);
    let [first, second] = &policy.occurrences[..] else {
        panic!("expected two source occurrences");
    };
    assert_ne!(first.trace_source, second.trace_source);
    assert_eq!(first.source_version, second.source_version);
    assert_eq!(first.text_sha256, second.text_sha256);
    assert_eq!(first.linked_decision_ids.len(), 1);
    assert_eq!(second.linked_decision_ids.len(), 1);
}

#[test]
fn preserves_the_trace_when_a_declaration_cannot_be_bound() {
    let mut metadata = otlp_declarations();
    metadata["relationships"][0]["decision"]["part_index"] = json!(8);
    let runs = otlp::import(&otlp_export(&metadata)).unwrap();
    let call = &runs[0].calls[0];
    assert!(call.context[0].source_id.is_none());
    assert!(call.decisions[0].applies_to.is_none());
    assert!(call.relationships.is_empty());
    assert_eq!(call.decisions[0].outcome, json!("30 days."));
    assert!(call.recorded_request.is_some());
    assert_eq!(
        call.context_metadata.as_ref().unwrap().evidence.field,
        SEMANTICS
    );
    assert!(call
        .context_metadata
        .as_ref()
        .unwrap()
        .issue
        .as_ref()
        .unwrap()
        .contains("part index 8"));

    let mut export = langsmith_export();
    export["extra"]["metadata"][SEMANTICS] = json!("{\"version\":");
    let runs = langsmith::import(&export).unwrap();
    let call = &runs[0].calls[0];
    assert_eq!(call.context[0].text.as_deref(), Some("Use 30 days."));
    assert!(call
        .context_metadata
        .as_ref()
        .unwrap()
        .issue
        .as_ref()
        .unwrap()
        .contains("Invalid context declaration"));
    assert_eq!(
        serde_json::from_str::<Vec<Run>>(&serde_json::to_string(&runs).unwrap()).unwrap(),
        runs
    );
}

#[test]
fn retains_content_references_and_keeps_unavailable_rule_values_out_of_evidence() {
    let mut export = otlp_export(&json!({
        "version":1, "context":[{"anchor":{"field":"gen_ai.input.messages_ref"}, "source_id":"retail/return-policy"}]
    }));
    let attributes = export["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array_mut()
        .unwrap();
    let input = attributes
        .iter_mut()
        .find(|attribute| attribute["key"] == "gen_ai.input.messages")
        .unwrap();
    input["key"] = json!("gen_ai.input.messages_ref");
    input["value"]["stringValue"] = json!("urn:source:policy");
    let runs = otlp::import(&export).unwrap();
    let context = &runs[0].calls[0].context[0];
    assert_eq!(context.source_id.as_deref(), Some("retail/return-policy"));
    assert_eq!(
        context.content_reference.as_deref(),
        Some("urn:source:policy")
    );
    assert_eq!(context.availability, Availability::Unknown);
    assert!(context.text.is_none());

    let attributes = export["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array_mut()
        .unwrap();
    let metadata = attributes
        .iter_mut()
        .find(|attribute| attribute["key"] == SEMANTICS)
        .unwrap();
    let mut declaration: Value =
        serde_json::from_str(metadata["value"]["stringValue"].as_str().unwrap()).unwrap();
    declaration["context"][0]["rule"] =
        json!({"key":"return_window_days","scope":"standard","value":30});
    metadata["value"]["stringValue"] = json!(declaration.to_string());
    let runs = otlp::import(&export).unwrap();
    assert!(runs[0].calls[0].context[0].rule.is_none());
    assert!(runs[0].calls[0]
        .context_metadata
        .as_ref()
        .unwrap()
        .issue
        .as_ref()
        .unwrap()
        .contains("unavailable context"));
}
