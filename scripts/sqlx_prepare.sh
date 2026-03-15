#!/usr/bin/env bash
set -euo pipefail

# Run from anywhere — resolves paths relative to repo root
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="$REPO_ROOT/infra/docker-compose.yml"
API_DIR="$REPO_ROOT/apps/api"
MIGRATIONS_DIR="$REPO_ROOT/packages/db/migrations"
DATABASE_URL="postgres://templar:templar@localhost:5432/templar"

echo "==> [sqlx-prepare] Checking postgres..."

# Start postgres if not running
if ! docker compose -f "$COMPOSE_FILE" ps postgres 2>/dev/null | grep -q "running"; then
    echo "==> [sqlx-prepare] Starting postgres container..."
    docker compose -f "$COMPOSE_FILE" up -d postgres
fi

# Wait for postgres to be healthy (up to 30s)
echo "==> [sqlx-prepare] Waiting for postgres to be ready..."
for i in $(seq 1 30); do
    if docker compose -f "$COMPOSE_FILE" exec -T postgres pg_isready -U templar -d templar > /dev/null 2>&1; then
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "ERROR: Postgres did not become ready in 30s" >&2
        exit 1
    fi
    sleep 1
done
echo "==> [sqlx-prepare] Postgres is ready."

# Apply all UP migrations in order (idempotent — errors on existing objects are silently ignored)
echo "==> [sqlx-prepare] Applying migrations..."
for migration in $(ls "$MIGRATIONS_DIR"/*.sql | grep -v '\.down\.sql$' | sort); do
    echo "    Applying $(basename "$migration")..."
    docker compose -f "$COMPOSE_FILE" exec -T postgres \
        psql -U templar -d templar < "$migration" > /dev/null 2>&1 || true
done
echo "==> [sqlx-prepare] Migrations done."

# Install cargo-sqlx if not present
if ! cargo sqlx --version > /dev/null 2>&1; then
    echo "==> [sqlx-prepare] Installing cargo-sqlx..."
    cargo install sqlx-cli --no-default-features --features postgres
fi

# Run sqlx prepare from the api crate directory
echo "==> [sqlx-prepare] Generating .sqlx query cache..."
cd "$API_DIR"
DATABASE_URL="$DATABASE_URL" cargo sqlx prepare

echo ""
echo "✓  .sqlx cache updated at apps/api/.sqlx/"
echo "   Commit these files alongside your code changes."
