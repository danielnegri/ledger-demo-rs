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

//! Transaction management.
//!
//! Transactions follow a state machine:
//! - [`Applied`] → [`Inflight`] (via dispute)
//! - [`Inflight`] → [`Resolved`] (via resolve) or [`Voided`] (via chargeback)

use crate::base::{ClientId, TransactionId};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransactionType {
    Deposit {
        client_id: ClientId,
        transaction_id: TransactionId,
        amount: Decimal,
        status: TransactionStatus,
    },
    Withdrawal {
        client_id: ClientId,
        transaction_id: TransactionId,
        amount: Decimal,
    },
    Dispute {
        client_id: ClientId,
        transaction_id: TransactionId,
    },
    Resolve {
        client_id: ClientId,
        transaction_id: TransactionId,
    },
    Chargeback {
        client_id: ClientId,
        transaction_id: TransactionId,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransactionStatus {
    Applied,
    Inflight,
    Resolved,
    Voided,
}

impl TransactionType {
    pub fn id(&self) -> TransactionId {
        match self {
            Self::Deposit { transaction_id, .. } => *transaction_id,
            Self::Withdrawal { transaction_id, .. } => *transaction_id,
            Self::Dispute { transaction_id, .. } => *transaction_id,
            Self::Resolve { transaction_id, .. } => *transaction_id,
            Self::Chargeback { transaction_id, .. } => *transaction_id,
        }
    }

    pub fn client_id(&self) -> ClientId {
        match self {
            Self::Deposit { client_id, .. } => *client_id,
            Self::Withdrawal { client_id, .. } => *client_id,
            Self::Dispute { client_id, .. } => *client_id,
            Self::Resolve { client_id, .. } => *client_id,
            Self::Chargeback { client_id, .. } => *client_id,
        }
    }

    pub fn amount(&self) -> Decimal {
        match self {
            Self::Deposit { amount, .. } => *amount,
            Self::Withdrawal { amount, .. } => *amount,
            _ => Decimal::ZERO,
        }
    }

    pub fn status(&self) -> TransactionStatus {
        match self {
            Self::Deposit { status, .. } => *status,
            _ => TransactionStatus::Applied,
        }
    }
}
