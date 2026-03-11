# Aura L2 — Mini Sequencer

A production-grade Layer 2 execution engine built in Rust. Aura demonstrates how a mini-sequencer works: it listens to L1 deposits, maintains an account state tree, executes L2 transfers via the EVM, and allows users to withdraw back to L1 using on-chain Merkle proofs.

---

## What Problem Does It Solve?

Ethereum is expensive. Every transaction competes for limited block space, making micro-payments and high-frequency interactions impractical. Layer 2 solutions move execution off-chain while inheriting L1 security.

Aura solves this by:

- **Batching state off-chain** — accounts and balances live in a RocksDB-backed Sparse Merkle Tree
- **Anchoring state on L1** — the operator periodically posts a 32-byte state root to `AuraL1Bridge.sol`
- **Trustless withdrawals** — any user can exit to L1 by providing a Merkle inclusion proof; the contract verifies it against the posted root without trusting anyone

This is the same fundamental design used by Polygon, StarkWare, and other production L2s, implemented from scratch in ~2,000 lines of Rust.

---

## Architecture

```
L1 (Anvil / Ethereum)
    │
    │  Deposit events (WebSocket)
    ▼
┌─────────────────────────────────────────────┐
│               API Binary                    │
│                                             │
│  ┌───────────┐   ┌──────────────────────┐  │
│  │ Ingestor  │──▶│   Event Processor    │  │
│  │  (task)   │   │   apply_deposit()    │  │
│  └───────────┘   └──────────────────────┘  │
│                            │                │
│                            ▼                │
│              ┌─────────────────────────┐    │
│              │      StateEngine        │    │
│              │  Arc<RwLock<SMT>>       │    │
│              │  Arc<RocksDbBackend>    │    │
│              └─────────────────────────┘    │
│                       ▲         ▲           │
│          ┌────────────┘         └─────┐     │
│          │                           │     │
│  ┌───────────────┐        ┌────────────────┐│
│  │  REST :3000   │        │  gRPC :50051   ││
│  │  POST /tx     │        │ SubmitTx       ││
│  │  GET /account │        │ GetProof       ││
│  │  GET /proof   │        │ GetStateRoot   ││
│  └───────────────┘        └────────────────┘│
└─────────────────────────────────────────────┘
    │
    │  POST stateRoot  /  withdraw(proof)
    ▼
L1 (AuraL1Bridge.sol)
```

**Single binary, four concurrent tasks:**

| Task            | Description                                               |
| --------------- | --------------------------------------------------------- |
| Ingestor        | WebSocket subscriber, feeds `Deposit` events into channel |
| Event processor | Reads channel, calls `apply_deposit()` on StateEngine     |
| REST server     | axum HTTP API on port 3000                                |
| gRPC server     | tonic service on port 50051                               |

All tasks share one `Arc<StateEngine>`. The API process is the sole primary RocksDB writer — no LOCK conflicts.

---

## Modules

### `state/` — Sparse Merkle Tree + RocksDB

The heart of the system. Stores account balances and produces cryptographic proofs.

**Sparse Merkle Tree** (`merkle.rs`)

- Depth 32 → capacity of 2³² ≈ 4 billion accounts
- Only populated nodes are stored in memory (`HashMap<(level, index), [u8;32]>`)
- Unpopulated subtrees are implicit zero-hashes (precomputed at startup)
- Every leaf update triggers an O(32) path recomputation to the root
- Proof generation returns 32 sibling hashes, leaf-level first

**Leaf hashing** (`account.rs`)

```
leaf = keccak256(keccak256(address_bytes || balance_be32))
```

Double keccak256 follows the OpenZeppelin standard and prevents second-preimage attacks.

**RocksDB backend** (`store.rs`)

- Key: 20-byte address
- Value: `bincode`-serialized `AccountData`
- Special key `__next_leaf_index__` tracks the next free SMT slot
- Sealed trait (`StateBackend`) prevents external implementations

**StateEngine** (`engine.rs`)

- `new(backend)` — scans all RocksDB keys and rebuilds the SMT in memory
- `apply_deposit(addr, amount)` — credits balance, updates SMT, returns new root
- `apply_transfer(from, to, amount, gas)` — validates balance, deducts sender, credits recipient
- `get_proof(addr)` — returns `MerkleProof { leaf_index, leaf_value, siblings[32], root }`

---

### `ingestor/` — L1 Event Listener

Connects to Ethereum (or Anvil) via WebSocket and forwards L1 deposit events into a `tokio::mpsc` channel.

- Subscribes to `Deposit(address indexed user, uint256 amount, uint256 indexed depositId)` logs from `AuraL1Bridge`
- Subscribes to new block headers (for sequencer heartbeat)
- Decodes events with `alloy`'s `sol!` macro
- Auto-reconnects with 5-second backoff on disconnect
- Exposed as both a standalone binary and a library (used by `api/`)

---

### `executor/` — EVM Simulation and Commit

Runs transfers through `revm` before writing to state.

- **`simulate_transfer`** — read-only, creates a `CacheDB` over `StateEngine`, executes a plain ETH transfer (21,000 gas), returns outcome without mutating state
- **`commit_transfer`** — calls simulate first; on success, calls `engine.apply_transfer()` and returns the new state root
- **`StateEngineDb`** — implements revm's `DatabaseRef` trait, bridging `StateEngine` data into the EVM execution environment

Gas model: plain ETH transfers only (21,000 gas fixed). No contract execution yet.

---

### `api/` — REST and gRPC Server

Single binary that owns the RocksDB primary handle and serves both protocols.

**REST endpoints**

| Method | Path                       | Description                                                                                                |
| ------ | -------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `POST` | `/tx`                      | Submit a transfer. Body: `{ from, to, value }`. Returns `{ gas_used, new_sender_balance, new_state_root }` |
| `GET`  | `/account/{address}`       | Get balance and nonce                                                                                      |
| `GET`  | `/account/{address}/proof` | Get Merkle proof for L1 withdrawal                                                                         |
| `GET`  | `/state/root`              | Current state root                                                                                         |

**gRPC service** (`proto/aura_l2.proto`)

```protobuf
service AuraL2 {
  rpc SubmitTransaction (TransferRequest) returns (TransactionResponse);
  rpc GetAccountProof   (AccountProofRequest) returns (AccountProofResponse);
  rpc GetStateRoot      (Empty) returns (StateRootResponse);
}
```

---

### `contracts/` — L1 Bridge (Solidity)

**`AuraL1Bridge.sol`** handles three things:

1. **Deposits** — users send ETH to `deposit()`, which emits a `Deposit` event picked up by the ingestor
2. **State root anchoring** — the operator calls `updateStateRoot(bytes32)` to post the latest L2 root on-chain
3. **Withdrawals** — users call `withdraw(amount, l2Balance, leafIndex, siblings[32])`:
   - Contract recomputes the leaf: `keccak256(keccak256(address || balance_be32))`
   - Walks the 32-level tree to recompute the root
   - Verifies it matches the stored `stateRoot`
   - Checks `withdrawnAmount[stateRoot][user] + amount ≤ l2Balance` (prevents double-withdrawal)
   - Records the withdrawal, then transfers ETH (checks-effects-interactions pattern)

When a new root is posted, the per-root withdrawal counter resets to zero automatically — no expensive mapping wipe needed.

---

## Data Flow

### Deposit (L1 → L2)

```
User calls AuraL1Bridge.deposit{value: 1 ether}()
    → Deposit event emitted on L1
    → Ingestor picks up event via WebSocket
    → Event processor calls engine.apply_deposit(user, 1 ether)
    → RocksDB updated, SMT leaf updated, new root computed
```

### Transfer (L2 → L2)

```
Client POST /tx  { from: Alice, to: Bob, value: 0.1 ether }
    → executor.simulate_transfer() — revm validates balance
    → executor.commit_transfer() — engine.apply_transfer() writes to RocksDB + SMT
    → Response: { gas_used: 21000, new_sender_balance: "...", new_state_root: "0x..." }
```

### Withdrawal (L2 → L1)

```
1. Operator posts state root:
   forge script script/UpdateStateRoot.s.sol --broadcast

2. User fetches proof:
   GET /account/0xAlice/proof
   → { leaf_index, leaf_value, siblings[32], state_root }

3. User submits withdrawal on L1:
   forge script script/Withdraw.s.sol --broadcast
   → AuraL1Bridge.withdraw(amount, l2Balance, leafIndex, siblings)
   → Contract verifies Merkle proof → transfers ETH to user
```

---

## Getting Started

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) + Docker Compose
- [Foundry](https://book.getfoundry.sh/getting-started/installation) (for manual contract interaction)
- Rust 1.80+ (for running tests locally)

### One-Command Start

```bash
docker-compose up --build
```

This will:

1. Start **Anvil** (local Ethereum node) on port `8545`
2. Run **deployer** — compiles and deploys `AuraL1Bridge.sol`, writes the contract address to `.env`
3. Start **API** — opens RocksDB, spawns the ingestor, serves REST on `:3000` and gRPC on `:50051`

### Verify It Works

```bash
# Check state root (should be non-zero once a deposit arrives)
curl http://localhost:3000/state/root

# Check an account
curl http://localhost:3000/account/0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266

# Submit a transfer
curl -X POST http://localhost:3000/tx \
  -H 'Content-Type: application/json' \
  -d '{"from":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","to":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8","value":"100000000000000000"}'

# Get Merkle proof for withdrawal
curl http://localhost:3000/account/0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266/proof
```

### Trigger a Deposit

```bash
cd contracts
forge script script/Deposit.s.sol \
  --rpc-url http://localhost:8545 \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --broadcast
```

### Post State Root to L1

```bash
cd contracts
STATE_ROOT=<0x...from /state/root> \
forge script script/UpdateStateRoot.s.sol \
  --rpc-url http://localhost:8545 \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --broadcast
```

### Withdraw from L1

```bash
cd contracts
BRIDGE_CONTRACT=<address> \
WITHDRAW_AMOUNT=<wei> \
L2_BALANCE=<wei> \
LEAF_INDEX=<n> \
SIBLINGS=<32 comma-separated 0x hashes> \
PRIVATE_KEY=0xac0974... \
forge script script/Withdraw.s.sol \
  --rpc-url http://localhost:8545 \
  --broadcast
```

---

## Running Tests

```bash
# All workspace tests
cargo test --workspace

# State crate only (SMT + RocksDB)
cargo test -p aura-l2-state

# Executor crate (revm simulation)
cargo test -p executor
```

Expected output: **25+ tests passing**, 0 failures.

---

## Configuration

All configuration is read from `.env` (auto-generated by the deployer, or set manually):

| Variable          | Default             | Description                                  |
| ----------------- | ------------------- | -------------------------------------------- |
| `PROVIDER_URL`    | `ws://anvil:8545`   | WebSocket endpoint for L1                    |
| `BRIDGE_CONTRACT` | _(set by deployer)_ | `AuraL1Bridge` address                       |
| `STATE_DB_PATH`   | `/app/data/state`   | RocksDB directory                            |
| `RUST_LOG`        | `info`              | Log level (`debug`, `info`, `warn`, `error`) |
| `GRPC_PORT`       | `50051`             | gRPC server port                             |
| `REST_PORT`       | `3000`              | REST server port                             |

---

## Project Structure

```
aura/
├── api/                    # REST + gRPC server binary
│   ├── proto/aura_l2.proto # Protobuf service definition
│   └── src/
│       ├── main.rs         # Startup: RocksDB, ingestor task, servers
│       ├── app_state.rs    # Shared AppState (Arc<StateEngine>)
│       ├── grpc/           # tonic service implementation
│       └── rest/           # axum handlers and router
├── contracts/              # Foundry workspace
│   ├── src/AuraL1Bridge.sol
│   └── script/             # Deploy, Deposit, UpdateStateRoot, Withdraw
├── docker/
│   ├── Dockerfile.deployer
│   └── deploy.sh
├── executor/               # revm EVM simulation
│   └── src/lib.rs
├── ingestor/               # L1 WebSocket event listener
│   └── src/
│       ├── lib.rs          # Ingestor struct (used by api/)
│       └── main.rs         # Standalone binary
├── state/                  # Core state: SMT + RocksDB
│   └── src/
│       ├── account.rs      # AccountData, MerkleProof, newtypes
│       ├── engine.rs       # StateEngine<S>
│       ├── error.rs        # StateError
│       ├── merkle.rs       # SparseMerkleTree (depth=32)
│       └── store.rs        # StateBackend sealed trait + RocksDbBackend
├── Dockerfile              # Multi-stage cargo-chef build
├── docker-compose.yml      # anvil → deployer → api
└── .env                    # Runtime configuration
```

---

## Security Notes

- **Double keccak256 leaf hashing** — protects against second-preimage attacks (OpenZeppelin standard)
- **Per-root withdrawal tracking** — `mapping(stateRoot => mapping(address => withdrawn))` prevents double-spending without an explicit reset
- **Checks-effects-interactions** — withdrawal counter updated before ETH transfer
- **Operator key** — the account that posts state roots; in production this would be a multisig or ZK verifier

---

## Future Roadmap

### Sequencer Loop

Accumulate transactions in a mempool (`tokio::sync::mpsc`) and apply them in batches every 500ms, then auto-post the new state root to L1. Currently every `POST /tx` commits immediately — a sequencer loop would make this a true L2 sequencer.

### ZK Proof Generation

Replace the trusted operator model with a mathematical guarantee. Integrate [SP1](https://github.com/succinctlabs/sp1) or [RISC Zero](https://risczero.com/) to generate a ZK proof for each batch of state transitions. `updateStateRoot` on L1 would then verify a proof instead of trusting the operator's signature — turning Aura from an optimistic into a **ZK-rollup**.

### Fraud Proof Window

Add a 7-day challenge period to `updateStateRoot`. During this window, anyone can submit a fraud proof to dispute an invalid root. This is the alternative to ZK — the approach used by Optimism and Arbitrum.

### Data Availability

The state root alone is not enough for trustless exits — users need the raw data to reconstruct their balance proofs. Post batch calldata to L1 (expensive) or integrate a dedicated DA layer like [Celestia](https://celestia.org/) or EigenDA (cheap). Without DA, users cannot exit if the operator disappears.

### Forced Exits

Add a `forceWithdraw` path to `AuraL1Bridge.sol` that allows users to exit directly via L1 even if the sequencer is offline or censoring their transactions.

### Multi-token Support

Extend the leaf schema to `keccak256(keccak256(address || token || balance))` to support ERC-20 tokens alongside native ETH.

### Transaction History (PostgreSQL)

Add a cold analytics path: persist transaction history to PostgreSQL via `sqlx` for block explorer queries. RocksDB remains the hot state store; Postgres handles `SELECT` history.
