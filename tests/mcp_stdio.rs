use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

const BIN: &str = env!("CARGO_BIN_EXE_nexusweave");
const ACTIVATION_REPORTS: &str = "examples/local-mlx-timeline/cohort/activation-reports.json";
const ACTIVATION_EVIDENCE: &str = "examples/local-mlx-timeline/cohort/evidence.json";

struct Client {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<Value>,
    id: u32,
    modern: bool,
}

impl Client {
    fn start(args: &[&str], modern: bool) -> Self {
        let mut child = Command::new(BIN)
            .arg("mcp-stdio")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let value: Value = serde_json::from_str(&line).unwrap();
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            stdin,
            responses,
            id: 0,
            modern,
        };
        if modern {
            let result = client.request("server/discover", json!({}));
            assert!(result["supportedVersions"]
                .as_array()
                .unwrap()
                .contains(&json!("2026-07-28")));
        } else {
            let result = client.request(
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25", "capabilities": {},
                    "clientInfo": {"name": "nexusweave-test", "version": "1"},
                }),
            );
            assert_eq!(result["protocolVersion"], "2025-11-25");
            client.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        }
        client
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, mut params: Value) -> Value {
        if self.modern {
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
            });
        }
        self.id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}));
        let response = self
            .responses
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], self.id);
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn live_mcp_applies_host_scope_as_trace_identity_arrives() {
    let directory = tempfile::tempdir().unwrap();
    let scope = directory.path().join("scope.json");
    std::fs::write(
        &scope,
        r#"{"kind":"user","organization_id":"demo-retail","user_id":"inventory-user"}"#,
    )
    .unwrap();
    let mut client = Client::start(
        &[
            "--scope",
            scope.to_str().unwrap(),
            "--otlp-store",
            directory.path().to_str().unwrap(),
        ],
        false,
    );
    let tools = client.request("tools/list", json!({}));
    assert!(tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "query_activation_effects"));
    let location =
        json!({"run_id":"otlp:0123456789abcdef0123456789abc001", "call_id":"0000000000000002"});
    let before = client.call("get_turn_evidence", location.clone());
    assert_eq!(before["isError"], true);

    let fixture: Value = serde_json::from_str(include_str!("../fixtures/otlp.json")).unwrap();
    let spans = &fixture["resourceSpans"][0]["scopeSpans"][0]["spans"];
    let publish = |number, id| {
        let mut batch = fixture.clone();
        let span = spans
            .as_array()
            .unwrap()
            .iter()
            .find(|span| span["spanId"] == id)
            .unwrap();
        batch["resourceSpans"][0]["scopeSpans"][0]["spans"] = json!([span]);
        let pending = directory.path().join(".pending");
        std::fs::write(&pending, batch.to_string()).unwrap();
        std::fs::rename(pending, directory.path().join(format!("{number:020}.json"))).unwrap();
    };
    publish(1, "0000000000000002");
    let unowned = client.call("get_turn_evidence", location.clone());
    assert_eq!(unowned["content"], before["content"]);
    publish(2, "0000000000000001");
    let visible = client.call("get_turn_evidence", location);
    assert_eq!(visible["structuredContent"]["user_id"], "inventory-user");
    assert_eq!(visible["structuredContent"]["call_id"], "0000000000000002");
}

#[test]
fn activation_tool_queries_and_drills_into_only_the_host_scoped_measurements() {
    let dir = tempfile::tempdir().unwrap();
    let scope_file = dir.path().join("scope.json");
    std::fs::write(
        &scope_file,
        json!({
            "kind": "user", "organization_id": "demo-retail-north", "user_id": "customer-01"
        })
        .to_string(),
    )
    .unwrap();
    let mut client = Client::start(
        &[
            "--scope",
            scope_file.to_str().unwrap(),
            "--activation-report",
            ACTIVATION_REPORTS,
            ACTIVATION_EVIDENCE,
        ],
        false,
    );
    let listed = client.request("tools/list", json!({}));
    let tool = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "query_activation_effects")
        .unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    let schema: Value =
        serde_json::from_str(include_str!("../schema/activation-effects-query-1.json")).unwrap();
    assert_eq!(tool["inputSchema"], schema);

    let mut spec = json!({
        "version": 1, "block": 0, "source_id": "returns-policy", "checkpoint": "Refund timing", "limit": 1
    });
    let response = client.call("query_activation_effects", spec.clone());
    let page = &response["structuredContent"];
    let group = &page["groups"][0];
    assert_eq!(page["total_groups"], 1);
    assert_eq!(group["decision_count"], 3);
    assert_eq!(group["lower_score_count"], 3);
    assert_eq!(group["flipped_count"], 1);
    assert_eq!(
        group["change_log_odds"],
        json!({"minimum": -12.0, "maximum": -8.25})
    );
    assert!(group.get("contrasts").is_none());
    assert!(!response.to_string().contains("Window: 30 days"));
    assert!(!response.to_string().contains("demo-retail-south"));
    let key = group["key"].as_str().unwrap();
    assert_eq!(key.len(), 64);
    spec["group"] = group["key"].clone();
    for offset in 0..3 {
        spec["offset"] = json!(offset);
        let response = client.call("query_activation_effects", spec.clone());
        let page = &response["structuredContent"];
        assert_eq!(page["total_occurrences"], 3);
        assert_eq!(
            page["next_offset"],
            if offset < 2 {
                json!(offset + 1)
            } else {
                Value::Null
            }
        );
        let occurrence = &page["occurrences"][0];
        assert_eq!(occurrence["organization_id"], "demo-retail-north");
        assert_eq!(occurrence["user_id"], "customer-01");
        assert_eq!(occurrence["context_id"], "returns-policy");
        assert!(occurrence["inspector_path"]
            .as_str()
            .unwrap()
            .ends_with("&checkpoint=1&block=0"));
        assert!(occurrence["source_inspector_path"]
            .as_str()
            .unwrap()
            .ends_with("&context=returns%2Dpolicy"));
    }
    let outside = json!({"kind": "organization", "organization_id": "demo-retail-south"});
    spec["scope"] = outside.clone();
    assert_eq!(
        client.call("query_activation_effects", spec)["isError"],
        true
    );
    let empty = client.call(
        "query_activation_effects",
        json!({"version": 1, "block": 0, "scope": outside}),
    );
    assert_eq!(empty["structuredContent"]["total_groups"], 0);
    let blocked = client.call(
        "query_activation_effects",
        json!({"version": 1, "block": 0, "include_excerpts": true}),
    );
    assert_eq!(blocked["isError"], true);
}

#[test]
fn activation_tool_matches_full_thread_identity_and_gates_edit_excerpts() {
    let mut client = Client::start(
        &[
            "--include-excerpts",
            "--activation-report",
            ACTIVATION_REPORTS,
            ACTIVATION_EVIDENCE,
        ],
        true,
    );
    let mut spec = json!({
        "version": 1, "block": 0, "source_id": "returns-policy", "checkpoint": "Return window",
        "thread": {
            "trace_source": "local-mlx", "organization_id": "demo-retail-south",
            "user_id": "customer-01", "conversation_id": "return-instructions-e"
        }
    });
    let first = client.call("query_activation_effects", spec.clone());
    let first = &first["structuredContent"];
    assert_eq!(first["groups"][0]["decision_count"], 1);
    assert_eq!(first["groups"][0]["flipped_count"], 1);
    assert!(first["groups"][0].get("contrasts").is_none());
    spec["include_excerpts"] = json!(true);
    let opt_in = client.call("query_activation_effects", spec.clone());
    assert_eq!(
        opt_in["structuredContent"]["groups"][0]["contrasts"]["after"]["text"],
        "60"
    );
    assert!(!opt_in
        .to_string()
        .contains("What is the standard return window"));
    spec["thread"]["organization_id"] = json!("demo-retail-north");
    let other_owner = client.call("query_activation_effects", spec);
    assert_eq!(other_owner["structuredContent"]["total_groups"], 0);
}

#[test]
fn output_token_effects_page_through_a_scoped_report_and_gate_token_text() {
    let dir = tempfile::tempdir().unwrap();
    let evidence = dir.path().join("evidence.json");
    let scope = dir.path().join("scope.json");
    let mut tool_runs: Value =
        serde_json::from_str(include_str!("../examples/local-mlx-routing/evidence.json")).unwrap();
    tool_runs[0]["organization_id"] = "allowed".into();
    let mut runs = tool_runs.as_array().unwrap().clone();
    let extra: Vec<Value> =
        serde_json::from_str(include_str!("../examples/runtime-comparison/evidence.json")).unwrap();
    runs.extend(extra);
    std::fs::write(&evidence, serde_json::to_vec(&runs).unwrap()).unwrap();
    std::fs::write(
        &scope,
        r#"{"kind":"organization","organization_id":"allowed"}"#,
    )
    .unwrap();
    let mut client = Client::start(
        &[
            "--scope",
            scope.to_str().unwrap(),
            "--activation-report",
            "examples/local-mlx-routing/output-token-report.json",
            "--activation-report",
            "examples/runtime-comparison/output-token-report-transformers.json",
            "--include-excerpts",
            evidence.to_str().unwrap(),
        ],
        true,
    );
    let output = "get_output_token_effects";
    let tools = client.request("tools/list", json!({}));
    assert!(tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| { tool["name"] == output && tool["annotations"]["readOnlyHint"] == true }));
    let mut request = json!({
        "version": 1, "block": 0, "offset": 7, "limit": 1,
        "report_id": "lfm-product-route-activation-output-tokens",
        "location": {
            "run_id": "local-model-product-route", "call_id": "model-inference",
            "decision_id": "product-route"
        }
    });
    let turn = client.call("get_turn_evidence", request["location"].clone());
    let available = &turn["structuredContent"]["output_score_reports"][0];
    assert_eq!(available["report_id"], request["report_id"]);
    assert_eq!(available["total_output_tokens"], 22);
    let summary = client.call(output, request.clone());
    let payload = &summary["structuredContent"];
    let token = &payload["tokens"][0];
    assert_eq!(payload["readout"], "output_token_log_probability");
    assert_eq!(payload["total_output_tokens"], 22);
    assert_eq!(payload["next_offset"], 8);
    assert_eq!(token["output_token"], 7);
    assert!(token.get("fragment").is_none() && token.get("token_id").is_none());
    assert!(token["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&token=7&block=0"));
    let product = token["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["context_id"] == "product-id")
        .unwrap();
    assert!(product["change_in_log_probability"].as_f64().unwrap() < -20.0);
    request["include_excerpts"] = true.into();
    request["offset"] = 20.into();
    let with_text = client.call(output, request.clone());
    assert_eq!(
        with_text["structuredContent"]["tokens"][0]["fragment"]["text"],
        "42"
    );
    request["location"]["run_id"] = "transformers-return-and-refund".into();
    request["location"]["decision_id"] = "return-and-refund".into();
    request["report_id"] = "qwen-return-and-refund-activation-output-tokens".into();
    let denied = client.call(output, request);
    assert_eq!(denied["isError"], true);
    assert!(denied.get("structuredContent").is_none());
}

#[test]
fn generation_branch_queries_page_and_hold_generated_text_until_opt_in() {
    let mut client = Client::start(
        &[
            "--include-excerpts",
            "--activation-report",
            "examples/runtime-comparison/generation-branches-transformers.json",
            "examples/runtime-comparison/evidence.json",
        ],
        true,
    );
    let loc = json!({
        "run_id": "transformers-return-and-refund", "call_id": "model-inference",
        "decision_id": "return-and-refund"
    });
    let turn = client.call("get_turn_evidence", loc.clone());
    let report = &turn["structuredContent"]["generation_branch_reports"][0];
    assert_eq!(report["total_branches"], 4);
    let mut args =
        json!({"version": 1, "location": loc, "report_id": report["report_id"], "limit": 1});
    let tool = "get_generation_branches";
    let early = client.call(tool, args.clone());
    let early = &early["structuredContent"];
    assert_eq!(early["next_offset"], 1);
    assert_eq!(
        early["branches"][0]["patched"]["first_changed_output_token"],
        5
    );
    assert!(early["branches"][0]["patched"]
        .get("text_excerpt")
        .is_none());
    assert!(early["branches"][0]["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&branch=0&block=0"));
    args["offset"] = 1.into();
    args["include_excerpts"] = true.into();
    let late = client.call(tool, args);
    let branch = &late["structuredContent"]["branches"][0];
    assert_eq!(branch["start_output_token"], 13);
    assert_eq!(branch["patched"]["first_changed_output_token"], Value::Null);
    assert_eq!(
        branch["patched"]["text_excerpt"]["text"],
        "The return window is 3 days and the refund time is 4 days."
    );
}

#[test]
fn source_visibility_queries_follow_the_turn_to_ranked_sources_and_pairs() {
    let mut client = Client::start(
        &[
            "--include-excerpts",
            "--activation-report",
            "examples/runtime-comparison/paired-visibility-transformers.json",
            "examples/runtime-comparison/paired-visibility-evidence.json",
        ],
        true,
    );
    let name = "query_source_visibility";
    let tools = client.request("tools/list", json!({}));
    assert!(tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == name && tool["annotations"]["readOnlyHint"] == true));
    let location = json!({"run_id":"qwen-redundant-refund-policy", "call_id":"model-inference", "decision_id":"return-and-refund"});
    let turn = client.call("get_turn_evidence", location.clone());
    let report = &turn["structuredContent"]["source_visibility_reports"][0];
    assert_eq!(report["measured_sources"], 6);
    assert_eq!(report["measured_pairs"], 2);

    let mut request = json!({"version":1, "location":location, "report_id":report["report_id"], "offset":13, "limit":1});
    let timeline = client.call(name, request.clone());
    let timeline = &timeline["structuredContent"];
    assert_eq!(timeline["next_offset"], 14);
    assert_eq!(timeline["rows"][0]["output_token"], 13);
    assert!(timeline["rows"][0].get("fragment").is_none());
    let strongest = &timeline["rows"][0]["strongest_pair"];
    assert_eq!(
        strongest["context_ids"],
        json!(["refund-policy", "refund-policy-copy"])
    );
    assert!(strongest["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&visibility=13&pair=0"));

    request["offset"] = 0.into();
    request["view"] = json!({"kind":"sources", "output_token":13});
    let sources = client.call(name, request.clone());
    let sources = &sources["structuredContent"];
    assert_eq!(sources["total_rows"], 6);
    assert_eq!(sources["rows"][0]["context_id"], "returns-policy");
    assert!(sources["rows"][0]["source_inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&context=returns%2Dpolicy"));
    request["view"]["context_id"] = "refund-policy-copy".into();
    let exact_source = client.call(name, request.clone());
    assert_eq!(exact_source["structuredContent"]["total_rows"], 1);
    assert_eq!(
        exact_source["structuredContent"]["rows"][0]["context_id"],
        "refund-policy-copy"
    );
    assert!(
        exact_source["structuredContent"]["rows"][0]["inspector_path"]
            .as_str()
            .unwrap()
            .ends_with("&visibility=13&mask=refund%2Dpolicy%2Dcopy")
    );

    request["view"] = json!({"kind":"pairs","output_token":13});
    let paired = client.call(name, request.clone());
    let paired = &paired["structuredContent"];
    let row = &paired["rows"][0];
    assert_eq!(row["inspector_path"], strongest["inspector_path"]);
    assert!(row["interaction_log_units"].as_f64().unwrap() < -5.8);
    assert!(
        row["only_first"]["change_in_log_probability"]
            .as_f64()
            .unwrap()
            .abs()
            < 0.001
    );
    assert!(
        row["only_second"]["change_in_log_probability"]
            .as_f64()
            .unwrap()
            .abs()
            < 0.001
    );
    request["offset"] = paired["next_offset"].clone();
    request["include_excerpts"] = true.into();
    let control = client.call(name, request.clone());
    assert_eq!(
        control["structuredContent"]["token"]["fragment"]["text"],
        "4"
    );
    assert_eq!(
        control["structuredContent"]["rows"][0]["context_ids"],
        json!(["refund-policy", "store-name"])
    );
    assert!(control["structuredContent"]["next_offset"].is_null());
    request["offset"] = 0.into();
    request["view"]["context_ids"] = json!(["store-name", "refund-policy"]);
    let exact_pair = client.call(name, request.clone());
    let exact_pair = &exact_pair["structuredContent"];
    assert_eq!(exact_pair["total_rows"], 1);
    assert_eq!(
        exact_pair["rows"][0]["inspector_path"],
        control["structuredContent"]["rows"][0]["inspector_path"]
    );
    request["view"]["output_token"] = 16.into();
    assert_eq!(client.call(name, request)["isError"], true);
}

#[test]
fn visibility_patterns_group_scoped_measurements_and_link_chronological_occurrences() {
    let mut client = Client::start(
        &[
            "--activation-report",
            "examples/runtime-comparison/visibility-cohort/reports.json",
            "examples/runtime-comparison/visibility-cohort/evidence.json",
        ],
        true,
    );
    let name = "query_visibility_patterns";
    let mut args =
        json!({"version":1,"kind":"pair","output_token":13,"source_id":"refund-policy-copy"});
    let all = client.call(name, args.clone());
    let group = &all["structuredContent"]["groups"][0];
    assert_eq!(all["structuredContent"]["total_groups"], 1);
    assert_eq!(group["conversation_count"], 4);
    assert_eq!(group["user_count"], 3);
    assert_eq!(group["organization_count"], 2);
    assert_eq!(group["negative_decisions"], 4);
    assert!(group["effect"]["maximum"].as_f64().unwrap() < -5.1);

    args["group"] = group["key"].clone();
    args["scope"] = json!({"kind":"user","organization_id":"retail-north","user_id":"sample-user"});
    args["limit"] = 1.into();
    let north = client.call(name, args.clone());
    let north = &north["structuredContent"];
    assert_eq!(north["total_occurrences"], 2);
    assert_eq!(north["occurrences"][0]["conversation_id"], "north-1");
    assert_eq!(north["next_offset"], 1);
    assert!(north["occurrences"][0]["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&visibility=13&pair=0"));
    args["after"] = "2026-11-05T12:00:00Z".into();
    args["before"] = "2026-11-08T12:00:00Z".into();
    args["thread"] = json!({"trace_source":"local-transformers","organization_id":"retail-north","user_id":"sample-user","conversation_id":"north-2"});
    let matched = client.call(name, args.clone());
    assert_eq!(matched["structuredContent"]["total_occurrences"], 1);
    assert_eq!(
        matched["structuredContent"]["occurrences"][0]["conversation_id"],
        "north-2"
    );
    args["min_conversations"] = 2.into();
    assert_eq!(client.call(name, args)["isError"], true);
}

#[test]
fn mask_continuations_link_exact_occurrences_and_gate_generated_text() {
    let fixtures = [
        "--activation-report",
        "examples/runtime-comparison/visibility-routing/continuations.json",
        "examples/runtime-comparison/visibility-routing/evidence.json",
    ];
    let mut client = Client::start(&fixtures, true);
    let location = json!({"run_id":"qwen-product-route","call_id":"model-inference","decision_id":"product-route"});
    let turn = client.call("get_turn_evidence", location.clone());
    let report = &turn["structuredContent"]["source_visibility_reports"][0];
    assert_eq!(report["measured_continuations"], 3);
    let mut args = json!({
        "version":1,"location":location,"report_id":report["report_id"],"limit":1,
        "view":{"kind":"continuations","output_token":7}
    });
    let page = client.call("query_source_visibility", args.clone());
    let page = &page["structuredContent"];
    assert_eq!(page["readout"], "masked_greedy_continuation");
    assert_eq!(page["total_rows"], 2);
    assert_eq!(page["next_offset"], 1);
    let row = &page["rows"][0];
    assert_eq!(row["outcome"], "changed_tool");
    assert_eq!(row["first_changed_output_token"], 7);
    assert!(row.get("text_excerpt").is_none() && row.get("proposed_tool").is_none());
    assert!(row["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&visibility=7&continuation=0"));
    let mut pattern_args = json!({"version":1,"target":"routing.tool","source_id":"stock"});
    let groups = client.call("query_visibility_patterns", pattern_args.clone());
    pattern_args["group"] = groups["structuredContent"]["groups"][0]["key"].clone();
    let detail = client.call("query_visibility_patterns", pattern_args);
    assert_eq!(
        detail["structuredContent"]["occurrences"][0]["mask_continuations"][0],
        *row
    );

    args["view"]["context_ids"] = json!(["tool-stock", "tool-price"]);
    let pair = client.call("query_source_visibility", args.clone());
    assert_eq!(pair["structuredContent"]["total_rows"], 1);
    assert_eq!(
        pair["structuredContent"]["rows"][0]["outcome"],
        "changed_arguments"
    );
    args["include_excerpts"] = true.into();
    assert_eq!(
        client.call("query_source_visibility", args.clone())["isError"],
        true
    );

    let mut client = Client::start(&[&["--include-excerpts"][..], &fixtures].concat(), true);
    args["view"] = json!({"kind":"continuations","output_token":17,"context_ids":["product-id"]});
    let product = client.call("query_source_visibility", args);
    let row = &product["structuredContent"]["rows"][0];
    assert_eq!(row["outcome"], "changed_arguments");
    assert_eq!(row["first_changed_output_token"], 17);
    assert_eq!(row["proposed_tool"]["text"], "stock");
    assert!(row["text_excerpt"]["text"]
        .as_str()
        .unwrap()
        .contains("12345"));
}

#[test]
fn routing_patterns_compare_recorded_tools_and_keep_exact_scoped_turns() {
    let mut client = Client::start(
        &[
            "--activation-report",
            "examples/runtime-comparison/visibility-routing-cohort/reports.json",
            "examples/runtime-comparison/visibility-routing-cohort/evidence.json",
        ],
        true,
    );
    let name = "query_visibility_patterns";
    let base = json!({"version":1, "target":"routing.tool", "source_id":"stock"});
    let mut args = base.clone();
    args["tool_name"] = "stock".into();
    args["compare"] = json!({"tool_name":"price"});
    let result = client.call(name, args.clone());
    let page = &result["structuredContent"];
    assert_eq!(page["coverage_a"]["reports_with_target"], 3);
    assert_eq!(page["coverage_b"]["reports_with_target"], 4);
    let group = &page["groups"][0];
    assert_eq!(page["total_groups"], 1);
    assert_eq!(group["scope_a"]["organization_count"], 2);
    assert_eq!(group["scope_b"]["organization_count"], 2);
    assert_eq!(group["scope_a"]["conversation_count"], 3);
    assert_eq!(group["scope_b"]["conversation_count"], 4);
    assert!(group["scope_a"]["effect"]["maximum"].as_f64().unwrap() < -3.4);
    assert!(group["scope_b"]["effect"]["minimum"].as_f64().unwrap() > 0.0);

    args = base;
    args["group"] = group["key"].clone();
    args["scope"] = json!({"kind":"user","organization_id":"retail-north","user_id":"analyst"});
    let user = client.call(name, args.clone());
    assert_eq!(user["structuredContent"]["group"]["conversation_count"], 2);
    assert_eq!(user["structuredContent"]["total_occurrences"], 3);
    args["thread"] = json!({
        "trace_source":"local-transformers","organization_id":"retail-north",
        "user_id":"analyst","conversation_id":"north-1"
    });
    let thread = client.call(name, args.clone());
    let turns = &thread["structuredContent"]["occurrences"];
    assert_eq!(turns[0]["tool_name"], "stock");
    assert_eq!(turns[1]["tool_name"], "price");
    assert_eq!(turns[1]["turn_id"], "price-check");
    assert_eq!(turns[1]["location"]["run_id"], "qwen-routing-north-1-price");
    assert!(turns[1]["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&visibility=7&mask=tool%2Dstock"));

    args["timeline"] = json!({"unit":"day"});
    args["tool_name"] = "price".into();
    let day = client.call(name, args.clone());
    let periods = &day["structuredContent"]["rows"];
    assert_eq!(day["structuredContent"]["total_periods"], 1);
    assert_eq!(periods[0]["period"], "2026-11-01");
    assert_eq!(periods[0]["scope_a"]["decision_count"], 1);
    args["tool_name"] = " ".into();
    assert_eq!(client.call(name, args)["isError"], true);
}

#[test]
fn visibility_pattern_comparisons_keep_both_scopes_and_page_them_independently() {
    let mut client = Client::start(
        &[
            "--activation-report",
            "examples/runtime-comparison/visibility-cohort/reports.json",
            "examples/runtime-comparison/visibility-cohort/evidence.json",
        ],
        true,
    );
    let name = "query_visibility_patterns";
    let north = json!({"kind":"organization","organization_id":"retail-north"});
    let mut args = json!({
        "version":1, "kind":"pair", "output_token":13, "source_id":"refund-policy-copy",
        "scope": north, "compare":{"scope":{"kind":"organization","organization_id":"retail-south"}},
        "min_conversations":2, "limit":1
    });
    let result = client.call(name, args.clone());
    let groups = &result["structuredContent"];
    assert_eq!(groups["kind"], "visibility_pattern_comparison");
    assert_eq!(groups["total_groups"], 1);
    let group = &groups["groups"][0];
    assert_eq!(group["scope_a"]["conversation_count"], 3);
    assert_eq!(group["scope_b"]["conversation_count"], 1);
    assert_ne!(group["scope_a"]["effect"], group["scope_b"]["effect"]);

    args["group"] = group["key"].clone();
    args["compare"] = json!({"scope":north,"after":"2026-11-05T00:00:00Z"});
    args["compare_offset"] = 1.into();
    let detail = client.call(name, args.clone());
    let detail = &detail["structuredContent"];
    assert_eq!(detail["kind"], "visibility_pattern_comparison_evidence");
    assert_eq!(detail["scope_a"]["total_occurrences"], 3);
    assert_eq!(
        detail["scope_a"]["occurrences"][0]["conversation_id"],
        "north-1"
    );
    assert_eq!(detail["scope_a"]["next_offset"], 1);
    assert_eq!(detail["scope_b"]["total_occurrences"], 2);
    assert_eq!(
        detail["scope_b"]["occurrences"][0]["conversation_id"],
        "north-3"
    );
    assert!(detail["scope_b"]["next_offset"].is_null());
    assert!(detail["scope_b"]["occurrences"][0]["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&visibility=13&pair=0"));

    args["compare"]["after"] = "2026-11-19T00:00:00Z".into();
    let absent = client.call(name, args.clone());
    assert!(absent["structuredContent"]["scope_b"].is_null());
    assert_eq!(
        absent["structuredContent"]["scope_a"]["total_occurrences"],
        3
    );

    let dir = tempfile::tempdir().unwrap();
    let restriction = dir.path().join("scope.json");
    std::fs::write(&restriction, north.to_string()).unwrap();
    let mut restricted = Client::start(
        &[
            "--scope",
            restriction.to_str().unwrap(),
            "--activation-report",
            "examples/runtime-comparison/visibility-cohort/reports.json",
            "examples/runtime-comparison/visibility-cohort/evidence.json",
        ],
        true,
    );
    args["compare"] = json!({"scope":{"kind":"organization","organization_id":"retail-south"}});
    let constrained = restricted.call(name, args);
    assert_eq!(
        constrained["structuredContent"]["scope_a"]["total_occurrences"],
        3
    );
    assert_eq!(
        constrained["structuredContent"]["coverage_b"]["reports_at_token"],
        0
    );
    assert!(constrained["structuredContent"]["scope_b"].is_null());
}

#[test]
fn tool_proposal_visibility_links_measured_arguments_to_named_patterns() {
    let mut client = Client::start(
        &[
            "--activation-report",
            "examples/runtime-comparison/visibility-routing/report.json",
            "examples/runtime-comparison/visibility-routing/evidence.json",
        ],
        true,
    );
    let location = json!({"run_id":"qwen-product-route", "call_id":"model-inference", "decision_id":"product-route"});
    let turn = client.call("get_turn_evidence", location.clone());
    let report = &turn["structuredContent"]["source_visibility_reports"][0];
    assert_eq!(report["decision_kind"], "tool_call");
    assert_eq!(report["measured_sources"], 7);
    assert_eq!(report["measured_pairs"], 4);
    assert!(report.get("baseline_tool_call").is_none());

    let sources = client.call(
        "query_source_visibility",
        json!({
            "version":1, "location":location, "report_id":report["report_id"],
            "view":{"kind":"sources","output_token":17,"context_id":"product-id"}
        }),
    );
    let row = &sources["structuredContent"]["rows"][0];
    assert_eq!(row["context_id"], "product-id");
    assert!(row.get("fragment").is_none());

    let definitions = client.call(
        "query_source_visibility",
        json!({
            "version":1, "location":location, "report_id":report["report_id"],
            "view":{"kind":"sources","output_token":7,"context_id":"tool-stock"}
        }),
    );
    let definition = &definitions["structuredContent"]["rows"][0];
    assert_eq!(definition["request_pointer"], "/tools/0");
    assert_eq!(definition["token_range"], json!([71, 129]));
    assert_eq!(
        definition["boundary_whitespace"],
        json!({"before":"","after":"\n"})
    );
    assert!(
        (definition["effect"]["change_in_log_probability"]
            .as_f64()
            .unwrap()
            + 3.428282)
            .abs()
            < 1e-6
    );
    let pair = client.call(
        "query_source_visibility",
        json!({
            "version":1, "location":location, "report_id":report["report_id"],
            "view":{"kind":"pairs","output_token":7,"context_ids":["tool-stock","tool-price"]}
        }),
    );
    let interaction = pair["structuredContent"]["rows"][0]["interaction_log_units"]
        .as_f64()
        .unwrap();
    assert!((interaction - 3.412300).abs() < 1e-6);

    let instruction = client.call(
        "query_source_visibility",
        json!({
            "version":1, "location":location, "report_id":report["report_id"],
            "view":{"kind":"sources","output_token":7,"context_id":"routing-instruction"}
        }),
    );
    let instruction = &instruction["structuredContent"]["rows"][0];
    assert_eq!(instruction["token_range"], json!([3, 36]));
    assert_eq!(
        instruction["boundary_whitespace"],
        json!({"before":"","after":"\n\n"})
    );

    let name = "query_visibility_patterns";
    let routing_groups = client.call(
        name,
        json!({"version":1, "target":"routing.tool", "limit":2}),
    );
    let routing_groups = &routing_groups["structuredContent"]["groups"];
    assert_eq!(
        routing_groups[0]["comparison"]["sources"][0]["source_id"],
        "stock"
    );
    assert_eq!(
        routing_groups[1]["comparison"]["sources"][0]["source_id"],
        "request-type"
    );

    let mut request = json!({
        "version":1, "kind":"source", "target":"routing.product_id", "source_id":"product-id"
    });
    let result = client.call(name, request.clone());
    assert_eq!(result["structuredContent"]["total_groups"], 1);
    let group = &result["structuredContent"]["groups"][0];
    assert_eq!(group["comparison"]["decision_kind"], "tool_call");
    request["group"] = group["key"].clone();
    let detail = client.call(name, request);
    let occurrence = &detail["structuredContent"]["occurrences"][0];
    assert_eq!(occurrence["output_token"], 17);
    assert_eq!(occurrence["output_token_end"], 19);
    assert!((occurrence["effect"].as_f64().unwrap() + 4.864170).abs() < 1e-6);
    assert!(occurrence["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&visibility=17&mask=product%2Did"));
}

#[test]
fn visibility_targets_align_different_prefixes_and_keep_exact_span_links() {
    let mut client = Client::start(
        &[
            "--activation-report",
            "examples/runtime-comparison/visibility-cohort/reports.json",
            "--activation-report",
            "examples/runtime-comparison/visibility-target/report.json",
            "examples/runtime-comparison/visibility-cohort/evidence.json",
            "examples/runtime-comparison/visibility-target/evidence.json",
        ],
        true,
    );
    let name = "query_visibility_patterns";
    let target = "returns.refund_duration";
    let tools = client.request("tools/list", json!({}));
    let tool = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap();
    let uri = tool["_meta"]["ui"]["resourceUri"].as_str().unwrap();
    let resource = client.request("resources/read", json!({"uri":uri}));
    assert_eq!(
        resource["contents"][0]["mimeType"],
        "text/html;profile=mcp-app"
    );
    assert!(resource["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Masked-source effects"));
    let turn = client.call("get_turn_evidence", json!({
        "run_id":"qwen-target-refund-only", "call_id":"model-inference", "decision_id":"refund-duration"
    }));
    assert_eq!(
        turn["structuredContent"]["source_visibility_reports"][0]["targets"],
        json!([{"key":target,"token_range":[7,9]}])
    );

    let mut args = json!({
        "version":1, "kind":"pair", "target":target, "source_id":"refund-policy-copy",
        "min_conversations":2
    });
    let result = client.call(name, args.clone());
    let page = &result["structuredContent"];
    assert_eq!(page["total_groups"], 1);
    assert_eq!(page["coverage"]["reports_with_target"], 3);
    assert_eq!(page["coverage"]["reports_in_scope"], 5);
    let group = &page["groups"][0];
    assert_eq!(group["comparison"]["target"], target);
    assert_eq!(group["conversation_count"], 3);
    assert_eq!(group["output_prefix_count"], 2);

    args["scope"] = json!({"kind":"organization","organization_id":"retail-north"});
    args["compare"] = json!({"scope":{"kind":"organization","organization_id":"retail-south"}});
    args["group"] = group["key"].clone();
    let details = client.call(name, args.clone());
    let details = &details["structuredContent"];
    let north = &details["scope_a"]["occurrences"][0];
    let south = &details["scope_b"]["occurrences"][0];
    assert_eq!(north["output_token"], 13);
    assert_eq!(north["output_token_end"], 15);
    assert_eq!(south["output_token"], 7);
    assert_eq!(south["output_token_end"], 9);
    assert_eq!(south["conversation_id"], "south-refund-only");
    assert!((south["effect"].as_f64().unwrap() - (-4.187813)).abs() < 1e-6);
    assert_ne!(north["output_prefix_sha256"], south["output_prefix_sha256"]);
    assert!(south["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&visibility=7&pair=0"));

    args["timeline"] = json!({"unit":"week"});
    args["limit"] = 1.into();
    let week = client.call(name, args.clone());
    let week = &week["structuredContent"];
    assert_eq!(week["kind"], "visibility_pattern_timeline");
    assert_eq!(week["total_periods"], 3);
    assert_eq!(week["rows"][0]["period"], "2026-10-26");
    assert_eq!(week["rows"][0]["scope_a"]["conversation_count"], 1);
    assert!(week["rows"][0]["scope_b"].is_null());
    assert_eq!(week["next_offset"], 1);
    args["offset"] = 2.into();
    let later = client.call(name, args.clone());
    let later = &later["structuredContent"];
    assert_eq!(later["rows"][0]["period"], "2026-11-09");
    assert_eq!(
        later["maximum_absolute_effect"],
        week["maximum_absolute_effect"]
    );
    assert_eq!(later["rows"][0]["scope_b"]["decision_count"], 1);
    assert!(later["next_offset"].is_null());
    args["offset"] = 0.into();
    args["timeline"]["period"] = later["rows"][0]["period"].clone();
    let selected = client.call(name, args.clone());
    let selected = &selected["structuredContent"];
    assert!(selected["scope_a"].is_null());
    assert_eq!(
        selected["scope_b"]["occurrences"][0]["inspector_path"],
        south["inspector_path"]
    );

    args["output_token"] = 13.into();
    assert_eq!(client.call(name, args)["isError"], true);
}

#[test]
fn context_source_queries_scope_recorded_coverage_and_link_measurements() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("one-report.json");
    let batch: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-cohort/reports.json"
    ))
    .unwrap();
    std::fs::write(&path, batch[0].to_string()).unwrap();
    let cohort = "examples/runtime-comparison/visibility-cohort/evidence.json";
    let mut client = Client::start(
        &[
            "--activation-report",
            path.to_str().unwrap(),
            "--replay",
            "examples/local-mlx-tool/report.json",
            cohort,
            "examples/local-mlx-tool/evidence.json",
        ],
        true,
    );
    let name = "query_context_sources";
    let listed = client.request("tools/list", json!({}));
    let definition = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap();
    assert_eq!(definition["annotations"]["readOnlyHint"], true);
    let output = client.call(name, json!({"version":1, "limit":1}));
    assert_eq!(
        output["structuredContent"]["kind"],
        "context_source_profiles"
    );
    assert_eq!(output["structuredContent"]["next_offset"], 1);
    assert_eq!(
        output["structuredContent"]["rows"][0]["source_id"],
        "format-instruction"
    );

    let mut request = json!({"version":1, "source_id":"refund-policy-copy", "limit":1});
    let result = client.call(name, request.clone());
    let page = &result["structuredContent"];
    assert_eq!(page["kind"], "context_source_occurrences");
    assert_eq!(page["source"]["occurrence_count"], 4);
    assert_eq!(page["source"]["conversation_count"], 4);
    assert_eq!(page["source"]["organization_count"], 2);
    assert_eq!(page["source"]["user_count"], 3);
    assert_eq!(page["source"]["linked_decision_count"], 0);
    assert_eq!(page["source"]["replayed_occurrences"], 0);
    assert_eq!(page["source"]["masked_occurrences"], 1);
    assert_eq!(page["next_offset"], 1);
    let first = &page["rows"][0];
    assert_eq!(first["conversation_id"], "north-1");
    assert_eq!(first["mask_measurement_count"], 1);
    assert_eq!(first["mask_example"]["output_token"], 3);
    assert!(first["mask_example"]["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&visibility=3&mask=refund%2Dpolicy%2Dcopy"));
    assert!(first.get("text").is_none());

    request["after"] = "2026-11-05T12:00:00Z".into();
    request["before"] = "2026-11-08T12:00:00Z".into();
    request["source_version"] = "v1".into();
    let later = client.call(name, request.clone());
    let later = &later["structuredContent"];
    assert_eq!(later["total_occurrences"], 1);
    assert_eq!(later["source"]["masked_occurrences"], 0);
    assert_eq!(later["rows"][0]["conversation_id"], "north-2");
    assert!(later["rows"][0].get("mask_example").is_none());

    let replay = client.call(name, json!({"version":1, "source_id":"get_store_hours"}));
    let replay = &replay["structuredContent"];
    assert_eq!(replay["source"]["replayed_occurrences"], 1);
    assert_eq!(replay["rows"][0]["replay_example"]["difference_pp"], 0.0);
    assert!(replay["rows"][0]["replay_example"]["inspector_path"]
        .as_str()
        .unwrap()
        .ends_with("&variant=get%5Fstore%5Fhours"));
    let linked = client.call(name, json!({"version":1, "source_id":"look_up_inventory"}));
    assert_eq!(
        linked["structuredContent"]["source"]["linked_decision_count"],
        1
    );

    let scope = temp.path().join("scope.json");
    std::fs::write(
        &scope,
        r#"{"kind":"organization","organization_id":"retail-south"}"#,
    )
    .unwrap();
    let mut restricted = Client::start(
        &[
            "--scope",
            scope.to_str().unwrap(),
            "--activation-report",
            path.to_str().unwrap(),
            cohort,
        ],
        true,
    );
    let mut args = json!({"version":1, "source_id":"refund-policy-copy"});
    let south = restricted.call(name, args.clone());
    assert_eq!(south["structuredContent"]["total_occurrences"], 1);
    assert_eq!(
        south["structuredContent"]["source"]["masked_occurrences"],
        0
    );
    let mut comparison = args.clone();
    comparison["compare"] =
        json!({"scope":{"kind":"organization","organization_id":"retail-north"}});
    let restricted_comparison = restricted.call(name, comparison);
    assert_eq!(
        restricted_comparison["structuredContent"]["scope_a"]["total_occurrences"],
        1
    );
    assert!(restricted_comparison["structuredContent"]["scope_b"].is_null());
    args["thread"] = json!({"trace_source":"local-transformers","organization_id":"retail-north",
        "user_id":"sample-user","conversation_id":"north-1"});
    let outside = restricted.call(name, args);
    assert_eq!(outside["structuredContent"]["total_occurrences"], 0);
    assert!(outside["structuredContent"]["source"].is_null());
}

#[test]
fn context_source_comparison_retains_both_versions_and_paginates_by_scope() {
    let temp = tempfile::tempdir().unwrap();
    let mut runs: Value = serde_json::from_str(include_str!(
        "../examples/runtime-comparison/visibility-cohort/evidence.json"
    ))
    .unwrap();
    runs[3]["calls"][0]["context"][3]["source_version"] = "v2".into();
    runs[3]["calls"][0]["context"][5]["source_id"] = "south-only-question".into();
    let path = temp.path().join("versions.json");
    std::fs::write(&path, runs.to_string()).unwrap();
    let mut client = Client::start(&[path.to_str().unwrap()], true);
    let north = json!({"kind":"organization","organization_id":"retail-north"});
    let south = json!({"kind":"organization","organization_id":"retail-south"});
    let mut args =
        json!({"version":1, "search":"V2", "limit":1, "scope":north, "compare":{"scope":south}});
    let name = "query_context_sources";
    let result = client.call(name, args.clone());
    let page = &result["structuredContent"];
    assert_eq!(page["kind"], "context_source_profile_comparison");
    assert_eq!(page["total_sources"], 1);
    assert_eq!(page["rows"][0]["source_id"], "refund-policy-copy");
    assert_eq!(
        page["rows"][0]["scope_a"]["versions"][0]["source_version"],
        "v1"
    );
    assert_eq!(
        page["rows"][0]["scope_b"]["versions"][0]["source_version"],
        "v2"
    );

    args["source_id"] = "refund-policy-copy".into();
    args["offset"] = 1.into();
    let result = client.call(name, args.clone());
    let page = &result["structuredContent"];
    assert_eq!(page["kind"], "context_source_occurrence_comparison");
    assert_eq!(page["scope_a"]["rows"][0]["conversation_id"], "north-2");
    assert_eq!(page["scope_b"]["rows"][0]["conversation_id"], "south-1");
    assert_eq!(page["scope_a"]["next_offset"], 2);
    assert!(page["scope_b"]["next_offset"].is_null());

    args["search"] = "".into();
    args["compare"]["before"] = "2026-11-11T12:00:00Z".into();
    let result = client.call(name, args);
    assert_eq!(
        result["structuredContent"]["scope_a"]["total_occurrences"],
        3
    );
    assert!(result["structuredContent"]["scope_b"].is_null());

    let only_south = client.call(
        name,
        json!({"version":1,"scope":north,"compare":{"scope":south},"search":"south-only"}),
    );
    assert!(only_south["structuredContent"]["rows"][0]["scope_a"].is_null());
    assert_eq!(
        only_south["structuredContent"]["rows"][0]["scope_b"]["occurrence_count"],
        1
    );
}

#[test]
fn legacy_client_gets_the_cli_counts_and_cannot_widen_host_scope() {
    let dir = tempfile::tempdir().unwrap();
    let scope = dir.path().join("scope.json");
    let restriction = json!({"kind":"user","organization_id":"demo-retail","user_id":"user-042"});
    std::fs::write(&scope, restriction.to_string()).unwrap();
    let evidence = "fixtures/support.json";
    let mut client = Client::start(&["--scope", scope.to_str().unwrap(), evidence], false);
    let listed = client.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 4);
    assert!(tools
        .iter()
        .all(|tool| tool["annotations"]["readOnlyHint"] == true));
    let query_tool = tools
        .iter()
        .find(|tool| tool["name"] == "query_conflicts")
        .unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../schema/conflict-view-1.json")).unwrap();
    assert_eq!(query_tool["inputSchema"], schema);
    let uri = query_tool["_meta"]["ui"]["resourceUri"].as_str().unwrap();
    let resources = client.request("resources/list", json!({}));
    assert_eq!(resources["resources"][0]["uri"], uri);
    let page = client.request("resources/read", json!({"uri":uri}));
    assert_eq!(page["contents"][0]["mimeType"], "text/html;profile=mcp-app");
    let html = page["contents"][0]["text"].as_str().unwrap();
    assert!(html.contains("<script type=\"module\">"));
    assert!(!html.contains("Use a 30-day window for standard returns."));

    let mut spec = json!({"version": 1, "group_by": ["source_version", "rule"]});
    let actual = client.call("query_conflicts", spec.clone());
    let text: Value = serde_json::from_str(actual["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, actual["structuredContent"]);
    spec["scope"] = restriction;
    let view = dir.path().join("view.json");
    std::fs::write(&view, spec.to_string()).unwrap();
    let cli = Command::new(BIN)
        .arg("query-conflicts")
        .arg(view)
        .arg(evidence)
        .output()
        .unwrap();
    assert!(cli.status.success());
    let expected: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(expected, actual["structuredContent"]);
    assert_eq!(expected["totals"]["conflicting_decisions"], 3);

    let outside = client.call(
        "get_turn_evidence",
        json!({
            "run_id":"other-store","call_id":"check","decision_id":"eligibility"
        }),
    );
    assert_eq!(outside["isError"], true);
    assert!(outside.get("structuredContent").is_none());
    let invalid = client.call(
        "query_conflicts",
        json!({"version":1,"group_by":["unregistered"]}),
    );
    assert_eq!(invalid["isError"], true);
    let mut request =
        json!({"run_id":"return-eligibility","call_id":"check","decision_id":"eligibility"});
    let turn = client.call("get_turn_evidence", request.clone());
    let payload = &turn["structuredContent"];
    assert_eq!(payload["decision"]["outcome_excerpt"], Value::Null);
    assert_eq!(payload["rows"][0]["finding"], "conflict");
    assert_eq!(payload["rows"][0]["inputs"][0]["excerpt"], Value::Null);
    assert!(!turn.to_string().contains("Use a 30-day window"));
    request["include_excerpts"] = json!(true);
    assert_eq!(client.call("get_turn_evidence", request)["isError"], true);
}

#[test]
fn modern_client_can_follow_a_turn_and_read_measured_replays() {
    let mut client = Client::start(
        &[
            "--include-excerpts",
            "--replay",
            "examples/local-mlx/report.json",
            "fixtures/conversation.json",
            "examples/local-mlx/evidence.json",
        ],
        true,
    );
    let capabilities = client.call("describe_dimensions", json!({}));
    assert_eq!(
        capabilities["structuredContent"]["captured_excerpts_enabled"],
        true
    );
    let args = json!({
        "run_id":"jacket-return-trace", "call_id":"jacket-answer", "decision_id":"answer",
        "include_excerpts":true, "limit":1,
    });
    let first = client.call("get_turn_evidence", args.clone());
    let turn = &first["structuredContent"];
    assert_eq!(turn["counts"]["recorded_calls"], 2);
    assert_eq!(turn["next_offset"], 1);
    assert_eq!(
        turn["rows"][0]["inputs"][0]["excerpt"]["text"],
        "Standard returns are accepted within 30 days."
    );
    assert!(turn["inspector_path"]
        .as_str()
        .unwrap()
        .contains("view=conversation"));

    let lookup = client.call(
        "get_turn_evidence",
        json!({
            "run_id":"jacket-return-trace", "call_id":"look-up-order", "decision_id":"lookup",
        }),
    );
    assert_eq!(
        lookup["structuredContent"]["recorded_tool_result"]["call_id"],
        "jacket-answer"
    );
    let report = client.call("get_turn_evidence", json!({
        "run_id":"local-model-returns", "call_id":"model-inference", "decision_id":"return-window",
    }));
    let rows = report["structuredContent"]["rows"].as_array().unwrap();
    let measured = rows
        .iter()
        .find(|row| row["kind"] == "replay_test" && row["result"]["context_id"] == "returns-policy")
        .unwrap();
    assert_eq!(measured["result"]["difference_pp"].as_f64(), Some(-100.0));
    assert_eq!(
        report["structuredContent"]["original_request_captured"],
        false
    );
    assert_eq!(report["structuredContent"]["counts"]["replay_tests"], 3);
}

#[test]
fn lists_decisions_before_resolving_evidence_for_a_multi_decision_call() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("multi.json");
    let mut fixture: Value =
        serde_json::from_str(include_str!("../fixtures/conversation.json")).unwrap();
    let decisions = fixture[0]["calls"][2]["decisions"].as_array_mut().unwrap();
    let mut other = decisions[0].clone();
    other["id"] = json!("second-answer");
    other["applies_to"] = Value::Null;
    decisions.push(other);
    std::fs::write(&file, fixture.to_string()).unwrap();

    let mut client = Client::start(&[file.to_str().unwrap()], true);
    let mut args = json!({"run_id":"jacket-return-trace","call_id":"jacket-answer","limit":1});
    let first = client.call("get_turn_evidence", args.clone());
    assert!(first["structuredContent"]["decision"].is_null());
    assert_eq!(
        first["structuredContent"]["rows"][0]["decision_id"],
        "answer"
    );
    args["offset"] = first["structuredContent"]["next_offset"].clone();
    let second = client.call("get_turn_evidence", args.clone());
    assert_eq!(
        second["structuredContent"]["rows"][0]["decision_id"],
        "second-answer"
    );
    args["offset"] = json!(0);
    args["decision_id"] = json!("answer");
    let evidence = client.call("get_turn_evidence", args);
    assert_eq!(
        evidence["structuredContent"]["rows"][0]["finding"],
        "conflict"
    );
}
