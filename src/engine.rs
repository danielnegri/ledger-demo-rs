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

//! Transaction processing engine.
//!
//! The [`Engine`] is the central component that processes transactions and manages
//! client accounts. It handles deposits, withdrawals, and the dispute lifecycle
//! (dispute, resolve, chargeback).
//!
//! # Transaction Processing
//!
//! - **Deposits**: Credit funds to a client account, creating the account if needed.
//! - **Withdrawals**: Debit funds from a client account (fails if insufficient funds).
//! - **Disputes**: Hold funds from a previous deposit pending investigation.
//! - **Resolves**: Release held funds back to available balance.
//! - **Chargebacks**: Remove held funds and lock the account.
//!
//! # Thread Safety
//!
//! The engine uses [`DashMap`] for concurrent access to accounts, allowing
//! multiple transactions to be processed in parallel for different clients.

use crate::account::Account;
use crate::base::ClientId;
use crate::{TransactionError, TransactionQueue, TransactionType};
use dashmap::DashMap;
use std::sync::Arc;

/// Transaction processing engine that manages client accounts.
///
/// The engine maintains a collection of client accounts and a transaction log.
/// It processes transactions sequentially per client while allowing concurrent
/// access across different clients.
///
/// # Invariants
///
/// - Transaction IDs are globally unique across all transaction types.
/// - Only deposits can be disputed (withdrawals cannot).
/// - Disputes can only transition: `Applied` -> `Inflight` -> `Resolved` or `Voided`.
/// - A chargeback permanently locks the client account.
pub struct Engine {
    /// Client accounts indexed by client ID.
    accounts: DashMap<ClientId, Account>,
    /// Global transaction log for deduplication.
    transactions: TransactionQueue,
}

impl Engine {
    /// Creates a new engine with no accounts or transactions.
    pub fn new() -> Self {
        Engine {
            accounts: DashMap::new(),
            transactions: TransactionQueue::new(),
        }
    }

    /// Processes a transaction, updating the appropriate client account.
    ///
    /// # Transaction Types
    ///
    /// | Type | Behavior |
    /// |------|----------|
    /// | Deposit | Creates account if needed, credits funds |
    /// | Withdrawal | Debits funds (fails if insufficient) |
    /// | Dispute | Holds deposit funds pending investigation |
    /// | Resolve | Releases held funds back to available |
    /// | Chargeback | Removes held funds, locks account |
    ///
    /// # Errors
    ///
    /// - [`TransactionError::DuplicateTransaction`] - Transaction ID already exists.
    /// - [`TransactionError::InsufficientFunds`] - Withdrawal exceeds available balance.
    /// - [`TransactionError::TransactionNotFound`] - Dispute references unknown transaction.
    /// - [`TransactionError::AlreadyDisputed`] - Deposit is already under dispute.
    /// - [`TransactionError::NotDisputed`] - Resolve/chargeback on non-disputed deposit.
    /// - [`TransactionError::AccountLocked`] - Account is frozen after chargeback.
    pub fn process(&self, transaction: TransactionType) -> Result<(), TransactionError> {
        let client_id = transaction.client_id();

        match &transaction {
            TransactionType::Deposit { .. } | TransactionType::Withdrawal { .. } => {
                // Store in transaction log first to validate unique tx_id.
                // This prevents duplicate transactions from being processed.
                let transaction_arc = Arc::new(transaction);
                self.transactions.push(Arc::clone(&transaction_arc))?;

                // Get existing account or create new one, then process the transaction.
                // New accounts start with zero balance.
                let mut account = self
                    .accounts
                    .entry(client_id)
                    .or_insert_with(|| Account::new(client_id));
                account.add_transaction(*transaction_arc)?;
            }
            TransactionType::Dispute { .. }
            | TransactionType::Resolve { .. }
            | TransactionType::Chargeback { .. } => {
                // Dispute operations reference existing deposits by transaction ID.
                // The account must exist (otherwise the referenced deposit can't exist).
                let mut account = self
                    .accounts
                    .get_mut(&client_id)
                    .ok_or(TransactionError::TransactionNotFound)?;
                account.add_transaction(transaction)?;
            }
        }

        Ok(())
    }

    /// Returns an iterator over all client accounts.
    ///
    /// Useful for generating output reports of account states.
    pub fn accounts(
        &self,
    ) -> impl Iterator<Item = dashmap::mapref::multiple::RefMulti<'_, ClientId, Account>> {
        self.accounts.iter()
    }

    /// Retrieves a client account by ID.
    ///
    /// Returns `None` if no account exists for the given client ID.
    pub fn get_account(
        &self,
        client_id: &ClientId,
    ) -> Option<dashmap::mapref::one::Ref<'_, ClientId, Account>> {
        self.accounts.get(client_id)
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
