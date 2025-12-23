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

//! Thread-safe transaction queue with deduplication.
//!
//! Provides a concurrent queue that ensures transaction ID uniqueness
//! while maintaining insertion order.

use crate::TransactionError;
use crate::base::TransactionId;
use crate::transaction::TransactionType;
use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use std::sync::Arc;

/// A thread-safe transaction queue with duplicate detection.
///
/// Combines a [`DashMap`] for O(1) duplicate checking with a [`SegQueue`]
/// to preserve insertion order. All operations are lock-free and safe
/// for concurrent access.
#[derive(Debug)]
pub struct TransactionQueue {
    /// Map of transaction IDs to transactions for O(1) duplicate detection.
    transactions: DashMap<TransactionId, Arc<TransactionType>>,

    /// Queue of transaction IDs maintaining FIFO order.
    transaction_ids: SegQueue<TransactionId>,
}

impl TransactionQueue {
    /// Creates a new empty transaction queue.
    pub fn new() -> Self {
        Self {
            transactions: DashMap::new(),
            transaction_ids: SegQueue::new(),
        }
    }

    /// Adds a transaction to the queue.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionError::DuplicateTransaction`] if a transaction
    /// with the same ID already exists in the queue.
    pub fn push(&self, transaction: Arc<TransactionType>) -> Result<(), TransactionError> {
        let transaction_id = transaction.id();

        // Use entry API for atomic check-and-insert to prevent race conditions
        match self.transactions.entry(transaction_id) {
            Entry::Occupied(_) => Err(TransactionError::DuplicateTransaction),
            Entry::Vacant(entry) => {
                entry.insert(transaction);
                self.transaction_ids.push(transaction_id);
                Ok(())
            }
        }
    }
}

impl Default for TransactionQueue {
    fn default() -> Self {
        Self::new()
    }
}
