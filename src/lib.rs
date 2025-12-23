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

//! # Ledger Demo
//!
//! This library provides a payment processing engine for handling transactions
//! like deposits, withdrawals, and the dispute lifecycle (dispute, resolve, chargeback).
//!
//! ## Core Components
//!
//! - [`Engine`]: Central transaction processor managing client accounts
//! - [`Account`]: Client account with balance tracking and dispute handling
//! - [`TransactionType`]: Supported transaction types (deposit, withdrawal, etc.)
//! - [`TransactionError`]: Error types for transaction processing failures
//!
//! ## Example
//!
//! ```
//! use ledger_demo_rs::{Engine, ClientId, TransactionId, TransactionType, TransactionStatus};
//! use rust_decimal_macros::dec;
//!
//! let engine = Engine::new();
//!
//! // Process a deposit
//! let deposit = TransactionType::Deposit {
//!     client_id: ClientId(1),
//!     transaction_id: TransactionId(1),
//!     amount: dec!(100.00),
//!     status: TransactionStatus::Applied,
//! };
//! engine.process(deposit).unwrap();
//!
//! // Check account balance
//! let account = engine.get_account(&ClientId(1)).unwrap();
//! assert_eq!(account.available(), dec!(100.00));
//! ```
//!
//! ## Thread Safety
//!
//! The engine uses handles concurrent access to accounts, allowing multiple transactions to be
//! processed in parallel for different clients.

pub mod account;
mod base;
mod engine;
pub mod error;
mod transaction;
mod transaction_queue;

pub use account::Account;
pub use base::{ClientId, TransactionId};
pub use engine::Engine;
pub use error::TransactionError;
pub use transaction::{TransactionStatus, TransactionType};
pub use transaction_queue::TransactionQueue;
