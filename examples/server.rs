//! Simple REST API server example for the ledger engine.
//!
//! Run with: `cargo run --example server`
//!
//! ## Endpoints
//!
//! - `POST /transactions` - Create a transaction (deposit, withdrawal, dispute, resolve, chargeback)
//! - `GET /accounts` - List all accounts
//! - `GET /accounts/:id` - Get an account by client ID
//!
//! ## Example Usage
//!
//! ```bash
//! # Deposit
//! curl -X POST http://localhost:3000/transactions \
//!   -H "Content-Type: application/json" \
//!   -d '{"type": "deposit", "client_id": 1, "transaction_id": 1, "amount": "100.00"}'
//!
//! # Withdrawal
//! curl -X POST http://localhost:3000/transactions \
//!   -H "Content-Type: application/json" \
//!   -d '{"type": "withdrawal", "client_id": 1, "transaction_id": 2, "amount": "25.00"}'
//!
//! # Get account
//! curl http://localhost:3000/accounts/1
//!
//! # List all accounts
//! curl http://localhost:3000/accounts
//! ```

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ledger_demo_rs::{ClientId, Engine, TransactionError, TransactionId, TransactionType};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::net::TcpListener;

// === Request/Response DTOs ===

/// Request body for creating transactions.
///
/// Uses a tagged enum for clean JSON representation:
/// ```json
/// {"type": "deposit", "client_id": 1, "transaction_id": 1, "amount": "100.00"}
/// ```
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TransactionRequest {
    Deposit {
        client_id: u16,
        transaction_id: u32,
        amount: Decimal,
    },
    Withdrawal {
        client_id: u16,
        transaction_id: u32,
        amount: Decimal,
    },
    Dispute {
        client_id: u16,
        transaction_id: u32,
    },
    Resolve {
        client_id: u16,
        transaction_id: u32,
    },
    Chargeback {
        client_id: u16,
        transaction_id: u32,
    },
}

impl TransactionRequest {
    /// Converts the request DTO into the internal transaction type.
    fn into_transaction_type(self) -> TransactionType {
        match self {
            Self::Deposit {
                client_id,
                transaction_id,
                amount,
            } => TransactionType::Deposit {
                client_id: ClientId(client_id),
                transaction_id: TransactionId(transaction_id),
                amount,
            },
            Self::Withdrawal {
                client_id,
                transaction_id,
                amount,
            } => TransactionType::Withdrawal {
                client_id: ClientId(client_id),
                transaction_id: TransactionId(transaction_id),
                amount,
            },
            Self::Dispute {
                client_id,
                transaction_id,
            } => TransactionType::Dispute {
                client_id: ClientId(client_id),
                transaction_id: TransactionId(transaction_id),
            },
            Self::Resolve {
                client_id,
                transaction_id,
            } => TransactionType::Resolve {
                client_id: ClientId(client_id),
                transaction_id: TransactionId(transaction_id),
            },
            Self::Chargeback {
                client_id,
                transaction_id,
            } => TransactionType::Chargeback {
                client_id: ClientId(client_id),
                transaction_id: TransactionId(transaction_id),
            },
        }
    }
}

/// Response body for account information.
#[derive(Debug, Serialize)]
pub struct AccountResponse {
    pub client: u16,
    pub available: Decimal,
    pub held: Decimal,
    pub total: Decimal,
    pub locked: bool,
}

/// Response body for errors.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

// === Application State ===

/// Shared application state containing the ledger engine.
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
}

// === Error Handling ===

/// Wrapper for converting `TransactionError` into HTTP responses.
pub struct AppError(TransactionError);

impl From<TransactionError> for AppError {
    fn from(err: TransactionError) -> Self {
        AppError(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            TransactionError::MissingAmount => (StatusCode::BAD_REQUEST, "MISSING_AMOUNT"),
            TransactionError::InvalidAmount => (StatusCode::BAD_REQUEST, "INVALID_AMOUNT"),
            TransactionError::InsufficientFunds => {
                (StatusCode::UNPROCESSABLE_ENTITY, "INSUFFICIENT_FUNDS")
            }
            TransactionError::TransactionNotFound => {
                (StatusCode::NOT_FOUND, "TRANSACTION_NOT_FOUND")
            }
            TransactionError::ClientMismatch => (StatusCode::BAD_REQUEST, "CLIENT_MISMATCH"),
            TransactionError::AlreadyDisputed => (StatusCode::CONFLICT, "ALREADY_DISPUTED"),
            TransactionError::NotDisputed => (StatusCode::CONFLICT, "NOT_DISPUTED"),
            TransactionError::NotDisputable => (StatusCode::BAD_REQUEST, "NOT_DISPUTABLE"),
            TransactionError::DuplicateTransaction => {
                (StatusCode::CONFLICT, "DUPLICATE_TRANSACTION")
            }
            TransactionError::AccountLocked => (StatusCode::FORBIDDEN, "ACCOUNT_LOCKED"),
        };

        (
            status,
            Json(ErrorResponse {
                error: self.0.to_string(),
                code: code.to_string(),
            }),
        )
            .into_response()
    }
}

// === Handlers ===

/// POST /transactions - Create a new transaction.
async fn create_transaction(
    State(state): State<AppState>,
    Json(request): Json<TransactionRequest>,
) -> Result<StatusCode, AppError> {
    let tx = request.into_transaction_type();
    state.engine.process(tx)?;
    Ok(StatusCode::CREATED)
}

/// GET /accounts/:id - Get account by client ID.
async fn get_account(
    State(state): State<AppState>,
    Path(id): Path<u16>,
) -> Result<Json<AccountResponse>, (StatusCode, Json<ErrorResponse>)> {
    let client_id = ClientId(id);

    state
        .engine
        .get_account(&client_id)
        .map(|account| {
            Json(AccountResponse {
                client: client_id.0,
                available: account.available(),
                held: account.held(),
                total: account.total(),
                locked: account.locked(),
            })
        })
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Account not found".to_string(),
                    code: "ACCOUNT_NOT_FOUND".to_string(),
                }),
            )
        })
}

/// GET /accounts - List all accounts.
async fn list_accounts(State(state): State<AppState>) -> Json<Vec<AccountResponse>> {
    let accounts: Vec<AccountResponse> = state
        .engine
        .accounts()
        .map(|ref_multi| {
            let account = ref_multi.value();
            let client_id = *ref_multi.key();
            AccountResponse {
                client: client_id.0,
                available: account.available(),
                held: account.held(),
                total: account.total(),
                locked: account.locked(),
            }
        })
        .collect();

    Json(accounts)
}

// === Router ===

fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/transactions", post(create_transaction))
        .route("/accounts", get(list_accounts))
        .route("/accounts/{id}", get(get_account))
        .with_state(state)
}

// === Main ===

#[tokio::main]
async fn main() {
    let state = AppState {
        engine: Arc::new(Engine::new()),
    };

    let app = create_router(state);

    let listener = TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("Ledger API server running on http://127.0.0.1:3000");
    println!();
    println!("Endpoints:");
    println!("  POST /transactions  - Create a transaction");
    println!("  GET  /accounts      - List all accounts");
    println!("  GET  /accounts/:id  - Get account by ID");

    axum::serve(listener, app).await.unwrap();
}
