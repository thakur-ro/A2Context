IMAGE ?= nexusweave:local
PYTHON ?= python3

.PHONY: demo check check-qa check-dispatch-study check-ui image verify-image

demo:
	cargo build --locked
	$(PYTHON) examples/exchange-approval/demo.py

check: check-qa check-dispatch-study
	$(PYTHON) scripts/check_doc_links.py
	cargo fmt --all --check
	cargo clippy --locked --all-targets -- -D warnings
	cargo test --locked
	cargo build --locked
	@for file in ui/*.js; do node --check "$$file" || exit 1; done
	node --test tests/*.test.mjs
	$(PYTHON) scripts/check_sidecar.py --binary target/debug/nexusweave
	$(PYTHON) tests/sidecar_prefix_reference.py target/debug/nexusweave
	$(PYTHON) tests/sidecar_tool_masks.py target/debug/nexusweave
	$(PYTHON) tests/check_exchange_demo.py target/debug/nexusweave

check-qa:
	$(PYTHON) examples/public-qa/compare_orders.py
	$(PYTHON) examples/public-qa/check_holdout.py
	$(PYTHON) examples/public-qa/annotation_gap.py --check-saved
	$(PYTHON) examples/public-qa/support-review/run.py --check examples/public-qa/support-review/scores.json
	$(PYTHON) examples/public-qa/support-review/compare.py
	$(PYTHON) examples/public-qa/support-review/transfer/squad.py --check examples/public-qa/support-review/transfer/scores.json
	$(PYTHON) examples/public-qa/support-review/witness-edits/generate.py --check examples/public-qa/support-review/witness-edits/answers.json

check-dispatch-study:
	$(PYTHON) examples/exchange-approval/dispatch-study/compare_parsers.py --check

check-ui:
	cargo build --locked
	$(PYTHON) scripts/check_browser.py

image:
	docker build -t "$(IMAGE)" .

verify-image: image
	$(PYTHON) scripts/check_image.py --image "$(IMAGE)"
