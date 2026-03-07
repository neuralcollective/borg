#!/bin/bash
set -e

VESPA_CONFIG_URL="${VESPA_CONFIG_URL:-http://localhost:19071}"
VESPA_APP_DIR="${VESPA_APP_DIR:-/app/vespa}"
MAX_WAIT=120

echo "Waiting for Vespa config server..."
elapsed=0
while [ $elapsed -lt $MAX_WAIT ]; do
    if curl -sf "$VESPA_CONFIG_URL/state/v1/health" | grep -q '"UP"'; then
        echo "Vespa config server ready (${elapsed}s)"
        break
    fi
    sleep 2
    elapsed=$((elapsed + 2))
done
if [ $elapsed -ge $MAX_WAIT ]; then
    echo "ERROR: Vespa config server not ready after ${MAX_WAIT}s"
    exit 1
fi

echo "Deploying application package..."
response=$(curl -sf -X POST "$VESPA_CONFIG_URL/application/v2/tenant/default/prepareandactivate" \
    -H "Content-Type: application/zip" \
    --data-binary @<(cd "$VESPA_APP_DIR" && zip -r - .))

if echo "$response" | grep -q '"prepared"'; then
    echo "Application deployed successfully"
else
    echo "WARNING: Deploy response: $response"
fi

# Wait for content cluster to be ready
echo "Waiting for content cluster..."
VESPA_QUERY_URL="${VESPA_QUERY_URL:-http://localhost:8080}"
elapsed=0
while [ $elapsed -lt 60 ]; do
    if curl -sf "$VESPA_QUERY_URL/state/v1/health" | grep -q '"UP"'; then
        echo "Content cluster ready (${elapsed}s)"
        break
    fi
    sleep 2
    elapsed=$((elapsed + 2))
done

echo "Vespa initialization complete"
