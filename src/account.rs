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

//! Account management.
//!
//! Implemented State Machine
//!
//! # Example
//!
//! ```
//! use rust_decimal_macros::dec;
//! use ledger_demo_rs::{Account, ClientId};
//!
//! let account = Account::new(ClientId(1));
//! assert_eq!(account.available(), dec!(0.00));
//! ```

use crate::base::{ClientId, TransactionId};
use crate::transaction::TransactionStatus;
use crate::{TransactionError, TransactionType};
use parking_lot::Mutex;
use rust_decimal::Decimal;
use serde::ser::{Serialize, SerializeStruct, Serializer};
use std::collections::HashMap;

/// Tracks deposit amount and status for dispute resolution.
///
//  Deposit (Applied) ──dispute──► Deposit (Inflight) ──resolve───► Deposit (Resolved)
//                                        │
//                                        └──chargeback──► Deposit (Voided) + Account Locked
#[derive(Debug, Clone)]
struct DepositRecord {
    amount: Decimal,
    status: TransactionStatus,
}

#[derive(Debug)]
struct AccountData {
    client_id: ClientId,
    available: Decimal,
    held: Decimal,
    locked: bool,
    /// Deposits indexed by transaction ID for dispute lookup.
    deposits: HashMap<TransactionId, DepositRecord>,
}

impl AccountData {
    fn new(client_id: ClientId) -> Self {
        Self {
            client_id,
            available: Decimal::ZERO,
            held: Decimal::ZERO,
            locked: false,
            deposits: HashMap::new(),
        }
    }

    fn assert_invariants(&self) {
        debug_assert!(
            self.available >= Decimal::ZERO,
            "Invariant violated: available balance went negative: {}",
            self.available
        );
        debug_assert!(
            self.held >= Decimal::ZERO,
            "Invariant violated: held balance went negative: {}",
            self.held
        );
    }

    /// Increases available balance.
    fn deposit(&mut self, amount: Decimal) -> Result<(), TransactionError> {
        if amount <= Decimal::ZERO {
            return Err(TransactionError::InvalidAmount);
        }
        if self.locked {
            return Err(TransactionError::AccountLocked);
        }
        self.available += amount;
        self.assert_invariants();
        Ok(())
    }

    /// Decreases available balance.
    fn withdraw(&mut self, amount: Decimal) -> Result<(), TransactionError> {
        if amount <= Decimal::ZERO {
            return Err(TransactionError::InvalidAmount);
        }
        if self.locked {
            return Err(TransactionError::AccountLocked);
        }
        if self.available < amount {
            return Err(TransactionError::InsufficientFunds);
        }
        self.available -= amount;
        self.assert_invariants();
        Ok(())
    }

    /// Moves funds from available to held (dispute).
    fn hold_funds(&mut self, amount: Decimal) -> Result<(), TransactionError> {
        if amount <= Decimal::ZERO {
            return Err(TransactionError::InvalidAmount);
        }
        if self.locked {
            return Err(TransactionError::AccountLocked);
        }
        if self.available < amount {
            return Err(TransactionError::InsufficientFunds);
        }
        self.available -= amount;
        self.held += amount;
        self.assert_invariants();
        Ok(())
    }

    /// Moves funds from held to available (resolve).
    fn release_funds(&mut self, amount: Decimal) -> Result<(), TransactionError> {
        if amount <= Decimal::ZERO {
            return Err(TransactionError::InvalidAmount);
        }
        if self.locked {
            return Err(TransactionError::AccountLocked);
        }
        if self.held < amount {
            return Err(TransactionError::InsufficientFunds);
        }
        self.held -= amount;
        self.available += amount;
        self.assert_invariants();
        Ok(())
    }

    /// Removes held funds and locks the account (chargeback).
    fn chargeback(&mut self, amount: Decimal) -> Result<(), TransactionError> {
        if amount <= Decimal::ZERO {
            return Err(TransactionError::InvalidAmount);
        }
        if self.locked {
            return Err(TransactionError::AccountLocked);
        }
        if self.held < amount {
            return Err(TransactionError::InsufficientFunds);
        }
        self.held -= amount;
        self.locked = true;
        self.assert_invariants();
        Ok(())
    }
}

/// Ledger account.
#[derive(Debug)]
pub struct Account {
    inner: Mutex<AccountData>,
}

impl Account {
    const DECIMAL_PRECISION: u32 = 4;

    pub fn new(client_id: ClientId) -> Self {
        Self {
            inner: Mutex::new(AccountData::new(client_id)),
        }
    }

    pub fn available(&self) -> Decimal {
        self.inner.lock().available
    }

    pub fn held(&self) -> Decimal {
        self.inner.lock().held
    }

    /// Returns `available + held`.
    pub fn total(&self) -> Decimal {
        let data = self.inner.lock();
        data.available + data.held
    }

    pub fn locked(&self) -> bool {
        self.inner.lock().locked
    }

    pub fn add_transaction(
        &mut self,
        transaction: TransactionType,
    ) -> Result<(), TransactionError> {
        let mut data = self.inner.lock();
        if transaction.client_id() != data.client_id {
            return Err(TransactionError::ClientMismatch);
        }

        match transaction {
            TransactionType::Deposit {
                transaction_id,
                amount,
                ..
            } => {
                // Process deposit
                data.deposit(amount)?;

                // Track deposit for future disputes
                data.deposits.insert(
                    transaction_id,
                    DepositRecord {
                        amount,
                        status: TransactionStatus::Applied,
                    },
                );
            }
            TransactionType::Withdrawal { amount, .. } => {
                // Process withdrawal (withdrawals cannot be disputed)
                data.withdraw(amount)?;
            }
            TransactionType::Dispute { transaction_id, .. } => {
                // Look up the referenced deposit
                let deposit = data
                    .deposits
                    .get(&transaction_id)
                    .ok_or(TransactionError::TransactionNotFound)?;

                // Only Applied deposits can be disputed
                if deposit.status != TransactionStatus::Applied {
                    return Err(TransactionError::AlreadyDisputed);
                }

                let amount = deposit.amount;

                // Move funds from available to held
                data.hold_funds(amount)?;

                // Update deposit status to Inflight
                data.deposits.get_mut(&transaction_id).unwrap().status =
                    TransactionStatus::Inflight;
            }
            TransactionType::Resolve { transaction_id, .. } => {
                // Look up the referenced deposit
                let deposit = data
                    .deposits
                    .get(&transaction_id)
                    .ok_or(TransactionError::TransactionNotFound)?;

                // Only Inflight deposits can be resolved
                if deposit.status != TransactionStatus::Inflight {
                    return Err(TransactionError::NotDisputed);
                }

                let amount = deposit.amount;

                // Move funds from held back to available
                data.release_funds(amount)?;

                // Update deposit status to Resolved
                data.deposits.get_mut(&transaction_id).unwrap().status =
                    TransactionStatus::Resolved;
            }
            TransactionType::Chargeback { transaction_id, .. } => {
                // Look up the referenced deposit
                let deposit = data
                    .deposits
                    .get(&transaction_id)
                    .ok_or(TransactionError::TransactionNotFound)?;

                // Only Inflight deposits can be charged back
                if deposit.status != TransactionStatus::Inflight {
                    return Err(TransactionError::NotDisputed);
                }

                let amount = deposit.amount;

                // Remove funds from held and lock account
                data.chargeback(amount)?;

                // Update deposit status to Voided
                data.deposits.get_mut(&transaction_id).unwrap().status = TransactionStatus::Voided;
            }
        }

        Ok(())
    }
}

impl Serialize for Account {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let data = self.inner.lock();
        let mut state = serializer.serialize_struct("Account", 5)?;
        state.serialize_field("client", &data.client_id)?;
        state.serialize_field(
            "available",
            &data.available.round_dp(Account::DECIMAL_PRECISION),
        )?;
        state.serialize_field("held", &data.held.round_dp(Account::DECIMAL_PRECISION))?;
        state.serialize_field(
            "total",
            &(data.available + data.held).round_dp(Account::DECIMAL_PRECISION),
        )?;
        state.serialize_field("locked", &data.locked)?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    // === AccountData Internal Tests ===
    // These test the private AccountData methods directly.

    #[test]
    fn account_data_hold_funds() {
        let mut data = AccountData::new(ClientId(1));
        data.deposit(dec!(100.00)).unwrap();
        data.hold_funds(dec!(30.00)).unwrap();
        assert_eq!(data.available, dec!(70.00));
        assert_eq!(data.held, dec!(30.00));
    }

    #[test]
    fn account_data_release_funds() {
        let mut data = AccountData::new(ClientId(1));
        data.deposit(dec!(100.00)).unwrap();
        data.hold_funds(dec!(30.00)).unwrap();
        data.release_funds(dec!(30.00)).unwrap();
        assert_eq!(data.available, dec!(100.00));
        assert_eq!(data.held, Decimal::ZERO);
    }

    #[test]
    fn account_data_chargeback_locks_account() {
        let mut data = AccountData::new(ClientId(1));
        data.deposit(dec!(100.00)).unwrap();
        data.hold_funds(dec!(50.00)).unwrap();
        data.chargeback(dec!(50.00)).unwrap();
        assert!(data.locked);
        assert_eq!(data.available, dec!(50.00));
        assert_eq!(data.held, Decimal::ZERO);
    }

    #[test]
    fn locked_account_rejects_deposit() {
        let mut data = AccountData::new(ClientId(1));
        data.deposit(dec!(100.00)).unwrap();
        data.hold_funds(dec!(50.00)).unwrap();
        data.chargeback(dec!(50.00)).unwrap();

        let result = data.deposit(dec!(10.00));
        assert_eq!(result, Err(TransactionError::AccountLocked));
    }

    #[test]
    fn locked_account_rejects_withdrawal() {
        let mut data = AccountData::new(ClientId(1));
        data.deposit(dec!(100.00)).unwrap();
        data.hold_funds(dec!(50.00)).unwrap();
        data.chargeback(dec!(50.00)).unwrap();

        let result = data.withdraw(dec!(10.00));
        assert_eq!(result, Err(TransactionError::AccountLocked));
    }

    #[test]
    fn hold_funds_insufficient_returns_error() {
        let mut data = AccountData::new(ClientId(1));
        data.deposit(dec!(50.00)).unwrap();
        let result = data.hold_funds(dec!(100.00));
        assert_eq!(result, Err(TransactionError::InsufficientFunds));
    }

    #[test]
    fn release_funds_insufficient_returns_error() {
        let mut data = AccountData::new(ClientId(1));
        data.deposit(dec!(100.00)).unwrap();
        data.hold_funds(dec!(30.00)).unwrap();
        let result = data.release_funds(dec!(50.00));
        assert_eq!(result, Err(TransactionError::InsufficientFunds));
    }

    #[test]
    fn chargeback_insufficient_returns_error() {
        let mut data = AccountData::new(ClientId(1));
        data.deposit(dec!(100.00)).unwrap();
        data.hold_funds(dec!(30.00)).unwrap();
        let result = data.chargeback(dec!(50.00));
        assert_eq!(result, Err(TransactionError::InsufficientFunds));
        assert!(!data.locked); // Should not be locked
    }

    // === Serialization Tests ===

    #[test]
    fn serializer_rounds_to_four_decimal_places() {
        use serde_json;

        let account = Account::new(ClientId(1));

        // Deposit amount with more than 4 decimal places
        {
            let mut data = account.inner.lock();
            // 123.456789 should round to 123.4568
            data.available = dec!(123.456789);
            data.held = dec!(0.000001); // Should round to 0.0000
        }

        let json = serde_json::to_string(&account).unwrap();

        // Parse the JSON to verify precision
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Available should be rounded to 4 decimal places: 123.456789 -> 123.4568
        let available = parsed["available"].as_str().unwrap();
        assert_eq!(
            available, "123.4568",
            "available should round to 4 decimal places"
        );

        // Held should be rounded to 4 decimal places: 0.000001 -> 0.0000
        let held = parsed["held"].as_str().unwrap();
        assert_eq!(held, "0.0000", "held should round to 4 decimal places");

        // Total should also be rounded
        let total = parsed["total"].as_str().unwrap();
        assert_eq!(total, "123.4568", "total should round to 4 decimal places");
    }

    #[test]
    fn serializer_preserves_precision_up_to_four_decimals() {
        use serde_json;

        let account = Account::new(ClientId(42));

        {
            let mut data = account.inner.lock();
            data.available = dec!(100.1234);
            data.held = dec!(50.5678);
        }

        let json = serde_json::to_string(&account).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["client"], 42);
        assert_eq!(parsed["available"].as_str().unwrap(), "100.1234");
        assert_eq!(parsed["held"].as_str().unwrap(), "50.5678");
        assert_eq!(parsed["total"].as_str().unwrap(), "150.6912");
        assert_eq!(parsed["locked"], false);
    }

    #[test]
    fn serializer_handles_whole_numbers() {
        use serde_json;

        let account = Account::new(ClientId(1));

        {
            let mut data = account.inner.lock();
            data.available = dec!(1000);
            data.held = dec!(500);
        }

        let json = serde_json::to_string(&account).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Whole numbers serialize without trailing zeros
        assert_eq!(parsed["available"].as_str().unwrap(), "1000");
        assert_eq!(parsed["held"].as_str().unwrap(), "500");
        assert_eq!(parsed["total"].as_str().unwrap(), "1500");
    }

    #[test]
    fn serializer_uses_bankers_rounding() {
        use serde_json;

        let account = Account::new(ClientId(1));

        {
            let mut data = account.inner.lock();
            // Banker's rounding (round half to even):
            // 0.00005 rounds to 0.0000 (rounds to even)
            // 0.00015 rounds to 0.0002 (rounds to even)
            data.available = dec!(0.00015);
            data.held = dec!(0.00005);
        }

        let json = serde_json::to_string(&account).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Decimal uses banker's rounding by default
        assert_eq!(parsed["available"].as_str().unwrap(), "0.0002");
        assert_eq!(parsed["held"].as_str().unwrap(), "0.0000");
    }

    #[test]
    fn serializer_precision_constant_is_four() {
        // Verify the precision constant is set correctly
        assert_eq!(Account::DECIMAL_PRECISION, 4);
    }
}
