use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

const BIN: &str = env!("CARGO_BIN_EXE_a2context");
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
                    "clientInfo": {"name": "a2context-test", "version": "1"},
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
fn pending_mcp_can_inspect_and_validate_without_a_trace_store() {
    let mut client = Client::start(&[], true);
    let listed = client.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["inspect_pending", "validate_shadow_case"]
    );
    assert!(tools
        .iter()
        .all(|tool| tool["annotations"]["readOnlyHint"] == true));
    assert_eq!(
        tools[1]["inputSchema"]["properties"]["case"]["properties"]["interventions"]["type"],
        "array"
    );
    assert_eq!(client.call("get_turn_evidence", json!({}))["isError"], true);

    let request: Value =
        serde_json::from_str(include_str!("../examples/preflight-returns.json")).unwrap();
    let mut args = json!({"input": {"request": request}, "limit": 1});
    let inspection = client.call("inspect_pending", args.clone());
    let view = &inspection["structuredContent"];
    assert_eq!(view["kind"], "pending_evidence");
    assert_eq!(view["preflight"]["finding_count"], 1);
    assert!(view["preflight"]["receipt"]["fingerprint"].is_string());
    assert_eq!(view["rows"][0]["kind"], "preflight_finding");
    assert!(view["rows"][0]["inputs"][0]["rule_value"].is_null());
    assert_eq!(view["next_offset"], 1);
    args["offset"] = view["next_offset"].clone();
    let sources = client.call("inspect_pending", args.clone());
    assert_eq!(
        sources["structuredContent"]["rows"][0]["kind"],
        "captured_input"
    );
    assert!(sources["structuredContent"]["rows"][0]["input"]["excerpt"].is_null());
    assert!(!sources.to_string().contains("Can I return"));
    args["include_excerpts"] = json!(true);
    assert_eq!(client.call("inspect_pending", args)["isError"], true);

    let request: Value =
        serde_json::from_str(include_str!("../fixtures/routing-request.json")).unwrap();
    let case: Value =
        serde_json::from_str(include_str!("../fixtures/routing-shadow.json")).unwrap();
    let mut plan = json!({"request":request,"case":case});
    let checked = client.call("validate_shadow_case", plan.clone());
    assert_eq!(checked["structuredContent"]["valid"], true);
    plan["case"]["match_rule"]["pointers"] = json!(["/absent"]);
    assert_eq!(client.call("validate_shadow_case", plan)["isError"], true);
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
fn legacy_client_gets_the_cli_counts_and_cannot_widen_host_scope() {
    let dir = tempfile::tempdir().unwrap();
    let scope = dir.path().join("scope.json");
    let restriction = json!({"kind":"user","organization_id":"demo-retail","user_id":"user-042"});
    std::fs::write(&scope, restriction.to_string()).unwrap();
    let evidence = "fixtures/support.json";
    let mut client = Client::start(&["--scope", scope.to_str().unwrap(), evidence], false);
    let listed = client.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6);
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
