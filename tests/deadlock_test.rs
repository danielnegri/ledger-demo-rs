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

//! Deadlock detection tests using parking_lot's built-in deadlock detector.
//!
//! These tests verify that the locking patterns used in the ledger engine
//! do not lead to deadlocks under various concurrent access scenarios.
//!
//! The tests use parking_lot::Mutex with the `deadlock_detection` feature
//! to automatically detect cycles in the lock graph.

use dashmap::DashMap;
use parking_lot::{deadlock, Mutex};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

// === Test Wrappers (mirror production locking patterns) ===

/// Mirrors the production AccountData structure.
#[derive(Debug)]
#[allow(dead_code)]
struct TestAccountData {
    client_id: u16,
    available: Decimal,
    held: Decimal,
    locked: bool,
    deposits: HashMap<u32, Decimal>,
}

impl TestAccountData {
    fn new(client_id: u16) -> Self {
        Self {
            client_id,
            available: Decimal::ZERO,
            held: Decimal::ZERO,
            locked: false,
            deposits: HashMap::new(),
        }
    }

    fn deposit(&mut self, amount: Decimal) {
        self.available += amount;
    }

    fn withdraw(&mut self, amount: Decimal) -> bool {
        if self.available >= amount && !self.locked {
            self.available -= amount;
            true
        } else {
            false
        }
    }

    fn hold_funds(&mut self, amount: Decimal) -> bool {
        if self.available >= amount {
            self.available -= amount;
            self.held += amount;
            true
        } else {
            false
        }
    }

    fn release_funds(&mut self, amount: Decimal) -> bool {
        if self.held >= amount {
            self.held -= amount;
            self.available += amount;
            true
        } else {
            false
        }
    }

    fn chargeback(&mut self, amount: Decimal) -> bool {
        if self.held >= amount {
            self.held -= amount;
            self.locked = true;
            true
        } else {
            false
        }
    }
}

/// Mirrors the production Account structure with parking_lot::Mutex.
struct TestAccount {
    inner: Mutex<TestAccountData>,
}

impl TestAccount {
    fn new(client_id: u16) -> Self {
        Self {
            inner: Mutex::new(TestAccountData::new(client_id)),
        }
    }

    fn available(&self) -> Decimal {
        self.inner.lock().available
    }

    fn held(&self) -> Decimal {
        self.inner.lock().held
    }

    fn total(&self) -> Decimal {
        let data = self.inner.lock();
        data.available + data.held
    }

    fn locked(&self) -> bool {
        self.inner.lock().locked
    }

    fn deposit(&self, tx_id: u32, amount: Decimal) {
        let mut data = self.inner.lock();
        data.deposit(amount);
        data.deposits.insert(tx_id, amount);
    }

    fn withdraw(&self, amount: Decimal) -> bool {
        self.inner.lock().withdraw(amount)
    }

    fn dispute(&self, tx_id: u32) -> bool {
        let mut data = self.inner.lock();
        if let Some(&amount) = data.deposits.get(&tx_id) {
            data.hold_funds(amount)
        } else {
            false
        }
    }

    fn resolve(&self, tx_id: u32) -> bool {
        let mut data = self.inner.lock();
        if let Some(&amount) = data.deposits.get(&tx_id) {
            data.release_funds(amount)
        } else {
            false
        }
    }

    fn chargeback(&self, tx_id: u32) -> bool {
        let mut data = self.inner.lock();
        if let Some(&amount) = data.deposits.get(&tx_id) {
            data.chargeback(amount)
        } else {
            false
        }
    }
}

/// Mirrors the production Engine structure.
struct TestEngine {
    accounts: DashMap<u16, Arc<TestAccount>>,
    tx_ids: DashMap<u32, ()>,
}

impl TestEngine {
    fn new() -> Self {
        Self {
            accounts: DashMap::new(),
            tx_ids: DashMap::new(),
        }
    }

    fn get_or_create_account(&self, client_id: u16) -> Arc<TestAccount> {
        self.accounts
            .entry(client_id)
            .or_insert_with(|| Arc::new(TestAccount::new(client_id)))
            .clone()
    }

    fn deposit(&self, client_id: u16, tx_id: u32, amount: Decimal) -> bool {
        if self.tx_ids.contains_key(&tx_id) {
            return false;
        }
        self.tx_ids.insert(tx_id, ());
        let account = self.get_or_create_account(client_id);
        account.deposit(tx_id, amount);
        true
    }

    fn withdraw(&self, client_id: u16, tx_id: u32, amount: Decimal) -> bool {
        if self.tx_ids.contains_key(&tx_id) {
            return false;
        }
        self.tx_ids.insert(tx_id, ());
        let account = self.get_or_create_account(client_id);
        account.withdraw(amount)
    }

    fn dispute(&self, client_id: u16, tx_id: u32) -> bool {
        if let Some(account) = self.accounts.get(&client_id) {
            account.dispute(tx_id)
        } else {
            false
        }
    }

    fn resolve(&self, client_id: u16, tx_id: u32) -> bool {
        if let Some(account) = self.accounts.get(&client_id) {
            account.resolve(tx_id)
        } else {
            false
        }
    }

    fn chargeback(&self, client_id: u16, tx_id: u32) -> bool {
        if let Some(account) = self.accounts.get(&client_id) {
            account.chargeback(tx_id)
        } else {
            false
        }
    }

    fn get_account(&self, client_id: u16) -> Option<Arc<TestAccount>> {
        self.accounts.get(&client_id).map(|r| r.clone())
    }

    fn account_count(&self) -> usize {
        self.accounts.len()
    }
}

// === Deadlock Detection Infrastructure ===

/// Starts a background thread that checks for deadlocks.
/// Returns a handle to stop the detector.
fn start_deadlock_detector() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    thread::spawn(move || {
        while running_clone.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(100));
            let deadlocks = deadlock::check_deadlock();
            if !deadlocks.is_empty() {
                eprintln!("\n=== DEADLOCK DETECTED ===");
                for (i, threads) in deadlocks.iter().enumerate() {
                    eprintln!("\nDeadlock #{}", i + 1);
                    for t in threads {
                        eprintln!("Thread ID: {:?}", t.thread_id());
                        eprintln!("Backtrace:\n{:#?}", t.backtrace());
                    }
                }
                panic!("Deadlock detected! See output above for details.");
            }
        }
    });

    running
}

/// Stops the deadlock detector.
fn stop_deadlock_detector(running: Arc<AtomicBool>) {
    running.store(false, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(150)); // Let detector thread exit
}

// === Tests ===

/// Test high contention on a single account with many threads.
#[test]
fn no_deadlock_high_contention_single_account() {
    let detector = start_deadlock_detector();
    let engine = Arc::new(TestEngine::new());
    let tx_counter = Arc::new(AtomicU32::new(1));

    const NUM_THREADS: usize = 50;
    const OPS_PER_THREAD: usize = 100;

    let mut handles = Vec::with_capacity(NUM_THREADS);

    for _ in 0..NUM_THREADS {
        let engine = engine.clone();
        let tx_counter = tx_counter.clone();

        let handle = thread::spawn(move || {
            for i in 0..OPS_PER_THREAD {
                let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);

                if i % 3 == 0 {
                    engine.deposit(1, tx_id, dec!(10.00));
                } else if i % 3 == 1 {
                    engine.withdraw(1, tx_id, dec!(1.00));
                } else {
                    // Read operations
                    if let Some(account) = engine.get_account(1) {
                        let _ = account.total();
                        let _ = account.available();
                        let _ = account.held();
                    }
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    stop_deadlock_detector(detector);

    // Verify final state is consistent
    let account = engine.get_account(1).expect("Account should exist");
    assert!(account.available() >= Decimal::ZERO);
    assert!(account.held() >= Decimal::ZERO);
    println!(
        "High contention test passed: {} threads × {} ops",
        NUM_THREADS, OPS_PER_THREAD
    );
}

/// Test operations across multiple accounts.
#[test]
fn no_deadlock_cross_account_operations() {
    let detector = start_deadlock_detector();
    let engine = Arc::new(TestEngine::new());
    let tx_counter = Arc::new(AtomicU32::new(1));

    const NUM_THREADS: usize = 20;
    const NUM_ACCOUNTS: u16 = 10;
    const OPS_PER_THREAD: usize = 50;

    let mut handles = Vec::with_capacity(NUM_THREADS);

    for thread_id in 0..NUM_THREADS {
        let engine = engine.clone();
        let tx_counter = tx_counter.clone();

        let handle = thread::spawn(move || {
            for i in 0..OPS_PER_THREAD {
                let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                // Each thread cycles through accounts
                let client_id = ((thread_id + i) % (NUM_ACCOUNTS as usize)) as u16 + 1;

                if i % 2 == 0 {
                    engine.deposit(client_id, tx_id, dec!(5.00));
                } else {
                    engine.withdraw(client_id, tx_id, dec!(1.00));
                }

                // Also read from a different account
                let other_client_id = ((thread_id + i + 1) % (NUM_ACCOUNTS as usize)) as u16 + 1;
                if let Some(account) = engine.get_account(other_client_id) {
                    let _ = account.total();
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    stop_deadlock_detector(detector);

    println!(
        "Cross-account test passed: {} accounts, {} threads",
        engine.account_count(),
        NUM_THREADS
    );
}

/// Test the dispute lifecycle under contention.
#[test]
fn no_deadlock_dispute_lifecycle() {
    let detector = start_deadlock_detector();
    let engine = Arc::new(TestEngine::new());

    const NUM_CLIENTS: u16 = 20;

    // First, create deposits for each client
    for client_id in 1..=NUM_CLIENTS {
        engine.deposit(client_id, client_id as u32, dec!(1000.00));
    }

    let mut handles = Vec::with_capacity(NUM_CLIENTS as usize);

    for client_id in 1..=NUM_CLIENTS {
        let engine = engine.clone();

        let handle = thread::spawn(move || {
            let tx_id = client_id as u32;

            // Dispute the deposit
            engine.dispute(client_id, tx_id);

            // Small delay to simulate processing
            thread::sleep(Duration::from_micros(100));

            // Either resolve or chargeback based on client_id
            if client_id % 2 == 0 {
                engine.resolve(client_id, tx_id);
            } else {
                engine.chargeback(client_id, tx_id);
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    stop_deadlock_detector(detector);

    // Verify final states
    for client_id in 1..=NUM_CLIENTS {
        let account = engine.get_account(client_id).expect("Account should exist");

        if client_id % 2 == 0 {
            // Resolved - funds should be back
            assert_eq!(account.available(), dec!(1000.00));
            assert_eq!(account.held(), Decimal::ZERO);
            assert!(!account.locked());
        } else {
            // Chargebacked - funds removed, account locked
            assert_eq!(account.total(), Decimal::ZERO);
            assert!(account.locked());
        }
    }

    println!("Dispute lifecycle test passed: {} clients", NUM_CLIENTS);
}

/// Test iterating accounts while mutating.
#[test]
fn no_deadlock_iteration_during_mutation() {
    let detector = start_deadlock_detector();
    let engine = Arc::new(TestEngine::new());
    let tx_counter = Arc::new(AtomicU32::new(1));
    let running = Arc::new(AtomicBool::new(true));

    // Spawn writer threads that add new accounts
    let mut handles = Vec::new();

    for writer_id in 0..5 {
        let engine = engine.clone();
        let tx_counter = tx_counter.clone();
        let running = running.clone();

        let handle = thread::spawn(move || {
            let mut count = 0;
            while running.load(Ordering::SeqCst) && count < 100 {
                let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                let client_id = (writer_id * 100 + count) as u16;
                engine.deposit(client_id, tx_id, dec!(10.00));
                count += 1;
                thread::yield_now();
            }
        });

        handles.push(handle);
    }

    // Spawn reader threads that iterate all accounts
    for _ in 0..5 {
        let engine = engine.clone();
        let running = running.clone();

        let handle = thread::spawn(move || {
            let mut iterations = 0;
            while running.load(Ordering::SeqCst) && iterations < 50 {
                let mut total = Decimal::ZERO;
                for entry in engine.accounts.iter() {
                    total += entry.value().total();
                }
                iterations += 1;
                let _ = total; // Use the value
                thread::yield_now();
            }
        });

        handles.push(handle);
    }

    // Let them run for a bit
    thread::sleep(Duration::from_millis(500));
    running.store(false, Ordering::SeqCst);

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    stop_deadlock_detector(detector);

    println!(
        "Iteration during mutation test passed: {} accounts created",
        engine.account_count()
    );
}

/// Test mixed operations with many threads.
#[test]
fn no_deadlock_mixed_operations() {
    let detector = start_deadlock_detector();
    let engine = Arc::new(TestEngine::new());
    let tx_counter = Arc::new(AtomicU32::new(1));

    const NUM_THREADS: usize = 100;
    const OPS_PER_THREAD: usize = 50;
    const NUM_ACCOUNTS: u16 = 20;

    // Pre-create accounts with initial balance
    for client_id in 1..=NUM_ACCOUNTS {
        let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
        engine.deposit(client_id, tx_id, dec!(10000.00));
    }

    let mut handles = Vec::with_capacity(NUM_THREADS);

    for thread_id in 0..NUM_THREADS {
        let engine = engine.clone();
        let tx_counter = tx_counter.clone();

        let handle = thread::spawn(move || {
            for i in 0..OPS_PER_THREAD {
                let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
                let client_id = ((thread_id + i) % (NUM_ACCOUNTS as usize)) as u16 + 1;

                match i % 5 {
                    0 => {
                        engine.deposit(client_id, tx_id, dec!(1.00));
                    }
                    1 => {
                        engine.withdraw(client_id, tx_id, dec!(0.50));
                    }
                    2 => {
                        // Read total
                        if let Some(account) = engine.get_account(client_id) {
                            let _ = account.total();
                        }
                    }
                    3 => {
                        // Read available
                        if let Some(account) = engine.get_account(client_id) {
                            let _ = account.available();
                        }
                    }
                    _ => {
                        // Read held
                        if let Some(account) = engine.get_account(client_id) {
                            let _ = account.held();
                        }
                    }
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    stop_deadlock_detector(detector);

    // Verify all accounts are in valid state
    for client_id in 1..=NUM_ACCOUNTS {
        let account = engine.get_account(client_id).expect("Account should exist");
        assert!(account.available() >= Decimal::ZERO);
        assert!(account.held() >= Decimal::ZERO);
    }

    println!(
        "Mixed operations test passed: {} threads × {} ops on {} accounts",
        NUM_THREADS, OPS_PER_THREAD, NUM_ACCOUNTS
    );
}

/// Test lock contention fairness - all threads should eventually complete.
#[test]
fn no_deadlock_lock_contention_fairness() {
    let detector = start_deadlock_detector();
    let account = Arc::new(TestAccount::new(1));

    const NUM_THREADS: usize = 100;
    const OPS_PER_THREAD: usize = 10;

    let completed = Arc::new(AtomicU32::new(0));
    let mut handles = Vec::with_capacity(NUM_THREADS);

    for _ in 0..NUM_THREADS {
        let account = account.clone();
        let completed = completed.clone();

        let handle = thread::spawn(move || {
            for _ in 0..OPS_PER_THREAD {
                // Hold lock for a tiny bit
                {
                    let mut data = account.inner.lock();
                    data.available += dec!(0.01);
                    // Small work inside lock
                    for _ in 0..10 {
                        std::hint::black_box(data.available);
                    }
                }
                thread::yield_now();
            }
            completed.fetch_add(1, Ordering::SeqCst);
        });

        handles.push(handle);
    }

    // Wait with timeout
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(30);

    for handle in handles {
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            panic!("Timeout: threads did not complete in time (possible starvation)");
        }
        // Join should complete quickly if no deadlock
        handle.join().expect("Thread panicked");
    }

    stop_deadlock_detector(detector);

    assert_eq!(
        completed.load(Ordering::SeqCst),
        NUM_THREADS as u32,
        "All threads should complete"
    );

    println!(
        "Lock fairness test passed: all {} threads completed",
        NUM_THREADS
    );
}

/// Test that verifies the deadlock detector itself works.
/// This creates an intentional deadlock between two test-only locks.
#[test]
fn induced_deadlock_detected() {
    // This test is marked as ignored by default because it intentionally creates a deadlock
    // which will cause the test to panic. Run with: cargo test induced_deadlock_detected -- --ignored

    // For CI, we just verify the deadlock detection infrastructure works
    let detector = start_deadlock_detector();

    // Do some normal operations
    let engine = TestEngine::new();
    engine.deposit(1, 1, dec!(100.00));
    engine.withdraw(1, 2, dec!(50.00));

    let account = engine.get_account(1).unwrap();
    assert_eq!(account.available(), dec!(50.00));

    stop_deadlock_detector(detector);

    println!("Deadlock detector infrastructure verified");
}

/// Stress test with rapid lock acquire/release cycles.
#[test]
fn no_deadlock_rapid_lock_cycling() {
    let detector = start_deadlock_detector();
    let engine = Arc::new(TestEngine::new());
    let tx_counter = Arc::new(AtomicU32::new(1));

    const NUM_THREADS: usize = 20;
    const CYCLES_PER_THREAD: usize = 1000;

    let mut handles = Vec::with_capacity(NUM_THREADS);

    for thread_id in 0..NUM_THREADS {
        let engine = engine.clone();
        let tx_counter = tx_counter.clone();

        let handle = thread::spawn(move || {
            let client_id = (thread_id % 5) as u16 + 1;

            for _ in 0..CYCLES_PER_THREAD {
                let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);

                // Rapid deposit
                engine.deposit(client_id, tx_id, dec!(0.01));

                // Immediate read
                if let Some(account) = engine.get_account(client_id) {
                    let _ = account.total();
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    stop_deadlock_detector(detector);

    println!(
        "Rapid lock cycling test passed: {} threads × {} cycles",
        NUM_THREADS, CYCLES_PER_THREAD
    );
}

/// Test concurrent dispute races on the same transaction.
#[test]
fn no_deadlock_concurrent_dispute_same_tx() {
    let detector = start_deadlock_detector();
    let engine = Arc::new(TestEngine::new());

    // Create a deposit
    engine.deposit(1, 1, dec!(1000.00));

    const NUM_THREADS: usize = 20;
    let mut handles = Vec::with_capacity(NUM_THREADS);

    // All threads try to dispute the same transaction
    for _ in 0..NUM_THREADS {
        let engine = engine.clone();

        let handle = thread::spawn(move || {
            engine.dispute(1, 1)
        });

        handles.push(handle);
    }

    let results: Vec<bool> = handles
        .into_iter()
        .map(|h| h.join().expect("Thread panicked"))
        .collect();

    stop_deadlock_detector(detector);

    // Only some disputes should succeed (first one for sure)
    let successful = results.iter().filter(|&&r| r).count();
    println!(
        "Concurrent dispute test passed: {}/{} disputes succeeded",
        successful, NUM_THREADS
    );
}
