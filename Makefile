.DEFAULT_GOAL := help

# Shared bootstrap (proto install + git hooks + pnpm install).
# Lives in the Landtal-level scripts/git-hooks/ folder, one level up.
include ../scripts/git-hooks/bootstrap.mk

.PHONY: help

help:
	@echo ""
	@echo "9router"
	@echo "======="
	@echo ""
	@echo "Setup:"
	@echo "  make bootstrap  - proto install + git hooks (.githooks) + pnpm install"
	@echo ""
