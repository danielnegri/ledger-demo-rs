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

//! Engine public API integration tests.

use ledger_demo_rs::{ClientId, Engine, TransactionError, TransactionId, TransactionType};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn make_deposit(client_id: u16, tx_id: u32, amount: Decimal) -> TransactionType {
    TransactionType::Deposit {
        client_id: ClientId(client_id),
        transaction_id: TransactionId(tx_id),
        amount,
    }
}

fn make_withdrawal(client_id: u16, tx_id: u32, amount: Decimal) -> TransactionType {
    TransactionType::Withdrawal {
        client_id: ClientId(client_id),
        transaction_id: TransactionId(tx_id),
        amount,
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

#[test]
fn deposit_creates_account() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 100, dec!(50.00))).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(50.00));
    assert_eq!(account.total, dec!(50.00));
}

#[test]
fn multiple_deposits_same_client() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_deposit(1, 2, dec!(50.00))).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(150.00));
}

#[test]
fn multiple_clients() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_deposit(2, 2, dec!(200.00))).unwrap();

    let account1 = engine.get_account(&ClientId(1)).unwrap();
    let account2 = engine.get_account(&ClientId(2)).unwrap();
    assert_eq!(account1.available, dec!(100.00));
    assert_eq!(account2.available, dec!(200.00));
}

#[test]
fn withdrawal_after_deposit() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_withdrawal(1, 2, dec!(30.00))).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(70.00));
}

#[test]
fn withdrawal_insufficient_funds() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(50.00))).unwrap();

    let result = engine.process(make_withdrawal(1, 2, dec!(100.00)));
    assert_eq!(result, Err(TransactionError::InsufficientFunds));

    // Balance unchanged
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(50.00));
}

#[test]
fn withdrawal_on_new_account_fails() {
    let engine = Engine::new();
    // Withdrawal creates account but fails due to insufficient funds
    let result = engine.process(make_withdrawal(1, 1, dec!(100.00)));
    assert_eq!(result, Err(TransactionError::InsufficientFunds));
}

#[test]
fn duplicate_transaction_id_returns_error() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();

    // Same tx_id should fail
    let result = engine.process(make_deposit(1, 1, dec!(50.00)));
    assert_eq!(result, Err(TransactionError::DuplicateTransaction));
}

#[test]
fn dispute_resolve_flow() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_dispute(1, 1)).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(0.00));
    assert_eq!(account.held, dec!(100.00));

    engine.process(make_resolve(1, 1)).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(100.00));
    assert_eq!(account.held, dec!(0.00));
}

#[test]
fn dispute_chargeback_flow() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_dispute(1, 1)).unwrap();
    engine.process(make_chargeback(1, 1)).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(0.00));
    assert_eq!(account.held, dec!(0.00));
    assert_eq!(account.total, dec!(0.00));
    assert!(account.locked);
}

#[test]
fn dispute_nonexistent_account_returns_error() {
    let engine = Engine::new();
    // No account exists for client 1
    let result = engine.process(make_dispute(1, 1));
    assert_eq!(result, Err(TransactionError::TransactionNotFound));
}

#[test]
fn dispute_nonexistent_transaction_returns_error() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();

    // Transaction 999 doesn't exist
    let result = engine.process(make_dispute(1, 999));
    assert_eq!(result, Err(TransactionError::TransactionNotFound));
}

#[test]
fn resolve_without_dispute_returns_error() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();

    let result = engine.process(make_resolve(1, 1));
    assert_eq!(result, Err(TransactionError::NotDisputed));
}

#[test]
fn chargeback_without_dispute_returns_error() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();

    let result = engine.process(make_chargeback(1, 1));
    assert_eq!(result, Err(TransactionError::NotDisputed));
}

// =============================================================================
// Dispute After Withdrawal - Edge Case Documentation
// =============================================================================
//
// When a client disputes a deposit after partially or fully withdrawing those
// funds, the dispute will fail with `InsufficientFunds`. This is the correct
// behavior because:
//
// 1. A dispute moves funds from `available` to `held`
// 2. If `available < dispute_amount`, we cannot hold the full disputed amount
// 3. The dispute is rejected to maintain the invariant: `available >= 0`
//
// This prevents a scenario where a malicious actor could:
// - Deposit $100
// - Withdraw $100
// - Dispute the original deposit (attempting to get $100 held/refunded)
// - Effectively "double-spend" by having withdrawn funds AND disputed funds
//
// The trade-off is that legitimate disputes may fail if the client has already
// spent some of the deposited funds. In a production system, you might want to:
// - Dispute only the remaining available portion
// - Flag the account for manual review
// - Implement a negative balance hold (with appropriate risk controls)
// =============================================================================

/// Disputing a deposit after full withdrawal fails with InsufficientFunds.
///
/// Scenario:
/// 1. Client deposits $100 (tx 1)
/// 2. Client withdraws $100 (tx 2) - available is now $0
/// 3. Dispute on tx 1 fails - cannot hold $100 when available is $0
///
/// This prevents double-spend fraud where a client withdraws funds and then
/// disputes the original deposit to get the money back twice.
#[test]
fn dispute_after_full_withdrawal_fails() {
    let engine = Engine::new();

    // Deposit $100
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();

    // Withdraw the full amount
    engine.process(make_withdrawal(1, 2, dec!(100.00))).unwrap();

    // Verify balance is zero
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(0.00));
    assert_eq!(account.held, dec!(0.00));

    // Attempt to dispute the original deposit - should fail
    let result = engine.process(make_dispute(1, 1));
    assert_eq!(
        result,
        Err(TransactionError::InsufficientFunds),
        "Dispute should fail: cannot hold $100 when available balance is $0"
    );

    // Account state should be unchanged
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(0.00));
    assert_eq!(account.held, dec!(0.00));
    assert!(!account.locked, "Account should not be locked");
}

/// Disputing a deposit after partial withdrawal fails with InsufficientFunds.
///
/// Scenario:
/// 1. Client deposits $100 (tx 1)
/// 2. Client withdraws $60 (tx 2) - available is now $40
/// 3. Dispute on tx 1 fails - cannot hold $100 when only $40 is available
///
/// The dispute requires holding the FULL original deposit amount, not just
/// the remaining balance. This is a design decision - alternative approaches
/// could allow partial disputes.
#[test]
fn dispute_after_partial_withdrawal_fails() {
    let engine = Engine::new();

    // Deposit $100
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();

    // Withdraw $60 (leaving $40 available)
    engine.process(make_withdrawal(1, 2, dec!(60.00))).unwrap();

    // Verify balance
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(40.00));

    // Attempt to dispute the original $100 deposit - should fail
    // because we can't hold $100 when only $40 is available
    let result = engine.process(make_dispute(1, 1));
    assert_eq!(
        result,
        Err(TransactionError::InsufficientFunds),
        "Dispute should fail: cannot hold $100 when only $40 is available"
    );

    // Account state should be unchanged
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(40.00));
    assert_eq!(account.held, dec!(0.00));
}

/// Disputing a deposit succeeds when sufficient funds remain.
///
/// Scenario:
/// 1. Client deposits $100 (tx 1)
/// 2. Client deposits $50 (tx 2) - available is now $150
/// 3. Client withdraws $40 (tx 3) - available is now $110
/// 4. Dispute on tx 1 succeeds - $100 moved from available to held
///
/// The dispute succeeds because available ($110) >= dispute amount ($100).
#[test]
fn dispute_succeeds_when_sufficient_funds_remain() {
    let engine = Engine::new();

    // Deposit $100 (will be disputed later)
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();

    // Deposit another $50
    engine.process(make_deposit(1, 2, dec!(50.00))).unwrap();

    // Withdraw $40 (available: $150 - $40 = $110)
    engine.process(make_withdrawal(1, 3, dec!(40.00))).unwrap();

    // Verify balance before dispute
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(110.00));
    assert_eq!(account.held, dec!(0.00));

    // Dispute the first deposit ($100) - should succeed
    engine.process(make_dispute(1, 1)).unwrap();

    // Verify funds moved to held
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(
        account.available,
        dec!(10.00),
        "available should be $110 - $100 = $10"
    );
    assert_eq!(account.held, dec!(100.00), "held should be $100");
    assert_eq!(account.total, dec!(110.00), "total should be unchanged");
}

/// Multiple deposits where only one can be disputed due to withdrawals.
///
/// Scenario:
/// 1. Deposit $100 (tx 1)
/// 2. Deposit $50 (tx 2)
/// 3. Withdraw $120 - available is now $30
/// 4. Dispute tx 1 ($100) fails - insufficient funds
/// 5. Dispute tx 2 ($50) also fails - insufficient funds
///
/// Neither deposit can be disputed because available ($30) < both amounts.
#[test]
fn dispute_fails_for_all_deposits_when_funds_depleted() {
    let engine = Engine::new();

    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_deposit(1, 2, dec!(50.00))).unwrap();
    engine.process(make_withdrawal(1, 3, dec!(120.00))).unwrap();

    // Only $30 remaining
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(30.00));

    // Neither deposit can be disputed
    let result1 = engine.process(make_dispute(1, 1));
    assert_eq!(result1, Err(TransactionError::InsufficientFunds));

    let result2 = engine.process(make_dispute(1, 2));
    assert_eq!(result2, Err(TransactionError::InsufficientFunds));

    // Balance unchanged
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(30.00));
    assert_eq!(account.held, dec!(0.00));
}

/// Dispute succeeds for smaller deposit after withdrawal.
///
/// Scenario:
/// 1. Deposit $100 (tx 1)
/// 2. Deposit $20 (tx 2)
/// 3. Withdraw $90 - available is now $30
/// 4. Dispute tx 1 ($100) fails - insufficient funds
/// 5. Dispute tx 2 ($20) succeeds - $30 available >= $20
#[test]
fn dispute_smaller_deposit_succeeds_after_withdrawal() {
    let engine = Engine::new();

    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_deposit(1, 2, dec!(20.00))).unwrap();
    engine.process(make_withdrawal(1, 3, dec!(90.00))).unwrap();

    // $30 remaining
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(30.00));

    // Large deposit cannot be disputed
    let result1 = engine.process(make_dispute(1, 1));
    assert_eq!(result1, Err(TransactionError::InsufficientFunds));

    // Smaller deposit CAN be disputed
    engine
        .process(make_dispute(1, 2))
        .expect("Dispute should succeed for $20 when $30 is available");

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(10.00));
    assert_eq!(account.held, dec!(20.00));
}

/// Withdrawal during active dispute fails due to insufficient available funds.
///
/// Scenario:
/// 1. Deposit $100 (tx 1)
/// 2. Dispute tx 1 - all $100 moved to held
/// 3. Withdrawal of any amount fails - available is $0
///
/// This ensures disputed funds cannot be withdrawn while under investigation.
#[test]
fn withdrawal_during_dispute_fails() {
    let engine = Engine::new();

    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_dispute(1, 1)).unwrap();

    // All funds are held
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(0.00));
    assert_eq!(account.held, dec!(100.00));

    // Cannot withdraw - no available funds
    let result = engine.process(make_withdrawal(1, 2, dec!(1.00)));
    assert_eq!(result, Err(TransactionError::InsufficientFunds));
}

/// Chargeback after partial withdrawal results in remaining balance.
///
/// Scenario:
/// 1. Deposit $100 (tx 1)
/// 2. Deposit $100 (tx 2)
/// 3. Withdraw $50 - available is $150
/// 4. Dispute tx 2 - $100 held, $50 available
/// 5. Chargeback tx 2 - $100 removed, account locked with $50 remaining
///
/// The client keeps the $50 they withdrew, but account is now frozen.
#[test]
fn chargeback_with_remaining_balance_locks_account() {
    let engine = Engine::new();

    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_deposit(1, 2, dec!(100.00))).unwrap();
    engine.process(make_withdrawal(1, 3, dec!(50.00))).unwrap();

    // available: $150, held: $0
    engine.process(make_dispute(1, 2)).unwrap();

    // available: $50, held: $100
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available, dec!(50.00));
    assert_eq!(account.held, dec!(100.00));

    engine.process(make_chargeback(1, 2)).unwrap();

    // Chargeback removes held funds and locks account
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(
        account.available,
        dec!(50.00),
        "Client keeps their remaining $50"
    );
    assert_eq!(account.held, dec!(0.00));
    assert_eq!(account.total, dec!(50.00));
    assert!(account.locked, "Account must be locked after chargeback");

    // Locked account cannot withdraw remaining funds
    let result = engine.process(make_withdrawal(1, 4, dec!(50.00)));
    assert_eq!(result, Err(TransactionError::AccountLocked));
}
