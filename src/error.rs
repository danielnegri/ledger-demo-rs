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

//! Error types for transaction processing.

use thiserror::Error;

/// Transaction processing errors.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TransactionError {    
    /// Amount field is missing for deposit or withdrawal
    #[error("missing amount for deposit/withdrawal")]
    MissingAmount,

    /// Amount is zero or negative
    #[error("invalid amount (must be positive)")]
    InvalidAmount,

    /// Withdrawal would exceed the available balance
    #[error("insufficient available funds")]
    InsufficientFunds,

    /// Referenced transaction ID does not exist
    #[error("transaction not found")]
    TransactionNotFound,

    /// Client does not own the referenced transaction
    #[error("client does not own this transaction")]
    ClientMismatch,

    /// Transaction is already under dispute
    #[error("transaction already under dispute")]
    AlreadyDisputed,

    /// Transaction is not under dispute
    #[error("transaction not under dispute")]
    NotDisputed,

    /// Only deposits can be disputed
    #[error("only deposits can be disputed")]
    NotDisputable,

    /// Duplicate transaction ID
    #[error("duplicate transaction ID")]
    DuplicateTransaction,

    /// Account is locked (after chargeback)
    #[error("account is locked")]
    AccountLocked,
}

#[cfg(test)]
mod tests {
    use super::TransactionError;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            TransactionError::MissingAmount.to_string(),
            "missing amount for deposit/withdrawal"
        );
        assert_eq!(
            TransactionError::InvalidAmount.to_string(),
            "invalid amount (must be positive)"
        );
        assert_eq!(
            TransactionError::InsufficientFunds.to_string(),
            "insufficient available funds"
        );
        assert_eq!(TransactionError::TransactionNotFound.to_string(), "transaction not found");
        assert_eq!(
            TransactionError::ClientMismatch.to_string(),
            "client does not own this transaction"
        );
        assert_eq!(
            TransactionError::AlreadyDisputed.to_string(),
            "transaction already under dispute"
        );
        assert_eq!(
            TransactionError::NotDisputed.to_string(),
            "transaction not under dispute"
        );
        assert_eq!(
            TransactionError::NotDisputable.to_string(),
            "only deposits can be disputed"
        );
        assert_eq!(TransactionError::DuplicateTransaction.to_string(), "duplicate transaction ID");
        assert_eq!(TransactionError::AccountLocked.to_string(), "account is locked");
    }

    #[test]
    fn errors_are_cloneable() {
        let error = TransactionError::InsufficientFunds;
        let cloned = error.clone();
        assert_eq!(error, cloned);
    }
}
