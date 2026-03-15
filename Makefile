.PHONY: sqlx-prepare docker-build help

## Generate sqlx offline query cache (run after adding new sqlx::query! macros)
sqlx-prepare:
	@bash scripts/sqlx_prepare.sh

## Build Docker images (ensures sqlx cache is fresh first)
docker-build: sqlx-prepare
	docker compose -f infra/docker-compose.yml build

## Show available targets
help:
	@grep -E '^[a-zA-Z_-]+:.*?##' Makefile | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

.DEFAULT_GOAL := help
