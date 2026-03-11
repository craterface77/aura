#!/bin/sh
# Deploys AuraL1Bridge to Anvil and writes BRIDGE_CONTRACT to /app/.env.
#
# Flow:
#   1. Run forge script Deploy.s.sol --broadcast
#   2. Read the deployed address from Foundry's broadcast JSON
#   3. Write .env with PROVIDER_URL, BRIDGE_CONTRACT, STATE_DB_PATH
#
# The .env is bind-mounted from the host, so changes here are immediately
# visible to the ingestor container when it starts.

set -e

ANVIL_URL="${ANVIL_URL:-http://anvil:8545}"
ENV_FILE="/app/.env"
CONTRACTS_DIR="/app/contracts"

# Default Anvil account 0 private key (public knowledge — never use on mainnet)
DEPLOYER_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

echo "[deployer] Deploying AuraL1Bridge to Anvil at ${ANVIL_URL}..."

cd "${CONTRACTS_DIR}"

# Run the Foundry deploy script. --broadcast writes results to the broadcast dir.
forge script script/Deploy.s.sol \
    --rpc-url "${ANVIL_URL}" \
    --private-key "${DEPLOYER_KEY}" \
    --broadcast \
    2>&1

echo "[deployer] Forge script complete. Reading deployed address..."

# Foundry writes broadcast results to:
#   broadcast/Deploy.s.sol/<chain_id>/run-latest.json
CHAIN_ID=$(cast chain-id --rpc-url "${ANVIL_URL}")
BROADCAST_FILE="${CONTRACTS_DIR}/broadcast/Deploy.s.sol/${CHAIN_ID}/run-latest.json"

if [ ! -f "${BROADCAST_FILE}" ]; then
    echo "[deployer] ERROR: broadcast file not found at ${BROADCAST_FILE}"
    exit 1
fi

echo "[deployer] Reading broadcast file: ${BROADCAST_FILE}"

# Extract the first contractAddress value from the broadcast JSON.
# awk is always present in the Foundry image; handles both "key":"val" and "key": "val".
CONTRACT_ADDRESS=$(awk -F'"' '/"contractAddress"/ {for(i=1;i<=NF;i++) if($i=="contractAddress") {print $(i+2); exit}}' "${BROADCAST_FILE}")

if [ -z "${CONTRACT_ADDRESS}" ]; then
    echo "[deployer] ERROR: Could not extract contract address from broadcast JSON"
    exit 1
fi

echo "[deployer] AuraL1Bridge deployed at: ${CONTRACT_ADDRESS}"

# Write the .env file that the ingestor will read.
# Using printf for reliable cross-shell newline handling.
printf 'PROVIDER_URL=ws://anvil:8545\n' > "${ENV_FILE}"
printf 'BRIDGE_CONTRACT=%s\n' "${CONTRACT_ADDRESS}" >> "${ENV_FILE}"
printf 'RUST_LOG=info\n' >> "${ENV_FILE}"
printf 'STATE_DB_PATH=/app/data/state\n' >> "${ENV_FILE}"

echo "[deployer] .env written:"
cat "${ENV_FILE}"
echo "[deployer] Done. Ingestor can now start."
