.PHONY: fmt lint test ci build web web-test web-e2e nvim-test e2e-setup e2e teardown helm docker release-check

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace --locked

ci: fmt lint test

build:
	cargo build --workspace --locked

web:
	npm --prefix webui ci && npm --prefix webui run build

web-test:
	npm --prefix webui ci && npm --prefix webui test -- --run

web-e2e:
	npm --prefix webui run test:e2e

nvim-test:
	./hack/nvim-test.sh

e2e-setup:
	./hack/setup-kind-e2e.sh

e2e:
	cargo run -p knect-e2e --locked -- --all

teardown:
	./hack/teardown-kind-e2e.sh

helm:
	helm lint charts/knectd && helm template knectd charts/knectd

docker:
	docker build -t knectd:dev .

release-check:
	actionlint .github/workflows/release.yml
