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

//! Account public API integration tests.

use ledger_demo_rs::{Account, ClientId, TransactionError, TransactionStatus, TransactionType};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::sync::{Arc, Mutex};
use std::thread;

// === Helper Functions ===

fn make_deposit(client_id: u16, tx_id: u32, amount: Decimal) -> TransactionType {
    TransactionType::Deposit {
        client_id: ClientId(client_id),
        transaction_id: ledger_demo_rs::TransactionId(tx_id),
        amount,
        status: TransactionStatus::Applied,
    }
}

fn make_withdrawal(client_id: u16, tx_id: u32, amount: Decimal) -> TransactionType {
    TransactionType::Withdrawal {
        client_id: ClientId(client_id),
        transaction_id: ledger_demo_rs::TransactionId(tx_id),
        amount,
    }
}

fn make_dispute(client_id: u16, tx_id: u32) -> TransactionType {
    TransactionType::Dispute {
        client_id: ClientId(client_id),
        transaction_id: ledger_demo_rs::TransactionId(tx_id),
    }
}

fn make_resolve(client_id: u16, tx_id: u32) -> TransactionType {
    TransactionType::Resolve {
        client_id: ClientId(client_id),
        transaction_id: ledger_demo_rs::TransactionId(tx_id),
    }
}

fn make_chargeback(client_id: u16, tx_id: u32) -> TransactionType {
    TransactionType::Chargeback {
        client_id: ClientId(client_id),
        transaction_id: ledger_demo_rs::TransactionId(tx_id),
    }
}

// === Basic Account Tests ===

#[test]
fn new_account_has_zero_balances() {
    let account = Account::new(ClientId(1));
    assert_eq!(account.available(), Decimal::ZERO);
    assert_eq!(account.held(), Decimal::ZERO);
    assert_eq!(account.total(), Decimal::ZERO);
    assert!(!account.locked());
}

#[test]
fn deposit_increases_available_balance() {
    let mut account = Account::new(ClientId(1));
    let tx = make_deposit(1, 100, dec!(50.00));
    account.add_transaction(tx).unwrap();
    assert_eq!(account.available(), dec!(50.00));
    assert_eq!(account.total(), dec!(50.00));
}

#[test]
fn multiple_deposits_accumulate() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account
        .add_transaction(make_deposit(1, 2, dec!(50.00)))
        .unwrap();
    account
        .add_transaction(make_deposit(1, 3, dec!(25.50)))
        .unwrap();
    assert_eq!(account.available(), dec!(175.50));
}

#[test]
fn withdrawal_decreases_available_balance() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account
        .add_transaction(make_withdrawal(1, 2, dec!(30.00)))
        .unwrap();
    assert_eq!(account.available(), dec!(70.00));
}

#[test]
fn total_equals_available_plus_held() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();

    // Dispute moves funds from available to held
    account.add_transaction(make_dispute(1, 1)).unwrap();

    // Total should remain the same
    assert_eq!(account.total(), dec!(100.00));
    assert_eq!(account.available(), dec!(0.00));
    assert_eq!(account.held(), dec!(100.00));
}

// === Error Cases ===

#[test]
fn deposit_zero_returns_invalid_amount() {
    let mut account = Account::new(ClientId(1));
    let tx = make_deposit(1, 1, Decimal::ZERO);
    let result = account.add_transaction(tx);
    assert_eq!(result, Err(TransactionError::InvalidAmount));
}

#[test]
fn deposit_negative_returns_invalid_amount() {
    let mut account = Account::new(ClientId(1));
    let tx = make_deposit(1, 1, dec!(-10.00));
    let result = account.add_transaction(tx);
    assert_eq!(result, Err(TransactionError::InvalidAmount));
}

#[test]
fn withdrawal_more_than_available_returns_insufficient_funds() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(50.00)))
        .unwrap();
    let result = account.add_transaction(make_withdrawal(1, 2, dec!(100.00)));
    assert_eq!(result, Err(TransactionError::InsufficientFunds));
    // Balance unchanged
    assert_eq!(account.available(), dec!(50.00));
}

#[test]
fn withdrawal_zero_returns_invalid_amount() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    let result = account.add_transaction(make_withdrawal(1, 2, Decimal::ZERO));
    assert_eq!(result, Err(TransactionError::InvalidAmount));
}

#[test]
fn client_mismatch_returns_error() {
    let mut account = Account::new(ClientId(1));
    let tx = make_deposit(2, 1, dec!(50.00)); // Different client_id
    let result = account.add_transaction(tx);
    assert_eq!(result, Err(TransactionError::ClientMismatch));
}

// === Edge Cases ===

#[test]
fn withdraw_exact_balance_succeeds() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account
        .add_transaction(make_withdrawal(1, 2, dec!(100.00)))
        .unwrap();
    assert_eq!(account.available(), Decimal::ZERO);
}

#[test]
fn small_decimal_precision() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(0.0001)))
        .unwrap();
    account
        .add_transaction(make_deposit(1, 2, dec!(0.0002)))
        .unwrap();
    assert_eq!(account.available(), dec!(0.0003));
}

#[test]
fn large_amounts() {
    let mut account = Account::new(ClientId(1));
    let large = dec!(999999999999.9999);
    account.add_transaction(make_deposit(1, 1, large)).unwrap();
    assert_eq!(account.available(), large);
}

// === Dispute State Machine Tests ===

#[test]
fn dispute_moves_funds_to_held() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account.add_transaction(make_dispute(1, 1)).unwrap();

    assert_eq!(account.available(), dec!(0.00));
    assert_eq!(account.held(), dec!(100.00));
    assert_eq!(account.total(), dec!(100.00));
    assert!(!account.locked());
}

#[test]
fn resolve_releases_held_funds() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account.add_transaction(make_dispute(1, 1)).unwrap();
    account.add_transaction(make_resolve(1, 1)).unwrap();

    assert_eq!(account.available(), dec!(100.00));
    assert_eq!(account.held(), dec!(0.00));
    assert_eq!(account.total(), dec!(100.00));
    assert!(!account.locked());
}

#[test]
fn chargeback_removes_funds_and_locks() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account.add_transaction(make_dispute(1, 1)).unwrap();
    account.add_transaction(make_chargeback(1, 1)).unwrap();

    assert_eq!(account.available(), dec!(0.00));
    assert_eq!(account.held(), dec!(0.00));
    assert_eq!(account.total(), dec!(0.00));
    assert!(account.locked());
}

#[test]
fn dispute_nonexistent_tx_returns_not_found() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();

    // Try to dispute a transaction that doesn't exist
    let result = account.add_transaction(make_dispute(1, 999));
    assert_eq!(result, Err(TransactionError::TransactionNotFound));
}

#[test]
fn dispute_already_disputed_returns_error() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account.add_transaction(make_dispute(1, 1)).unwrap();

    // Try to dispute again
    let result = account.add_transaction(make_dispute(1, 1));
    assert_eq!(result, Err(TransactionError::AlreadyDisputed));
}

#[test]
fn resolve_not_disputed_returns_error() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();

    // Try to resolve without dispute
    let result = account.add_transaction(make_resolve(1, 1));
    assert_eq!(result, Err(TransactionError::NotDisputed));
}

#[test]
fn chargeback_not_disputed_returns_error() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();

    // Try to chargeback without dispute
    let result = account.add_transaction(make_chargeback(1, 1));
    assert_eq!(result, Err(TransactionError::NotDisputed));
}

#[test]
fn resolve_after_resolve_returns_error() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account.add_transaction(make_dispute(1, 1)).unwrap();
    account.add_transaction(make_resolve(1, 1)).unwrap();

    // Try to resolve again (status is now Resolved, not Inflight)
    let result = account.add_transaction(make_resolve(1, 1));
    assert_eq!(result, Err(TransactionError::NotDisputed));
}

#[test]
fn chargeback_after_resolve_returns_error() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account.add_transaction(make_dispute(1, 1)).unwrap();
    account.add_transaction(make_resolve(1, 1)).unwrap();

    // Try to chargeback after resolve
    let result = account.add_transaction(make_chargeback(1, 1));
    assert_eq!(result, Err(TransactionError::NotDisputed));
}

#[test]
fn dispute_after_chargeback_returns_error() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account.add_transaction(make_dispute(1, 1)).unwrap();
    account.add_transaction(make_chargeback(1, 1)).unwrap();

    // Try to dispute again (status is now Voided)
    let result = account.add_transaction(make_dispute(1, 1));
    assert_eq!(result, Err(TransactionError::AlreadyDisputed));
}

#[test]
fn partial_dispute_preserves_other_funds() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(50.00)))
        .unwrap();
    account
        .add_transaction(make_deposit(1, 2, dec!(100.00)))
        .unwrap();

    // Dispute only the first deposit
    account.add_transaction(make_dispute(1, 1)).unwrap();

    assert_eq!(account.available(), dec!(100.00)); // Second deposit still available
    assert_eq!(account.held(), dec!(50.00)); // First deposit held
    assert_eq!(account.total(), dec!(150.00));
}

#[test]
fn multiple_disputes_independent() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(50.00)))
        .unwrap();
    account
        .add_transaction(make_deposit(1, 2, dec!(100.00)))
        .unwrap();

    // Dispute both deposits
    account.add_transaction(make_dispute(1, 1)).unwrap();
    account.add_transaction(make_dispute(1, 2)).unwrap();

    assert_eq!(account.available(), dec!(0.00));
    assert_eq!(account.held(), dec!(150.00));

    // Resolve only the first
    account.add_transaction(make_resolve(1, 1)).unwrap();

    assert_eq!(account.available(), dec!(50.00));
    assert_eq!(account.held(), dec!(100.00));
}

#[test]
fn withdrawal_cannot_be_disputed() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(100.00)))
        .unwrap();
    account
        .add_transaction(make_withdrawal(1, 2, dec!(30.00)))
        .unwrap();

    // Try to dispute the withdrawal (should fail - withdrawals not tracked in deposits map)
    let result = account.add_transaction(make_dispute(1, 2));
    assert_eq!(result, Err(TransactionError::TransactionNotFound));
}

#[test]
fn chargeback_with_remaining_balance() {
    let mut account = Account::new(ClientId(1));
    account
        .add_transaction(make_deposit(1, 1, dec!(50.00)))
        .unwrap();
    account
        .add_transaction(make_deposit(1, 2, dec!(100.00)))
        .unwrap();

    // Dispute and chargeback only the first deposit
    account.add_transaction(make_dispute(1, 1)).unwrap();
    account.add_transaction(make_chargeback(1, 1)).unwrap();

    // Second deposit should still be there, but account is locked
    assert_eq!(account.available(), dec!(100.00));
    assert_eq!(account.held(), dec!(0.00));
    assert_eq!(account.total(), dec!(100.00));
    assert!(account.locked());
}

// === Multi-threading Tests ===

#[test]
fn concurrent_deposits_are_atomic() {
    let account = Arc::new(Mutex::new(Account::new(ClientId(1))));
    let mut handles = vec![];

    for i in 0..100u32 {
        let acc = Arc::clone(&account);
        handles.push(thread::spawn(move || {
            let mut account = acc.lock().unwrap();
            let tx = make_deposit(1, i, dec!(1.00));
            let _ = account.add_transaction(tx);
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let account = account.lock().unwrap();
    assert_eq!(account.available(), dec!(100.00));
}

#[test]
fn concurrent_mixed_operations_maintain_invariants() {
    let account = Arc::new(Mutex::new(Account::new(ClientId(1))));

    // Initial deposit
    {
        let mut acc = account.lock().unwrap();
        acc.add_transaction(make_deposit(1, 0, dec!(1000.00)))
            .unwrap();
    }

    let mut handles = vec![];

    // 50 deposits of 10.00
    for i in 1..=50u32 {
        let acc = Arc::clone(&account);
        handles.push(thread::spawn(move || {
            let mut account = acc.lock().unwrap();
            let tx = make_deposit(1, i, dec!(10.00));
            let _ = account.add_transaction(tx);
        }));
    }

    // 50 withdrawals of 10.00
    for i in 51..=100u32 {
        let acc = Arc::clone(&account);
        handles.push(thread::spawn(move || {
            let mut account = acc.lock().unwrap();
            let tx = make_withdrawal(1, i, dec!(10.00));
            let _ = account.add_transaction(tx);
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Net effect: 1000 + 500 - 500 = 1000
    let account = account.lock().unwrap();
    assert_eq!(account.available(), dec!(1000.00));
}

#[test]
fn stress_test_many_transactions() {
    let account = Arc::new(Mutex::new(Account::new(ClientId(1))));
    let num_threads = 10;
    let ops_per_thread = 100;

    // Initial balance
    {
        let mut acc = account.lock().unwrap();
        acc.add_transaction(make_deposit(1, 0, dec!(10000.00)))
            .unwrap();
    }

    let mut handles = vec![];

    for t in 0..num_threads {
        let acc = Arc::clone(&account);
        handles.push(thread::spawn(move || {
            for i in 0..ops_per_thread {
                let mut account = acc.lock().unwrap();
                let tx_id = (t * ops_per_thread + i + 1) as u32;
                if i % 2 == 0 {
                    let _ = account.add_transaction(make_deposit(1, tx_id, dec!(1.00)));
                } else {
                    let _ = account.add_transaction(make_withdrawal(1, tx_id, dec!(1.00)));
                }
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Should end up with initial balance (equal deposits and withdrawals)
    let account = account.lock().unwrap();
    assert_eq!(account.available(), dec!(10000.00));
}

// === Race Condition Tests ===

#[test]
fn no_double_spend_race_condition() {
    // Test that concurrent withdrawals don't cause double-spending
    for _ in 0..10 {
        let account = Arc::new(Mutex::new(Account::new(ClientId(1))));

        // Deposit exactly 100
        {
            let mut acc = account.lock().unwrap();
            acc.add_transaction(make_deposit(1, 0, dec!(100.00)))
                .unwrap();
        }

        let mut handles = vec![];
        let successful_withdrawals = Arc::new(Mutex::new(0u32));

        // Try 10 concurrent withdrawals of 100 each
        for i in 1..=10u32 {
            let acc = Arc::clone(&account);
            let counter = Arc::clone(&successful_withdrawals);
            handles.push(thread::spawn(move || {
                let mut account = acc.lock().unwrap();
                let tx = make_withdrawal(1, i, dec!(100.00));
                if account.add_transaction(tx).is_ok() {
                    let mut count = counter.lock().unwrap();
                    *count += 1;
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Only ONE withdrawal should succeed
        let count = *successful_withdrawals.lock().unwrap();
        assert_eq!(
            count, 1,
            "Expected exactly 1 successful withdrawal, got {}",
            count
        );

        // Balance should be zero
        let account = account.lock().unwrap();
        assert_eq!(account.available(), Decimal::ZERO);
    }
}

#[test]
fn balance_never_goes_negative() {
    for _ in 0..10 {
        let account = Arc::new(Mutex::new(Account::new(ClientId(1))));

        {
            let mut acc = account.lock().unwrap();
            acc.add_transaction(make_deposit(1, 0, dec!(50.00)))
                .unwrap();
        }

        let mut handles = vec![];

        // Many concurrent withdrawals trying to overdraw
        for i in 1..=20u32 {
            let acc = Arc::clone(&account);
            handles.push(thread::spawn(move || {
                let mut account = acc.lock().unwrap();
                let tx = make_withdrawal(1, i, dec!(10.00));
                let _ = account.add_transaction(tx);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let account = account.lock().unwrap();
        assert!(
            account.available() >= Decimal::ZERO,
            "Balance went negative!"
        );
    }
}
