IMAGE ?= a2context:local
PYTHON ?= python3
SDK_OPTION = $(if $(SDK_PYTHON),--sdk-python "$(SDK_PYTHON)")

.PHONY: demo check check-sidecar check-bundles check-release check-ui image verify-image

demo:
	cargo run --locked -- demo

check: check-sidecar
	$(PYTHON) scripts/check_doc_links.py
	cargo fmt --all --check
	cargo clippy --locked --all-targets -- -D warnings
	cargo test --locked
	@for file in ui/*.js; do node --check "$$file" || exit 1; done
	node --test tests/*.test.mjs

check-sidecar:
	cargo build --locked
	$(PYTHON) scripts/check_sidecar.py --binary target/debug/a2context $(SDK_OPTION)
	$(PYTHON) tests/check_exchange_demo.py target/debug/a2context
	$(PYTHON) tests/sidecar_tool_reference_score.py target/debug/a2context
	$(PYTHON) tests/sidecar_context_claims.py target/debug/a2context
	$(PYTHON) tests/sidecar_shared_identifiers.py target/debug/a2context
	$(PYTHON) tests/sidecar_source_gaps.py target/debug/a2context
	$(PYTHON) tests/sidecar_source_history.py target/debug/a2context

check-bundles:
	npm ci --prefix ui/mcp-app --no-audit --no-fund
	npm run check --prefix ui/mcp-app
	npm ci --prefix ui/a2ui --no-audit --no-fund
	npm run check --prefix ui/a2ui

check-release: SDK_PYTHON ?= $(PYTHON)
check-release: check check-ui check-bundles verify-image
	RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
	cargo package --locked

check-ui:
	cargo build --locked
	$(PYTHON) scripts/check_browser.py

image:
	docker build -t "$(IMAGE)" .

verify-image: image
	$(PYTHON) scripts/check_image.py --image "$(IMAGE)"
