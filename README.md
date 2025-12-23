# Ledger Demo

A payment processing engine that handles transactions like:

- **Deposits** - Credit funds to a client account
- **Withdrawals** - Debit funds from a client account
- **Disputes** - Hold funds from a previous deposit pending investigation
- **Resolves** - Release held funds back to available balance
- **Chargebacks** - Remove held funds and lock the account

The engine supports concurrent transaction processing:

- Different clients can be processed in parallel
- Same client operations are serialized (via per-account)
- Transaction deduplication is thread-safe

## Quick Start

```bash
# Build the project
cargo build --release

# Run with a CSV file
cargo run --release -- transactions.csv > accounts.csv

# Run tests
cargo test

# Run benchmarks
cargo bench
```

## Usage

```bash
ledger-demo-rs <input.csv>
```

The program reads transactions from a CSV file and outputs account states to stdout.

### Input Format

```csv
type,client,tx,amount
deposit,1,1,100.0
deposit,2,2,50.0
withdrawal,1,3,25.0
dispute,1,1,
resolve,1,1,
```

| Column   | Description                                           |
|----------|-------------------------------------------------------|
| `type`   | Transaction type: deposit, withdrawal, dispute, resolve, chargeback |
| `client` | Client ID (u16: 0-65535)                              |
| `tx`     | Transaction ID (u32: 0-4294967295)                    |
| `amount` | Decimal amount (required for deposit/withdrawal, ignored for others) |

### Output Format

```csv
client,available,held,total,locked
1,75.0,0.0,75.0,false
2,50.0,0.0,50.0,false
```

| Column      | Description                           |
|-------------|---------------------------------------|
| `client`    | Client ID                             |
| `available` | Funds available for withdrawal        |
| `held`      | Funds held due to disputes            |
| `total`     | available + held                      |
| `locked`    | Account frozen after chargeback       |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Engine                              │
│  ┌─────────────────┐    ┌─────────────────────────────────┐ │
│  │ TransactionQueue│    │         DashMap<ClientId,       │ │
│  │  (deduplication)│    │              Account>           │ │
│  └─────────────────┘    └─────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                                    │
                    ┌───────────────┼───────────────┐
                    ▼               ▼               ▼
              ┌─────────┐     ┌─────────┐     ┌─────────┐
              │Account 1│     │Account 2│     │Account N│              
              └─────────┘     └─────────┘     └─────────┘
```

### Transaction State Machine

Deposits follow a state machine for dispute handling:

```
Deposit (Applied) ──dispute──► Deposit (Inflight) ──resolve───► Deposit (Resolved)
                                        │
                                        └──chargeback──► Deposit (Voided)
                                                         + Account Locked
```

### Invariants

- `available >= 0` 
- `held >= 0` 
- `total = available + held`
- Transaction IDs are globally unique
- Only deposits can be disputed (not withdrawals)
- A chargeback locks the account

## Error Handling

The engine silently skips invalid transactions per the specification:

| Error | Behavior |
|-------|----------|
| Malformed CSV rows | Skipped |
| Unknown transaction type | Skipped |
| Duplicate transaction ID | Skipped |
| Insufficient funds | Skipped |
| Dispute on non-existent tx | Skipped |
| Operations on locked account | Skipped |

In debug builds, skipped transactions are logged to stderr.

## Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture

# Run property tests only
cargo test --test proptest
```

## Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark group
cargo bench -- "parallel"

# Quick validation (no measurements)
cargo bench -- --test
```

### Benchmark Groups

| Group | Description |
|-------|-------------|
| `single_threaded` | Single transaction latency and throughput |
| `disputes` | Dispute lifecycle operations |
| `multi_client` | Sequential multi-client processing |
| `multi_threaded` | Parallel processing with rayon |
| `scaling` | Thread scaling (1-8 threads) |
| `contention` | Lock contention analysis |
| `memory` | Account creation and history growth |

## Future Work

### Performance Optimizations

- **Memory-mapped CSV parsing** - Use `memmap2` for zero-copy parsing of large CSV files, reducing memory allocation overhead
- **Parallel CSV processing** - Leverage `rayon` for multi-threaded transaction parsing
- **RwLock for read-heavy workloads** - Replace `Mutex` with `RwLock` for balance queries to allow concurrent reads
- **History compaction** - Archive resolved/voided disputes to reduce per-account memory usage

### Persistence & Durability

- **Durable storage backend** - Integrate with an embedded database for crash recovery and restart capability
- **Write-ahead logging (WAL)** - Persist transactions before applying to ensure atomicity
- **Snapshots & recovery** - Support point-in-time recovery and incremental backups

### Observability

- **Structured logging** - Integrate `tracing` crate for request-scoped logging and debugging
- **Metrics export** - Prometheus metrics for throughput, latency percentiles, dispute rates, and chargeback rates
- **Health checks** - Engine consistency verification endpoints

### Features

- **Streaming output** - Stream CSV output to avoid buffering entire dataset in memory
- **Pagination** - Cursor-based pagination for large account sets
- **Transaction history API** - Query transaction history per account
- **Rate limiting** - Backpressure handling for transaction bursts

### Compliance & Auditability

- **Immutable audit log** - Append-only transaction log with cryptographic integrity
- **Dispute expiration** - Auto-resolve disputes after configurable time period (e.g., 90 days)

## Disclaimer

This project was developed with the assistance of AI-powered tools:

- **Coding autocomplete (Github Co-Pilot)**
- **Documentation**
- **Unit testing**

All generated code has been reviewed for correctness, security, and adherence to best practices. The final implementation decisions and code quality remain the responsibility of the project maintainers.

## License

AGPL-3.0-or-later
