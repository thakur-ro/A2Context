use a2context::pending;
use serde_json::{json, Value};

fn inspection(input: Value) -> Value {
    pending::inspect(serde_json::from_value(input).unwrap()).unwrap()
}

#[test]
fn inspects_rules_and_editable_locations_without_a_recorded_call() {
    let request: Value =
        serde_json::from_str(include_str!("../examples/preflight-returns.json")).unwrap();
    let view = inspection(json!({ "request": request }));
    assert_eq!(view["preflight"]["findings"][0]["kind"], "conflict");
    assert!(view["preflight"]["receipt"]["fingerprint"].is_string());
    assert_eq!(view["screening"]["status"], "matched");
    assert!(view["screening"]["report"]["scorer"].is_null());
    assert!(view["request_fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["fields"][0]["pointer"] == "/messages/0/content"
                && item["fields"][0]["text_range"]["end_byte"].is_number()
        }));
    assert!(view.get("request").is_none());
}

#[test]
fn compares_the_named_catalog_and_keeps_unknown_tools_and_missing_capture_visible() {
    let envelope = |body: Value| {
        json!({
            "request":{"version":1,"attributes":{"a2context.request.body":body.to_string()}},
            "allowed_tools":["fare"]
        })
    };
    let input = envelope(json!({"tools":[{"name":"fare"},
        {"type":"function","function":{"name":"book"}},
        {"type":"web_search","name":"builtin"}, {"name":" "}]}));
    assert_eq!(
        inspection(input.clone())["tool_scope"],
        json!({
            "captured":4,"outside":[{"name":"book","index":1}],"unclassified":[2,3]
        })
    );
    let query = serde_json::from_value(json!({"input":input})).unwrap();
    assert_eq!(
        pending::evidence(query, false).unwrap()["tool_scope"],
        json!({"captured":4,"outside_count":1,"unclassified_count":2})
    );
    assert_eq!(
        inspection(envelope(json!({})))["tool_scope"]["captured"],
        Value::Null
    );
    assert_eq!(
        inspection(envelope(json!({"tools":[]})))["tool_scope"]["captured"],
        0
    );
    let mut input = envelope(json!({"tools":[{"name":"fare"}]}));
    input["allowed_tools"] = json!([]);
    assert_eq!(
        inspection(input.clone())["tool_scope"]["outside"][0]["name"],
        "fare"
    );
    input["allowed_tools"] = json!([" "]);
    assert!(pending::inspect(serde_json::from_value(input).unwrap()).is_err());
}

#[test]
fn renderer_receives_independent_removals_in_captured_order_and_preserves_rejections() {
    use pending::rendering::{measure, Rendered};
    let request = serde_json::from_value(json!({"version":1,"attributes":{
        "a2context.request.body":r#"{"middle":2,"a/~b":1,"tail":[7,8]}"#
    }}))
    .unwrap();
    let receipt = measure(request, vec!["/a~1~0b".into(), "/tail/0".into()], |batch| {
        assert_eq!(
            batch["requests"][1].to_string(),
            r#"{"middle":2,"tail":[7,8]}"#
        );
        assert_eq!(
            batch["requests"][2].to_string(),
            r#"{"middle":2,"a/~b":1,"tail":[8]}"#
        );
        Ok(Rendered {
            renderer: "test".into(),
            renderer_sha256: "a".repeat(64),
            tokens: vec![Some(vec![1, 2]), Some(vec![2, 1]), None],
        })
    })
    .unwrap();
    let view = inspection(json!({"request":receipt}));
    assert_eq!(view["render_check"]["unchanged"], 0);
    assert_eq!(view["render_check"]["fields"][0]["status"], "changed");
    assert_eq!(view["render_check"]["fields"][1]["status"], "unrenderable");
}
