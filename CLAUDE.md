# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

This is **rusty-kaspa**, the Rust implementation of the Kaspa full-node and its ancillary libraries. Kaspa is a high-throughput cryptocurrency with a blockDAG (Directed Acyclic Graph) architecture designed for scalability and speed.

## Core Architecture

### Project Structure
- **kaspad**: Main node daemon implementation
- **consensus**: Core consensus engine with GHOSTDAG protocol implementation  
- **mining**: Mining and proof-of-work components
- **rpc**: RPC service layer with gRPC and wRPC implementations
- **wallet**: Multi-platform wallet implementation (native, WASM, CLI)
- **simpa**: Network simulator for testing and benchmarking
- **protocol**: P2P networking and flow control
- **crypto**: Cryptographic primitives (hashes, transactions, addresses)

### Key Architectural Patterns
- **Processor Pipeline**: Header → Body → Virtual → Pruning processors for block validation
- **Service Layer**: ConsensusServices manages reachability, difficulty, and other core services
- **Store Pattern**: Trait-based storage abstraction with RocksDB backend
- **Notification System**: Event-driven architecture with ConsensusNotificationRoot
- **Session Management**: SessionLock/SessionReadGuard for concurrent access control

## Common Development Tasks

### Building
```bash
# Build main node
cargo build --release --bin kaspad

# Build with UTXO index (required for wallets)
cargo run --release --bin kaspad -- --utxoindex

# Build simulator
cargo build --release --bin simpa

# Build CLI wallet
cd cli && cargo build --release
```

### Testing
```bash
# Run all tests
cargo test --release

# Run with nextest (if installed)
cargo nextest run --release

# Run benchmarks
cargo bench

# Run specific test module
cargo test --release -p kaspa-consensus
```

### Code Quality
```bash
# Format and check all code
./check

# Individual checks
cargo fmt --all
cargo clippy --workspace --tests --benches
```

### WASM Building
```bash
# Build WASM SDK (from wasm/ directory)
./build-release  # Full release
./build-web      # Web target only
./build-nodejs   # Node.js target only
```

### Running
```bash
# Mainnet node
cargo run --release --bin kaspad -- --utxoindex

# Testnet node  
cargo run --release --bin kaspad -- --testnet --utxoindex

# Simulator (1000 blocks, 8 BPS, 200 tx/block)
cargo run --release --bin simpa -- -t=200 -b=8 -n=1000

# CLI wallet
cd cli && cargo run --release
```

## Development Guidelines

### Critical Patterns
- **Always use Arc for shared ownership** between processors and services
- **Store traits** must have Reader/Writer variants for concurrent access
- **ProcessingCounters** track metrics across the processing pipeline
- **BlockProcessResult** wraps validation outcomes with proper error handling
- **SessionLock** must be acquired for any pruning operations

### Performance Considerations
- **File Descriptor Limits**: Node requires min 10,000 FDs (check with `ulimit -n`)
- **Parallel Processing**: Virtual processor uses rayon for transaction verification
- **Caching**: TxScriptCache for script validation, GhostdagDataCache for DAG queries
- **Memory Management**: Uses kaspa-alloc with configurable RAM scaling

### Consensus Rules
- **GHOSTDAG K parameter**: Calculated based on network BPS (blocks per second)
- **Pruning**: Maintains pruning_depth blocks, with pruning_proof_m samples
- **Difficulty Adjustment**: Window-based using BlockWindowManager
- **Blue Score**: GHOSTDAG blue set size determines chain selection

### Testing Approach
- **Integration tests** in testing/integration for cross-component validation
- **Consensus tests** validate fork resolution and reorganization
- **Simpa** for network-wide behavior and performance benchmarking
- **Property-based testing** for critical algorithms (see rv usage)

## Important Notes

- **Minimum Rust version**: 1.82.0 (specified in workspace)
- **Database**: RocksDB for persistent storage, with temp DB option for testing
- **Network Protocol**: Binary protocol with protobuf definitions
- **Cryptography**: secp256k1 for signatures, Blake2b for hashing
- **WASM Compatibility**: Core libraries support wasm32-unknown-unknown target