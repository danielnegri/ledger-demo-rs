// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Copyright (C) 2025 Daniel Negri
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Benchmarks for the ledger engine.
//!
//! Run with: cargo bench
//!
//! Benchmarks include:
//! - Single-threaded transaction processing
//! - Multi-threaded concurrent transaction processing
//! - Dispute lifecycle operations
//! - Scaling with number of clients

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use ledger_demo_rs::{ClientId, Engine, TransactionId, TransactionType};
use rayon::prelude::*;
use rust_decimal::Decimal;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

// =============================================================================
// Helper Functions
// =============================================================================

fn make_deposit(client_id: u16, tx_id: u32, amount: i64) -> TransactionType {
    TransactionType::Deposit {
        client_id: ClientId(client_id),
        transaction_id: TransactionId(tx_id),
        amount: Decimal::new(amount, 4),
    }
}

fn make_withdrawal(client_id: u16, tx_id: u32, amount: i64) -> TransactionType {
    TransactionType::Withdrawal {
        client_id: ClientId(client_id),
        transaction_id: TransactionId(tx_id),
        amount: Decimal::new(amount, 4),
    }
}

fn make_dispute(client_id: u16, tx_id: u32) -> TransactionType {
    TransactionType::Dispute {
        client_id: ClientId(client_id),
        transaction_id: TransactionId(tx_id),
    }
}

fn make_resolve(client_id: u16, tx_id: u32) -> TransactionType {
    TransactionType::Resolve {
        client_id: ClientId(client_id),
        transaction_id: TransactionId(tx_id),
    }
}

fn make_chargeback(client_id: u16, tx_id: u32) -> TransactionType {
    TransactionType::Chargeback {
        client_id: ClientId(client_id),
        transaction_id: TransactionId(tx_id),
    }
}

// =============================================================================
// Single-Threaded Benchmarks
// =============================================================================

fn bench_single_deposit(c: &mut Criterion) {
    c.bench_function("single_deposit", |b| {
        let mut tx_id = 0u32;
        b.iter(|| {
            let engine = Engine::new();
            let tx = make_deposit(1, tx_id, 10000);
            tx_id += 1;
            engine.process(black_box(tx)).unwrap();
        })
    });
}

fn bench_single_withdrawal(c: &mut Criterion) {
    c.bench_function("single_withdrawal", |b| {
        let mut tx_id = 0u32;
        b.iter(|| {
            let engine = Engine::new();
            // Deposit first
            let deposit = make_deposit(1, tx_id, 10000);
            tx_id += 1;
            engine.process(deposit).unwrap();
            // Then withdraw
            let withdrawal = make_withdrawal(1, tx_id, 5000);
            tx_id += 1;
            engine.process(black_box(withdrawal)).unwrap();
        })
    });
}

fn bench_deposit_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("deposit_throughput");

    for count in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let engine = Engine::new();
                for i in 0..count {
                    let tx = make_deposit(1, i as u32, 10000);
                    engine.process(tx).unwrap();
                }
                black_box(&engine);
            })
        });
    }
    group.finish();
}

fn bench_mixed_transactions(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_transactions");

    for count in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let engine = Engine::new();
                let mut tx_id = 0u32;

                for _ in 0..count {
                    // Deposit
                    let deposit = make_deposit(1, tx_id, 10000);
                    tx_id += 1;
                    engine.process(deposit).unwrap();

                    // Withdraw half
                    let withdrawal = make_withdrawal(1, tx_id, 5000);
                    tx_id += 1;
                    let _ = engine.process(withdrawal);
                }
                black_box(&engine);
            })
        });
    }
    group.finish();
}

// =============================================================================
// Dispute Lifecycle Benchmarks
// =============================================================================

fn bench_dispute_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispute_lifecycle");

    // Benchmark dispute only
    group.bench_function("dispute", |b| {
        let mut tx_id = 0u32;
        b.iter(|| {
            let engine = Engine::new();
            let deposit = make_deposit(1, tx_id, 10000);
            engine.process(deposit).unwrap();
            let dispute = make_dispute(1, tx_id);
            tx_id += 1;
            engine.process(black_box(dispute)).unwrap();
        })
    });

    // Benchmark dispute + resolve
    group.bench_function("dispute_resolve", |b| {
        let mut tx_id = 0u32;
        b.iter(|| {
            let engine = Engine::new();
            let deposit = make_deposit(1, tx_id, 10000);
            engine.process(deposit).unwrap();
            let dispute = make_dispute(1, tx_id);
            engine.process(dispute).unwrap();
            let resolve = make_resolve(1, tx_id);
            tx_id += 1;
            engine.process(black_box(resolve)).unwrap();
        })
    });

    // Benchmark dispute + chargeback
    group.bench_function("dispute_chargeback", |b| {
        let mut tx_id = 0u32;
        b.iter(|| {
            let engine = Engine::new();
            let deposit = make_deposit(1, tx_id, 10000);
            engine.process(deposit).unwrap();
            let dispute = make_dispute(1, tx_id);
            engine.process(dispute).unwrap();
            let chargeback = make_chargeback(1, tx_id);
            tx_id += 1;
            engine.process(black_box(chargeback)).unwrap();
        })
    });

    group.finish();
}

// =============================================================================
// Multi-Client Benchmarks
// =============================================================================

fn bench_multi_client_sequential(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_client_sequential");

    for num_clients in [10, 100, 1_000].iter() {
        let tx_per_client = 100;
        let total_tx = *num_clients as u64 * tx_per_client;

        group.throughput(Throughput::Elements(total_tx));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_clients),
            num_clients,
            |b, &num_clients| {
                b.iter(|| {
                    let engine = Engine::new();
                    let mut tx_id = 0u32;

                    for client in 0..num_clients {
                        for _ in 0..tx_per_client {
                            let deposit = make_deposit(client as u16, tx_id, 10000);
                            tx_id += 1;
                            engine.process(deposit).unwrap();
                        }
                    }
                    black_box(&engine);
                })
            },
        );
    }
    group.finish();
}

// =============================================================================
// Multi-Threaded Benchmarks
// =============================================================================

fn bench_parallel_deposits_same_client(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_deposits_same_client");

    for count in [1_000, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let engine = Arc::new(Engine::new());
                let tx_counter = AtomicU32::new(0);

                (0..count).into_par_iter().for_each(|_| {
                    let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                    let deposit = make_deposit(1, tx_id, 10000);
                    let _ = engine.process(deposit);
                });

                black_box(&engine);
            })
        });
    }
    group.finish();
}

fn bench_parallel_deposits_different_clients(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_deposits_different_clients");

    for count in [1_000, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let engine = Arc::new(Engine::new());
                let tx_counter = AtomicU32::new(0);

                (0..count).into_par_iter().for_each(|i| {
                    let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                    // Each iteration uses a different client (wrapping at u16::MAX)
                    let client_id = (i % 65535) as u16 + 1;
                    let deposit = make_deposit(client_id, tx_id, 10000);
                    engine.process(deposit).unwrap();
                });

                black_box(&engine);
            })
        });
    }
    group.finish();
}

fn bench_parallel_mixed_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_mixed_operations");

    for num_clients in [10, 100, 1_000].iter() {
        let ops_per_client = 100;
        let total_ops = *num_clients as u64 * ops_per_client * 2; // deposit + withdrawal

        group.throughput(Throughput::Elements(total_ops));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_clients),
            num_clients,
            |b, &num_clients| {
                b.iter(|| {
                    let engine = Arc::new(Engine::new());
                    let tx_counter = AtomicU32::new(0);

                    // Phase 1: Parallel deposits for all clients
                    (0..num_clients).into_par_iter().for_each(|client| {
                        for _ in 0..ops_per_client {
                            let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                            let deposit = make_deposit(client as u16 + 1, tx_id, 10000);
                            engine.process(deposit).unwrap();
                        }
                    });

                    // Phase 2: Parallel withdrawals for all clients
                    (0..num_clients).into_par_iter().for_each(|client| {
                        for _ in 0..ops_per_client {
                            let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                            let withdrawal = make_withdrawal(client as u16 + 1, tx_id, 5000);
                            let _ = engine.process(withdrawal);
                        }
                    });

                    black_box(&engine);
                })
            },
        );
    }
    group.finish();
}

fn bench_parallel_disputes(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_disputes");

    for num_clients in [10, 100, 1_000].iter() {
        group.throughput(Throughput::Elements(*num_clients as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_clients),
            num_clients,
            |b, &num_clients| {
                b.iter_batched(
                    || {
                        // Setup: Create engine with one deposit per client
                        let engine = Engine::new();
                        for client in 0..num_clients {
                            let deposit = make_deposit(client as u16 + 1, client as u32, 10000);
                            engine.process(deposit).unwrap();
                        }
                        Arc::new(engine)
                    },
                    |engine| {
                        // Benchmark: Parallel disputes
                        (0..num_clients).into_par_iter().for_each(|client| {
                            let dispute = make_dispute(client as u16 + 1, client as u32);
                            engine.process(dispute).unwrap();
                        });
                        black_box(&engine);
                    },
                    criterion::BatchSize::SmallInput,
                )
            },
        );
    }
    group.finish();
}

// =============================================================================
// Scaling Benchmarks
// =============================================================================

fn bench_thread_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("thread_scaling");
    let total_transactions = 100_000u32;

    for num_threads in [1, 2, 4, 8].iter() {
        group.throughput(Throughput::Elements(total_transactions as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_threads),
            num_threads,
            |b, &num_threads| {
                // Configure rayon thread pool for this benchmark
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(num_threads)
                    .build()
                    .unwrap();

                b.iter(|| {
                    let engine = Arc::new(Engine::new());
                    let tx_counter = AtomicU32::new(0);

                    pool.install(|| {
                        (0..total_transactions).into_par_iter().for_each(|i| {
                            let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                            // Distribute across 1000 clients
                            let client_id = (i % 1000) as u16 + 1;
                            let deposit = make_deposit(client_id, tx_id, 10000);
                            engine.process(deposit).unwrap();
                        });
                    });

                    black_box(&engine);
                })
            },
        );
    }
    group.finish();
}

fn bench_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("contention");
    let total_ops = 10_000u32;

    // Benchmark with varying number of clients to measure contention effects
    // Fewer clients = more contention (more threads competing for same locks)
    for num_clients in [1, 10, 100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(total_ops as u64));
        group.bench_with_input(
            BenchmarkId::new("clients", num_clients),
            num_clients,
            |b, &num_clients| {
                b.iter(|| {
                    let engine = Arc::new(Engine::new());
                    let tx_counter = AtomicU32::new(0);

                    (0..total_ops).into_par_iter().for_each(|i| {
                        let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                        let client_id = (i % num_clients as u32) as u16 + 1;
                        let deposit = make_deposit(client_id, tx_id, 10000);
                        engine.process(deposit).unwrap();
                    });

                    black_box(&engine);
                })
            },
        );
    }
    group.finish();
}

// =============================================================================
// Memory/Allocation Benchmarks
// =============================================================================

fn bench_account_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("account_creation");

    for count in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let engine = Engine::new();
                for i in 0..count {
                    // Each deposit creates a new account
                    let deposit = make_deposit(i as u16 + 1, i as u32, 10000);
                    engine.process(deposit).unwrap();
                }
                black_box(&engine);
            })
        });
    }
    group.finish();
}

fn bench_transaction_history(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_history");

    // Benchmark how performance changes as transaction history grows
    for history_size in [100, 1_000, 10_000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(history_size),
            history_size,
            |b, &history_size| {
                b.iter_batched(
                    || {
                        // Setup: Create engine with existing transaction history
                        let engine = Engine::new();
                        for i in 0..history_size {
                            let deposit = make_deposit(1, i as u32, 10000);
                            engine.process(deposit).unwrap();
                        }
                        (engine, history_size as u32)
                    },
                    |(engine, next_tx_id)| {
                        // Benchmark: Add one more transaction
                        let deposit = make_deposit(1, next_tx_id, 10000);
                        engine.process(black_box(deposit)).unwrap();
                    },
                    criterion::BatchSize::SmallInput,
                )
            },
        );
    }
    group.finish();
}

// =============================================================================
// Criterion Groups
// =============================================================================

criterion_group!(
    single_threaded,
    bench_single_deposit,
    bench_single_withdrawal,
    bench_deposit_throughput,
    bench_mixed_transactions,
);

criterion_group!(disputes, bench_dispute_lifecycle,);

criterion_group!(multi_client, bench_multi_client_sequential,);

criterion_group!(
    multi_threaded,
    bench_parallel_deposits_same_client,
    bench_parallel_deposits_different_clients,
    bench_parallel_mixed_operations,
    bench_parallel_disputes,
);

criterion_group!(scaling, bench_thread_scaling, bench_contention,);

criterion_group!(memory, bench_account_creation, bench_transaction_history,);

criterion_main!(
    single_threaded,
    disputes,
    multi_client,
    multi_threaded,
    scaling,
    memory
);
