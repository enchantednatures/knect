.PHONY: fmt lint test ci web nvim-test e2e helm release-check

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace --locked

ci: fmt lint test

web:
	npm --prefix webui ci && npm --prefix webui run build

nvim-test:
	./hack/nvim-test.sh

e2e:
	cargo run -p knect-e2e --locked -- --all

helm:
	helm lint charts/knectd && helm template knectd charts/knectd

release-check:
	actionlint .github/workflows/release.yml
