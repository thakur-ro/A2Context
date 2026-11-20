IMAGE ?= nexusweave:local
PYTHON ?= python3

.PHONY: demo check image verify-image

demo:
	cargo run --locked -- sidecar --demo

check:
	$(PYTHON) scripts/check_doc_links.py
	cargo fmt --all --check
	cargo clippy --locked --all-targets -- -D warnings
	cargo test --locked
	cargo build --locked
	@for file in ui/*.js; do node --check "$$file" || exit 1; done
	node --test tests/*.test.mjs
	$(PYTHON) scripts/check_sidecar.py --binary target/debug/nexusweave
	$(PYTHON) tests/sidecar_prefix_reference.py target/debug/nexusweave

image:
	docker build -t "$(IMAGE)" .

verify-image: image
	$(PYTHON) scripts/check_image.py --image "$(IMAGE)"
