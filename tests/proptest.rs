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

//! Property-based tests for the ledger engine.
//!
//! These tests verify invariants that should hold for any sequence of
//! valid transactions.

use ledger_demo_rs::{
    Account, ClientId, Engine, TransactionError, TransactionId, TransactionSatus, TransactionType,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

// =============================================================================
// Arbitrary Strategies
// =============================================================================

/// Generate a positive amount (1 to 10000 with 4 decimal places).
fn arb_amount() -> impl Strategy<Value = Decimal> {
    (1i64..=10_000_000i64).prop_map(|cents| Decimal::new(cents, 4))
}

// =============================================================================
// Account Invariant Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Total balance always equals available + held.
    #[test]
    fn total_equals_available_plus_held(
        deposits in prop::collection::vec(arb_amount(), 1..10),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        for (i, amount) in deposits.iter().enumerate() {
            let tx = TransactionType::Deposit {
                client_id,
                transaction_id: TransactionId(i as u32),
                amount: *amount,
                status: TransactionSatus::Applied,
            };
            let _ = account.add_transaction(tx);
        }

        prop_assert_eq!(account.total(), account.available() + account.held());
    }

    /// Available balance is never negative after any operation.
    #[test]
    fn available_never_negative(
        deposits in prop::collection::vec(arb_amount(), 1..5),
        withdrawals in prop::collection::vec(arb_amount(), 0..5),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);
        let mut tx_counter = 0u32;

        // Process deposits
        for amount in &deposits {
            let tx = TransactionType::Deposit {
                client_id,
                transaction_id: TransactionId(tx_counter),
                amount: *amount,
                status: TransactionSatus::Applied,
            };
            tx_counter += 1;
            let _ = account.add_transaction(tx);
        }

        // Process withdrawals (may fail, that's ok)
        for amount in &withdrawals {
            let tx = TransactionType::Withdrawal {
                client_id,
                transaction_id: TransactionId(tx_counter),
                amount: *amount,
            };
            tx_counter += 1;
            let _ = account.add_transaction(tx);
        }

        prop_assert!(account.available() >= Decimal::ZERO);
    }

    /// Held balance is never negative.
    #[test]
    fn held_never_negative(
        deposit_amount in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        // Make a deposit
        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        // Dispute it
        let dispute = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        let _ = account.add_transaction(dispute);

        // Resolve it
        let resolve = TransactionType::Resolve {
            client_id,
            transaction_id: TransactionId(1),
        };
        let _ = account.add_transaction(resolve);

        prop_assert!(account.held() >= Decimal::ZERO);
    }
}

// =============================================================================
// Deposit Property Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Sum of deposits equals total balance (when no withdrawals or disputes).
    #[test]
    fn deposits_sum_to_total(
        amounts in prop::collection::vec(arb_amount(), 1..20),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);
        let expected_total: Decimal = amounts.iter().copied().sum();

        for (i, amount) in amounts.iter().enumerate() {
            let tx = TransactionType::Deposit {
                client_id,
                transaction_id: TransactionId(i as u32),
                amount: *amount,
                status: TransactionSatus::Applied,
            };
            account.add_transaction(tx).unwrap();
        }

        prop_assert_eq!(account.total(), expected_total);
        prop_assert_eq!(account.available(), expected_total);
        prop_assert_eq!(account.held(), Decimal::ZERO);
    }

    /// Order of deposits doesn't affect final balance.
    #[test]
    fn deposit_order_independent(
        amounts in prop::collection::vec(arb_amount(), 2..10),
    ) {
        let client_id = ClientId(1);
        let expected_total: Decimal = amounts.iter().copied().sum();

        // Process in original order
        let mut account1 = Account::new(client_id);
        for (i, amount) in amounts.iter().enumerate() {
            let tx = TransactionType::Deposit {
                client_id,
                transaction_id: TransactionId(i as u32),
                amount: *amount,
                status: TransactionSatus::Applied,
            };
            account1.add_transaction(tx).unwrap();
        }

        // Process in reverse order (with different tx IDs)
        let mut account2 = Account::new(client_id);
        for (i, amount) in amounts.iter().rev().enumerate() {
            let tx = TransactionType::Deposit {
                client_id,
                transaction_id: TransactionId((i + 1000) as u32),
                amount: *amount,
                status: TransactionSatus::Applied,
            };
            account2.add_transaction(tx).unwrap();
        }

        prop_assert_eq!(account1.total(), account2.total());
        prop_assert_eq!(account1.total(), expected_total);
    }
}

// =============================================================================
// Withdrawal Property Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Withdrawals correctly reduce available balance.
    #[test]
    fn withdrawal_reduces_available(
        deposit_amount in arb_amount(),
        withdrawal_fraction in 0.01f64..0.99,
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        let withdrawal_amount = deposit_amount * Decimal::try_from(withdrawal_fraction).unwrap();
        let withdrawal_amount = withdrawal_amount.round_dp(4);

        if withdrawal_amount > Decimal::ZERO {
            let withdrawal = TransactionType::Withdrawal {
                client_id,
                transaction_id: TransactionId(2),
                amount: withdrawal_amount,
            };
            account.add_transaction(withdrawal).unwrap();

            let expected = deposit_amount - withdrawal_amount;
            prop_assert_eq!(account.available(), expected);
        }
    }

    /// Cannot withdraw more than available.
    #[test]
    fn cannot_overdraw(
        deposit_amount in arb_amount(),
        extra in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        let withdrawal_amount = deposit_amount + extra;
        let withdrawal = TransactionType::Withdrawal {
            client_id,
            transaction_id: TransactionId(2),
            amount: withdrawal_amount,
        };

        let result = account.add_transaction(withdrawal);
        prop_assert_eq!(result, Err(TransactionError::InsufficientFunds));
        prop_assert_eq!(account.available(), deposit_amount);
    }

    /// Multiple withdrawals sum correctly.
    #[test]
    fn multiple_withdrawals_sum_correctly(
        deposit_amount in (100i64..=1_000_000i64).prop_map(|v| Decimal::new(v, 4)),
        withdrawal_count in 1usize..=5,
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(0),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        // Withdraw small equal amounts
        let per_withdrawal = (deposit_amount / Decimal::from(withdrawal_count as i64 * 2)).round_dp(4);
        let mut total_withdrawn = Decimal::ZERO;

        for i in 0..withdrawal_count {
            if per_withdrawal > Decimal::ZERO {
                let withdrawal = TransactionType::Withdrawal {
                    client_id,
                    transaction_id: TransactionId((i + 1) as u32),
                    amount: per_withdrawal,
                };
                if account.add_transaction(withdrawal).is_ok() {
                    total_withdrawn += per_withdrawal;
                }
            }
        }

        prop_assert_eq!(account.available(), deposit_amount - total_withdrawn);
    }
}

// =============================================================================
// Dispute Lifecycle Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Dispute moves funds from available to held, preserving total.
    #[test]
    fn dispute_preserves_total(
        deposit_amount in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        let total_before = account.total();

        let dispute = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(dispute).unwrap();

        prop_assert_eq!(account.total(), total_before);
        prop_assert_eq!(account.held(), deposit_amount);
        prop_assert_eq!(account.available(), Decimal::ZERO);
    }

    /// Resolve returns funds from held to available, preserving total.
    #[test]
    fn resolve_preserves_total(
        deposit_amount in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        let dispute = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(dispute).unwrap();

        let total_before = account.total();

        let resolve = TransactionType::Resolve {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(resolve).unwrap();

        prop_assert_eq!(account.total(), total_before);
        prop_assert_eq!(account.held(), Decimal::ZERO);
        prop_assert_eq!(account.available(), deposit_amount);
    }

    /// Chargeback removes held funds and locks account.
    #[test]
    fn chargeback_removes_funds_and_locks(
        deposit_amount in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        let dispute = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(dispute).unwrap();

        let chargeback = TransactionType::Chargeback {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(chargeback).unwrap();

        prop_assert!(account.locked());
        prop_assert_eq!(account.held(), Decimal::ZERO);
        prop_assert_eq!(account.available(), Decimal::ZERO);
        prop_assert_eq!(account.total(), Decimal::ZERO);
    }

    /// Cannot dispute already disputed transaction.
    #[test]
    fn cannot_double_dispute(
        deposit_amount in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        let dispute1 = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(dispute1).unwrap();

        let dispute2 = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        let result = account.add_transaction(dispute2);

        prop_assert_eq!(result, Err(TransactionError::AlreadyDisputed));
    }

    /// Cannot resolve non-disputed transaction.
    #[test]
    fn cannot_resolve_non_disputed(
        deposit_amount in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        let resolve = TransactionType::Resolve {
            client_id,
            transaction_id: TransactionId(1),
        };
        let result = account.add_transaction(resolve);

        prop_assert_eq!(result, Err(TransactionError::NotDisputed));
    }
}

// =============================================================================
// Locked Account Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Locked account rejects all new deposits.
    #[test]
    fn locked_rejects_deposits(
        initial_deposit in arb_amount(),
        new_deposit in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        // Setup: deposit -> dispute -> chargeback
        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: initial_deposit,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        let dispute = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(dispute).unwrap();

        let chargeback = TransactionType::Chargeback {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(chargeback).unwrap();

        // Try to deposit to locked account
        let new_tx = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(2),
            amount: new_deposit,
            status: TransactionSatus::Applied,
        };
        let result = account.add_transaction(new_tx);

        prop_assert_eq!(result, Err(TransactionError::AccountLocked));
    }

    /// Locked account rejects all withdrawals.
    #[test]
    fn locked_rejects_withdrawals(
        initial_deposit in arb_amount(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        // Make two deposits so there's remaining balance after chargeback
        let deposit1 = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: initial_deposit,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit1).unwrap();

        let deposit2 = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(2),
            amount: initial_deposit,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit2).unwrap();

        // Dispute and chargeback only first deposit
        let dispute = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(dispute).unwrap();

        let chargeback = TransactionType::Chargeback {
            client_id,
            transaction_id: TransactionId(1),
        };
        account.add_transaction(chargeback).unwrap();

        // Account should have remaining balance but be locked
        prop_assert!(account.locked());
        prop_assert_eq!(account.available(), initial_deposit);

        // Try to withdraw
        let withdrawal = TransactionType::Withdrawal {
            client_id,
            transaction_id: TransactionId(3),
            amount: Decimal::new(1, 4), // Tiny amount
        };
        let result = account.add_transaction(withdrawal);

        prop_assert_eq!(result, Err(TransactionError::AccountLocked));
    }
}

// =============================================================================
// Engine Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Duplicate transaction IDs are rejected.
    #[test]
    fn engine_rejects_duplicate_tx_ids(
        amount1 in arb_amount(),
        amount2 in arb_amount(),
    ) {
        let engine = Engine::new();
        let client_id = ClientId(1);
        let tx_id = TransactionId(1);

        let deposit1 = TransactionType::Deposit {
            client_id,
            transaction_id: tx_id,
            amount: amount1,
            status: TransactionSatus::Applied,
        };
        engine.process(deposit1).unwrap();

        let deposit2 = TransactionType::Deposit {
            client_id,
            transaction_id: tx_id, // Same ID!
            amount: amount2,
            status: TransactionSatus::Applied,
        };
        let result = engine.process(deposit2);

        prop_assert_eq!(result, Err(TransactionError::DuplicateTransaction));
    }

    /// Different clients are isolated.
    #[test]
    fn clients_are_isolated(
        amount1 in arb_amount(),
        amount2 in arb_amount(),
    ) {
        let engine = Engine::new();

        let deposit1 = TransactionType::Deposit {
            client_id: ClientId(1),
            transaction_id: TransactionId(1),
            amount: amount1,
            status: TransactionSatus::Applied,
        };
        engine.process(deposit1).unwrap();

        let deposit2 = TransactionType::Deposit {
            client_id: ClientId(2),
            transaction_id: TransactionId(2),
            amount: amount2,
            status: TransactionSatus::Applied,
        };
        engine.process(deposit2).unwrap();

        let account1 = engine.get_account(&ClientId(1)).unwrap();
        let account2 = engine.get_account(&ClientId(2)).unwrap();

        prop_assert_eq!(account1.total(), amount1);
        prop_assert_eq!(account2.total(), amount2);
    }

    /// Dispute references must be for same client.
    #[test]
    fn dispute_must_match_client(
        amount in arb_amount(),
    ) {
        let engine = Engine::new();

        // Client 1 makes a deposit
        let deposit = TransactionType::Deposit {
            client_id: ClientId(1),
            transaction_id: TransactionId(1),
            amount,
            status: TransactionSatus::Applied,
        };
        engine.process(deposit).unwrap();

        // Client 2 tries to dispute it (should fail - no account exists)
        let dispute = TransactionType::Dispute {
            client_id: ClientId(2),
            transaction_id: TransactionId(1),
        };
        let result = engine.process(dispute);

        prop_assert_eq!(result, Err(TransactionError::TransactionNotFound));
    }

    /// Engine handles many transactions without panic.
    #[test]
    fn engine_handles_many_transactions(
        tx_count in 10usize..100,
    ) {
        let engine = Engine::new();
        let client_id = ClientId(1);

        for i in 0..tx_count {
            let amount = Decimal::new((i as i64 + 1) * 100, 4);
            let deposit = TransactionType::Deposit {
                client_id,
                transaction_id: TransactionId(i as u32),
                amount,
                status: TransactionSatus::Applied,
            };
            engine.process(deposit).unwrap();
        }

        let account = engine.get_account(&client_id).unwrap();
        let expected: Decimal = (1..=tx_count as i64)
            .map(|i| Decimal::new(i * 100, 4))
            .sum();

        prop_assert_eq!(account.total(), expected);
    }
}

// =============================================================================
// Complex Scenario Tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Full dispute lifecycle maintains invariants.
    #[test]
    fn full_dispute_lifecycle_invariants(
        deposits in prop::collection::vec(arb_amount(), 2..5),
        dispute_idx in 0usize..4,
        resolve_not_chargeback in any::<bool>(),
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);
        let expected_total: Decimal = deposits.iter().copied().sum();

        // Process all deposits
        for (i, amount) in deposits.iter().enumerate() {
            let tx = TransactionType::Deposit {
                client_id,
                transaction_id: TransactionId(i as u32),
                amount: *amount,
                status: TransactionSatus::Applied,
            };
            account.add_transaction(tx).unwrap();
        }

        prop_assert_eq!(account.total(), expected_total);

        // Pick a deposit to dispute
        let dispute_idx = dispute_idx % deposits.len();
        let disputed_amount = deposits[dispute_idx];

        let dispute = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(dispute_idx as u32),
        };
        account.add_transaction(dispute).unwrap();

        prop_assert_eq!(account.total(), expected_total);
        prop_assert_eq!(account.held(), disputed_amount);

        if resolve_not_chargeback {
            // Resolve the dispute
            let resolve = TransactionType::Resolve {
                client_id,
                transaction_id: TransactionId(dispute_idx as u32),
            };
            account.add_transaction(resolve).unwrap();

            prop_assert_eq!(account.total(), expected_total);
            prop_assert_eq!(account.held(), Decimal::ZERO);
            prop_assert_eq!(account.available(), expected_total);
            prop_assert!(!account.locked());
        } else {
            // Chargeback
            let chargeback = TransactionType::Chargeback {
                client_id,
                transaction_id: TransactionId(dispute_idx as u32),
            };
            account.add_transaction(chargeback).unwrap();

            prop_assert_eq!(account.total(), expected_total - disputed_amount);
            prop_assert_eq!(account.held(), Decimal::ZERO);
            prop_assert!(account.locked());
        }
    }

    /// Partial withdrawal followed by dispute.
    #[test]
    fn withdraw_then_dispute(
        deposit_amount in (100i64..=1_000_000i64).prop_map(|v| Decimal::new(v, 4)),
        withdraw_fraction in 0.1f64..0.5,
    ) {
        let client_id = ClientId(1);
        let mut account = Account::new(client_id);

        // Deposit
        let deposit = TransactionType::Deposit {
            client_id,
            transaction_id: TransactionId(1),
            amount: deposit_amount,
            status: TransactionSatus::Applied,
        };
        account.add_transaction(deposit).unwrap();

        // Withdraw partial amount
        let withdraw_amount = (deposit_amount * Decimal::try_from(withdraw_fraction).unwrap()).round_dp(4);
        let withdrawal = TransactionType::Withdrawal {
            client_id,
            transaction_id: TransactionId(2),
            amount: withdraw_amount,
        };
        account.add_transaction(withdrawal).unwrap();

        let available_after_withdraw = deposit_amount - withdraw_amount;
        prop_assert_eq!(account.available(), available_after_withdraw);

        // Now dispute the original deposit - should fail if withdrawn funds exceed available
        let dispute = TransactionType::Dispute {
            client_id,
            transaction_id: TransactionId(1),
        };
        let result = account.add_transaction(dispute);

        // If available < deposit_amount, dispute should fail (insufficient funds to hold)
        if available_after_withdraw < deposit_amount {
            prop_assert_eq!(result, Err(TransactionError::InsufficientFunds));
        } else {
            prop_assert!(result.is_ok());
        }
    }
}
