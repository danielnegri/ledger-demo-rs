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

use ledger_demo_rs::{
    ClientId, Engine, TransactionError, TransactionId, TransactionStatus, TransactionType,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn make_deposit(client_id: u16, tx_id: u32, amount: Decimal) -> TransactionType {
    TransactionType::Deposit {
        client_id: ClientId(client_id),
        transaction_id: TransactionId(tx_id),
        amount,
        status: TransactionStatus::Applied,
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
    assert_eq!(account.available(), dec!(50.00));
    assert_eq!(account.total(), dec!(50.00));
}

#[test]
fn multiple_deposits_same_client() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_deposit(1, 2, dec!(50.00))).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available(), dec!(150.00));
}

#[test]
fn multiple_clients() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_deposit(2, 2, dec!(200.00))).unwrap();

    let account1 = engine.get_account(&ClientId(1)).unwrap();
    let account2 = engine.get_account(&ClientId(2)).unwrap();
    assert_eq!(account1.available(), dec!(100.00));
    assert_eq!(account2.available(), dec!(200.00));
}

#[test]
fn withdrawal_after_deposit() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_withdrawal(1, 2, dec!(30.00))).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available(), dec!(70.00));
}

#[test]
fn withdrawal_insufficient_funds() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(50.00))).unwrap();

    let result = engine.process(make_withdrawal(1, 2, dec!(100.00)));
    assert_eq!(result, Err(TransactionError::InsufficientFunds));

    // Balance unchanged
    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available(), dec!(50.00));
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

    {
        let account = engine.get_account(&ClientId(1)).unwrap();
        assert_eq!(account.available(), dec!(0.00));
        assert_eq!(account.held(), dec!(100.00));
    }

    engine.process(make_resolve(1, 1)).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available(), dec!(100.00));
    assert_eq!(account.held(), dec!(0.00));
}

#[test]
fn dispute_chargeback_flow() {
    let engine = Engine::new();
    engine.process(make_deposit(1, 1, dec!(100.00))).unwrap();
    engine.process(make_dispute(1, 1)).unwrap();
    engine.process(make_chargeback(1, 1)).unwrap();

    let account = engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.available(), dec!(0.00));
    assert_eq!(account.held(), dec!(0.00));
    assert_eq!(account.total(), dec!(0.00));
    assert!(account.locked());
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
