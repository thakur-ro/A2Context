IMAGE ?= nexusweave:local
PYTHON ?= python3
SDK_OPTION = $(if $(SDK_PYTHON),--sdk-python "$(SDK_PYTHON)")

.PHONY: demo check check-sidecar check-bundles check-release check-qa check-dispatch-study check-qwen-agent check-ui image verify-image

demo:
	cargo run --locked -- demo

check: check-qa check-dispatch-study check-sidecar
	$(PYTHON) scripts/check_doc_links.py
	cargo fmt --all --check
	cargo clippy --locked --all-targets -- -D warnings
	cargo test --locked
	@for file in ui/*.js; do node --check "$$file" || exit 1; done
	node --test tests/*.test.mjs

check-sidecar:
	cargo build --locked
	$(PYTHON) scripts/check_sidecar.py --binary target/debug/nexusweave $(SDK_OPTION)
	$(PYTHON) tests/sidecar_targets.py target/debug/nexusweave $(SDK_OPTION)
	$(PYTHON) tests/sidecar_conflict_coverage.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_prefix_reference.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_tool_masks.py target/debug/nexusweave
	$(PYTHON) tests/check_exchange_demo.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_baseline.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_source_gaps.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_prefix_experiment.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_prefix_cohort.py target/debug/nexusweave $(SDK_OPTION)
	$(PYTHON) tests/sidecar_opening_study.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_position.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_prefill.py target/debug/nexusweave $(SDK_OPTION)

check-bundles:
	npm ci --prefix ui/mcp-app --no-audit --no-fund
	npm run check --prefix ui/mcp-app
	npm ci --prefix ui/a2ui --no-audit --no-fund
	npm run check --prefix ui/a2ui

check-release: SDK_PYTHON ?= $(PYTHON)
check-release: check check-ui check-bundles verify-image
	RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
	cargo package --locked

check-qa:
	$(PYTHON) examples/public-qa/compare_orders.py
	$(PYTHON) examples/public-qa/check_holdout.py
	$(PYTHON) examples/public-qa/annotation_gap.py --check-saved
	$(PYTHON) examples/public-qa/support-review/run.py --check examples/public-qa/support-review/scores.json
	$(PYTHON) examples/public-qa/support-review/compare.py
	$(PYTHON) examples/public-qa/support-review/transfer/squad.py --check examples/public-qa/support-review/transfer/scores.json
	$(PYTHON) examples/public-qa/support-review/witness-edits/generate.py --check examples/public-qa/support-review/witness-edits/answers.json

check-dispatch-study:
	$(PYTHON) examples/context-screening/replay/prefix-study/study.py
	$(PYTHON) examples/context-screening/replay/prefix-transfer/study.py
	$(PYTHON) examples/context-screening/replay/prefix-indirect/study.py
	$(PYTHON) examples/exchange-approval/dispatch-study/compare_parsers.py --check
	$(PYTHON) examples/airline-transfer/run.py --check examples/airline-transfer/results.json
	$(PYTHON) examples/airline-transfer/baseline_budget.py --check examples/airline-transfer/baseline-budget.json

check-qwen-agent:
	cargo build --locked
	$(PYTHON) examples/exchange-approval/dispatch-study/framework/replay.py --check
	$(PYTHON) examples/exchange-approval/dispatch-study/framework/retry.py --check
	$(PYTHON) examples/exchange-approval/dispatch-study/prospective/run.py --check examples/exchange-approval/dispatch-study/prospective/results.json
	$(PYTHON) examples/exchange-approval/dispatch-study/rendering/measure.py --check examples/exchange-approval/dispatch-study/rendering/results.json
	$(PYTHON) examples/exchange-approval/dispatch-study/prefill-tool/measure.py --check examples/exchange-approval/dispatch-study/prefill-tool/results.json
	$(PYTHON) examples/exchange-approval/dispatch-study/lookup-field/run_lookup.py --check examples/exchange-approval/dispatch-study/lookup-field/results.json
	$(PYTHON) examples/exchange-approval/dispatch-study/framework/check_sidecar_export.py

check-ui:
	cargo build --locked
	$(PYTHON) scripts/check_browser.py

image:
	docker build -t "$(IMAGE)" .

verify-image: image
	$(PYTHON) scripts/check_image.py --image "$(IMAGE)"
