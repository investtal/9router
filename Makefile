.DEFAULT_GOAL := help
SHELL := /bin/bash

.PHONY: bootstrap build clean format help install-hooks lint scan-security test-integration test-unit
bootstrap:
	@if [ -f .prototools ]; then echo "==> proto install"; proto install; fi
	@if [ -d .githooks ]; then \
		echo "==> git config core.hooksPath .githooks"; \
		git config core.hooksPath .githooks; \
		chmod +x .githooks/* 2>/dev/null || true; \
	fi
	@if [ -f package.json ]; then echo "==> pnpm install"; pnpm install; fi
	@echo "==> bootstrap complete"

install-hooks:
	@if [ -d .githooks ]; then \
		git config core.hooksPath .githooks; \
		chmod +x .githooks/* 2>/dev/null || true; \
		echo "Git hooks installed (core.hooksPath = .githooks)."; \
	else \
		echo "No .githooks/ directory found." >&2; exit 1; \
	fi

help:
	@echo ""
	@echo "9router"
	@echo "======="
	@echo ""
	@echo "Setup:"
	@echo "  make bootstrap  - proto install + git hooks (.githooks) + pnpm install"
	@echo ""

lint:
	@if command -v oxlint >/dev/null 2>&1 || [ -x node_modules/.bin/oxlint ]; then pnpm exec oxlint . 2>/dev/null || npx --yes oxlint@1.76.0 .; else echo "Install oxlint (pnpm add -D oxlint) or run make bootstrap"; fi

format:
	@if command -v oxfmt >/dev/null 2>&1 || [ -x node_modules/.bin/oxfmt ]; then pnpm exec oxfmt . 2>/dev/null || npx --yes oxfmt@0.61.0 .; else echo "Install oxfmt"; fi

test-unit:
	@if [ -f package.json ] && grep -q vitest package.json; then pnpm exec vitest run --passWithNoTests; else echo "No unit tests"; fi

test-integration:
	@echo "9router: no integration tests"

build:
	@pnpm run build

clean:
	@rm -rf node_modules dist .next 2>/dev/null || true
	@echo "clean complete"

scan-security:
	@if [ -f scripts/security-gate.sh ]; then \
		bash scripts/security-gate.sh; \
	elif [ -f scripts/security-scan.sh ]; then \
		bash scripts/security-scan.sh; \
	else \
		echo "==> scan-security (best-effort: gitleaks + trivy + semgrep)"; \
		fail=0; ran=0; \
		if command -v gitleaks >/dev/null 2>&1; then ran=$$((ran+1)); gitleaks detect --redact --no-banner || fail=1; \
		else echo "  skip gitleaks"; fi; \
		if command -v trivy >/dev/null 2>&1; then ran=$$((ran+1)); trivy fs --scanners vuln --severity HIGH,CRITICAL --ignore-unfixed --exit-code 1 --quiet . || fail=1; \
		else echo "  skip trivy"; fi; \
		if command -v semgrep >/dev/null 2>&1; then ran=$$((ran+1)); semgrep ci --config p/owasp-top-ten --metrics off || fail=1; \
		else echo "  skip semgrep (make bootstrap)"; fi; \
		if [ $$ran -eq 0 ]; then echo "WARNING: no security tools found"; exit 0; fi; \
		if [ $$fail -ne 0 ]; then echo "scan-security FAILED"; exit 1; fi; \
		echo "scan-security OK"; \
	fi

