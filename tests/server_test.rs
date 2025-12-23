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

//! Integration tests for the REST API server with concurrent requests.
//!
//! These tests verify that the server correctly handles thousands of
//! concurrent requests while maintaining data consistency.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ledger_demo_rs::{ClientId, Engine, TransactionError, TransactionId, TransactionType};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use tokio::net::TcpListener;

// === DTOs (duplicated from example for test isolation) ===

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountResponse {
    pub client: u16,
    pub available: Decimal,
    pub held: Decimal,
    pub total: Decimal,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

// === Server Setup ===

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
}

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

async fn create_transaction(
    State(state): State<AppState>,
    Json(request): Json<TransactionRequest>,
) -> Result<StatusCode, AppError> {
    let tx = request.into_transaction_type();
    state.engine.process(tx)?;
    Ok(StatusCode::CREATED)
}

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

fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/transactions", post(create_transaction))
        .route("/accounts", get(list_accounts))
        .route("/accounts/{id}", get(get_account))
        .with_state(state)
}

/// Test server that binds to an ephemeral port.
struct TestServer {
    base_url: String,
    engine: Arc<Engine>,
}

impl TestServer {
    async fn new() -> Self {
        let engine = Arc::new(Engine::new());
        let state = AppState {
            engine: engine.clone(),
        };

        let app = create_router(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{}", addr);

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Wait for server to be ready by polling with retries
        let client = Client::new();
        let health_url = format!("{}/accounts", base_url);
        for _ in 0..50 {
            match client.get(&health_url).send().await {
                Ok(_) => break,
                Err(_) => tokio::time::sleep(tokio::time::Duration::from_millis(50)).await,
            }
        }

        TestServer { base_url, engine }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

// === Tests ===
// These tests are ignored in CI due to connection issues on some platforms.
// Run manually with: cargo test --test server_test -- --ignored

/// Test concurrent deposits to different clients.
/// Each client should have exactly the sum of their deposits.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn concurrent_deposits_to_multiple_clients() {
    let server = TestServer::new().await;
    let client = Client::new();

    const NUM_CLIENTS: u16 = 50;
    const DEPOSITS_PER_CLIENT: u32 = 20;
    const AMOUNT_PER_DEPOSIT: &str = "10.00";
    const BATCH_SIZE: usize = 100; // Limit concurrent connections

    let tx_counter = Arc::new(AtomicU32::new(1));
    let start = Instant::now();

    let total_requests = (NUM_CLIENTS as usize) * (DEPOSITS_PER_CLIENT as usize);
    let mut successful = 0usize;

    // Process in batches to avoid exhausting ephemeral ports
    let mut all_requests: Vec<(u16, u32)> = Vec::with_capacity(total_requests);
    for client_id in 1..=NUM_CLIENTS {
        for _ in 0..DEPOSITS_PER_CLIENT {
            let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);
            all_requests.push((client_id, tx_id));
        }
    }

    for batch in all_requests.chunks(BATCH_SIZE) {
        let mut handles = Vec::with_capacity(batch.len());

        for &(client_id, tx_id) in batch {
            let client = client.clone();
            let url = server.url("/transactions");

            let handle = tokio::spawn(async move {
                let request = TransactionRequest::Deposit {
                    client_id,
                    transaction_id: tx_id,
                    amount: AMOUNT_PER_DEPOSIT.parse().unwrap(),
                };

                let response = client.post(&url).json(&request).send().await.unwrap();
                response.status()
            });

            handles.push(handle);
        }

        let results: Vec<_> = futures::future::join_all(handles).await;
        successful += results
            .iter()
            .filter(|r| r.as_ref().unwrap().is_success())
            .count();
    }

    let elapsed = start.elapsed();

    println!(
        "Processed {} requests in {:?} ({:.0} req/s)",
        total_requests,
        elapsed,
        total_requests as f64 / elapsed.as_secs_f64()
    );

    assert_eq!(successful, total_requests, "All deposits should succeed");

    // Verify each client has the correct balance
    let expected_balance: Decimal =
        AMOUNT_PER_DEPOSIT.parse::<Decimal>().unwrap() * Decimal::from(DEPOSITS_PER_CLIENT);

    for client_id in 1..=NUM_CLIENTS {
        let account = server.engine.get_account(&ClientId(client_id)).unwrap();
        assert_eq!(
            account.total(),
            expected_balance,
            "Client {} should have {} total",
            client_id,
            expected_balance
        );
        assert_eq!(account.available(), expected_balance);
        assert_eq!(account.held(), Decimal::ZERO);
        assert!(!account.locked());
    }
}

/// Test concurrent deposits to a single client.
/// The total should be exactly the sum of all deposits.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn concurrent_deposits_single_client() {
    let server = TestServer::new().await;
    let client = Client::new();

    const NUM_DEPOSITS: u32 = 1000;
    const AMOUNT_PER_DEPOSIT: &str = "1.50";

    let tx_counter = Arc::new(AtomicU32::new(1));
    let start = Instant::now();

    let mut handles = Vec::with_capacity(NUM_DEPOSITS as usize);

    for _ in 0..NUM_DEPOSITS {
        let client = client.clone();
        let url = server.url("/transactions");
        let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);

        let handle = tokio::spawn(async move {
            let request = TransactionRequest::Deposit {
                client_id: 1,
                transaction_id: tx_id,
                amount: AMOUNT_PER_DEPOSIT.parse().unwrap(),
            };

            let response = client.post(&url).json(&request).send().await.unwrap();
            response.status()
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let elapsed = start.elapsed();

    let successful = results
        .iter()
        .filter(|r| r.as_ref().unwrap().is_success())
        .count();

    println!(
        "Single client: {} deposits in {:?} ({:.0} req/s)",
        NUM_DEPOSITS,
        elapsed,
        NUM_DEPOSITS as f64 / elapsed.as_secs_f64()
    );

    assert_eq!(successful, NUM_DEPOSITS as usize);

    let expected_balance: Decimal =
        AMOUNT_PER_DEPOSIT.parse::<Decimal>().unwrap() * Decimal::from(NUM_DEPOSITS);

    let account = server.engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.total(), expected_balance);
}

/// Test that duplicate transaction IDs are rejected.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn concurrent_duplicate_transactions_rejected() {
    let server = TestServer::new().await;
    let client = Client::new();

    const NUM_DUPLICATES: usize = 100;
    const TX_ID: u32 = 999;

    let mut handles = Vec::with_capacity(NUM_DUPLICATES);

    for _ in 0..NUM_DUPLICATES {
        let client = client.clone();
        let url = server.url("/transactions");

        let handle = tokio::spawn(async move {
            let request = TransactionRequest::Deposit {
                client_id: 1,
                transaction_id: TX_ID,
                amount: "100.00".parse().unwrap(),
            };

            let response = client.post(&url).json(&request).send().await.unwrap();
            response.status()
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    let successful = results
        .iter()
        .filter(|r| *r.as_ref().unwrap() == StatusCode::CREATED)
        .count();
    let conflicts = results
        .iter()
        .filter(|r| *r.as_ref().unwrap() == StatusCode::CONFLICT)
        .count();

    // Exactly one should succeed, the rest should be conflicts
    assert_eq!(successful, 1, "Exactly one duplicate should succeed");
    assert_eq!(conflicts, NUM_DUPLICATES - 1, "Others should be conflicts");

    // Verify balance reflects only one deposit
    let account = server.engine.get_account(&ClientId(1)).unwrap();
    assert_eq!(account.total(), Decimal::new(10000, 2)); // 100.00
}

/// Test concurrent deposits and withdrawals to the same client.
/// Final balance should never go negative.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn concurrent_deposits_and_withdrawals() {
    let server = TestServer::new().await;
    let client = Client::new();

    // First, make an initial deposit to ensure there are funds
    let initial_deposit = TransactionRequest::Deposit {
        client_id: 1,
        transaction_id: 1,
        amount: "10000.00".parse().unwrap(),
    };
    let response = client
        .post(server.url("/transactions"))
        .json(&initial_deposit)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    const NUM_OPS: u32 = 500;
    let tx_counter = Arc::new(AtomicU32::new(2)); // Start from 2 since 1 is used

    let mut handles = Vec::with_capacity((NUM_OPS * 2) as usize);

    // Alternate deposits and withdrawals
    for i in 0..NUM_OPS {
        let client = client.clone();
        let url = server.url("/transactions");
        let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);

        let is_deposit = i % 2 == 0;
        let handle = tokio::spawn(async move {
            let request = if is_deposit {
                TransactionRequest::Deposit {
                    client_id: 1,
                    transaction_id: tx_id,
                    amount: "10.00".parse().unwrap(),
                }
            } else {
                TransactionRequest::Withdrawal {
                    client_id: 1,
                    transaction_id: tx_id,
                    amount: "5.00".parse().unwrap(),
                }
            };

            let response = client.post(&url).json(&request).send().await.unwrap();
            (is_deposit, response.status())
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    let deposit_success = results
        .iter()
        .filter(|r| {
            let (is_deposit, status) = r.as_ref().unwrap();
            *is_deposit && status.is_success()
        })
        .count();
    let withdrawal_success = results
        .iter()
        .filter(|r| {
            let (is_deposit, status) = r.as_ref().unwrap();
            !*is_deposit && status.is_success()
        })
        .count();

    println!(
        "Deposits succeeded: {}, Withdrawals succeeded: {}",
        deposit_success, withdrawal_success
    );

    // Verify balance is correct and non-negative
    let account = server.engine.get_account(&ClientId(1)).unwrap();
    assert!(
        account.available() >= Decimal::ZERO,
        "Available balance should never be negative"
    );
    assert!(
        account.total() >= Decimal::ZERO,
        "Total balance should never be negative"
    );

    // Expected: 10000 + (250 * 10) - (withdrawals * 5)
    // Some withdrawals may fail if they race, so we verify the math is consistent
    let expected_deposits: Decimal =
        Decimal::new(1000000, 2) + Decimal::new(10, 0) * Decimal::from(deposit_success as u32);
    let expected_withdrawals: Decimal =
        Decimal::new(5, 0) * Decimal::from(withdrawal_success as u32);
    let expected_balance = expected_deposits - expected_withdrawals;

    assert_eq!(
        account.total(),
        expected_balance,
        "Balance should match successful operations"
    );
}

/// Test concurrent dispute, resolve, and chargeback operations.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn concurrent_dispute_lifecycle() {
    let server = TestServer::new().await;
    let client = Client::new();

    // Create deposits for multiple clients
    const NUM_CLIENTS: u16 = 50;
    let mut tx_id: u32 = 1;

    for client_id in 1..=NUM_CLIENTS {
        let request = TransactionRequest::Deposit {
            client_id,
            transaction_id: tx_id,
            amount: "1000.00".parse().unwrap(),
        };
        tx_id += 1;

        let response = client
            .post(server.url("/transactions"))
            .json(&request)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    // Now dispute all deposits concurrently
    let mut handles = Vec::with_capacity(NUM_CLIENTS as usize);
    for client_id in 1..=NUM_CLIENTS {
        let client = client.clone();
        let url = server.url("/transactions");
        let dispute_tx_id = client_id as u32; // Transaction IDs match client IDs in this test

        let handle = tokio::spawn(async move {
            let request = TransactionRequest::Dispute {
                client_id,
                transaction_id: dispute_tx_id,
            };
            let response = client.post(&url).json(&request).send().await.unwrap();
            response.status()
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let successful_disputes = results
        .iter()
        .filter(|r| r.as_ref().unwrap().is_success())
        .count();
    assert_eq!(
        successful_disputes, NUM_CLIENTS as usize,
        "All disputes should succeed"
    );

    // Verify all accounts have funds held
    for client_id in 1..=NUM_CLIENTS {
        let account = server.engine.get_account(&ClientId(client_id)).unwrap();
        assert_eq!(account.available(), Decimal::ZERO);
        assert_eq!(account.held(), Decimal::new(100000, 2)); // 1000.00
        assert_eq!(account.total(), Decimal::new(100000, 2));
    }

    // Resolve half, chargeback the other half concurrently
    let mut handles = Vec::with_capacity(NUM_CLIENTS as usize);
    for client_id in 1..=NUM_CLIENTS {
        let client = client.clone();
        let url = server.url("/transactions");
        let tx_id_to_reference = client_id as u32;
        let should_resolve = client_id % 2 == 0;

        let handle = tokio::spawn(async move {
            let request = if should_resolve {
                TransactionRequest::Resolve {
                    client_id,
                    transaction_id: tx_id_to_reference,
                }
            } else {
                TransactionRequest::Chargeback {
                    client_id,
                    transaction_id: tx_id_to_reference,
                }
            };
            let response = client.post(&url).json(&request).send().await.unwrap();
            (should_resolve, response.status())
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    // Verify all operations succeeded
    for result in &results {
        let (_, status) = result.as_ref().unwrap();
        assert!(status.is_success(), "All resolve/chargeback should succeed");
    }

    // Verify final states
    for client_id in 1..=NUM_CLIENTS {
        let account = server.engine.get_account(&ClientId(client_id)).unwrap();

        if client_id % 2 == 0 {
            // Resolved - funds back to available
            assert_eq!(account.available(), Decimal::new(100000, 2));
            assert_eq!(account.held(), Decimal::ZERO);
            assert!(!account.locked());
        } else {
            // Chargebacked - funds removed, account locked
            assert_eq!(account.available(), Decimal::ZERO);
            assert_eq!(account.held(), Decimal::ZERO);
            assert_eq!(account.total(), Decimal::ZERO);
            assert!(account.locked());
        }
    }
}

/// Stress test with thousands of mixed operations.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn stress_test_mixed_operations() {
    let server = TestServer::new().await;
    let client = Client::new();

    const NUM_CLIENTS: u16 = 50;
    const OPS_PER_CLIENT: u32 = 100;
    const TOTAL_OPS: usize = (NUM_CLIENTS as usize) * (OPS_PER_CLIENT as usize);

    let tx_counter = Arc::new(AtomicU32::new(1));
    let start = Instant::now();

    let mut handles = Vec::with_capacity(TOTAL_OPS);

    for client_id in 1..=NUM_CLIENTS {
        for op in 0..OPS_PER_CLIENT {
            let client = client.clone();
            let url = server.url("/transactions");
            let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);

            let handle = tokio::spawn(async move {
                // Mostly deposits with some withdrawals
                let request = if op % 5 == 0 && op > 0 {
                    TransactionRequest::Withdrawal {
                        client_id,
                        transaction_id: tx_id,
                        amount: "5.00".parse().unwrap(),
                    }
                } else {
                    TransactionRequest::Deposit {
                        client_id,
                        transaction_id: tx_id,
                        amount: "10.00".parse().unwrap(),
                    }
                };

                let response = client.post(&url).json(&request).send().await.unwrap();
                response.status()
            });

            handles.push(handle);
        }
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let elapsed = start.elapsed();

    let successful = results
        .iter()
        .filter(|r| r.as_ref().unwrap().is_success())
        .count();

    println!(
        "Stress test: {} operations in {:?} ({:.0} req/s)",
        TOTAL_OPS,
        elapsed,
        TOTAL_OPS as f64 / elapsed.as_secs_f64()
    );

    // Most operations should succeed (some withdrawals may fail)
    assert!(
        successful > TOTAL_OPS * 80 / 100,
        "At least 80% of operations should succeed"
    );

    // Verify all accounts are in a valid state
    for client_id in 1..=NUM_CLIENTS {
        let account = server.engine.get_account(&ClientId(client_id)).unwrap();
        assert!(account.available() >= Decimal::ZERO);
        assert!(account.held() >= Decimal::ZERO);
        assert_eq!(account.total(), account.available() + account.held());
    }
}

/// Test concurrent GET requests while processing transactions.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn concurrent_reads_and_writes() {
    let server = TestServer::new().await;
    let client = Client::new();

    const NUM_WRITES: u32 = 500;
    const NUM_READS: u32 = 500;

    let tx_counter = Arc::new(AtomicU32::new(1));
    let start = Instant::now();

    let mut handles = Vec::with_capacity((NUM_WRITES + NUM_READS) as usize);

    // Spawn write operations
    for client_id in 1..=10u16 {
        for _ in 0..(NUM_WRITES / 10) {
            let client = client.clone();
            let url = server.url("/transactions");
            let tx_id = tx_counter.fetch_add(1, Ordering::SeqCst);

            let handle = tokio::spawn(async move {
                let request = TransactionRequest::Deposit {
                    client_id,
                    transaction_id: tx_id,
                    amount: "1.00".parse().unwrap(),
                };
                let response = client.post(&url).json(&request).send().await.unwrap();
                ("write", response.status())
            });

            handles.push(handle);
        }
    }

    // Spawn read operations
    for _ in 0..NUM_READS {
        let client = client.clone();
        let url = server.url("/accounts");

        let handle = tokio::spawn(async move {
            let response = client.get(&url).send().await.unwrap();
            ("read", response.status())
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let elapsed = start.elapsed();

    let write_success = results
        .iter()
        .filter(|r| {
            let (op, status) = r.as_ref().unwrap();
            *op == "write" && status.is_success()
        })
        .count();
    let read_success = results
        .iter()
        .filter(|r| {
            let (op, status) = r.as_ref().unwrap();
            *op == "read" && status.is_success()
        })
        .count();

    println!(
        "Concurrent reads/writes: {} writes, {} reads in {:?}",
        write_success, read_success, elapsed
    );

    assert_eq!(write_success, NUM_WRITES as usize);
    assert_eq!(read_success, NUM_READS as usize);
}

/// Test that the list accounts endpoint returns correct data under load.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn list_accounts_under_load() {
    let server = TestServer::new().await;
    let client = Client::new();

    const NUM_CLIENTS: u16 = 100;

    // Create accounts
    let mut tx_id: u32 = 1;
    for client_id in 1..=NUM_CLIENTS {
        let request = TransactionRequest::Deposit {
            client_id,
            transaction_id: tx_id,
            amount: format!("{}.00", client_id).parse().unwrap(),
        };
        tx_id += 1;

        let response = client
            .post(server.url("/transactions"))
            .json(&request)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    // Fetch all accounts
    let response = client.get(server.url("/accounts")).send().await.unwrap();
    assert!(response.status().is_success());

    let accounts: Vec<AccountResponse> = response.json().await.unwrap();
    assert_eq!(accounts.len(), NUM_CLIENTS as usize);

    // Verify totals
    let total_balance: Decimal = accounts.iter().map(|a| a.total).sum();
    let expected_total: Decimal = (1..=NUM_CLIENTS).map(Decimal::from).sum();
    assert_eq!(total_balance, expected_total);
}

/// Test getting individual accounts concurrently.
#[tokio::test]
#[ignore = "requires running server, may fail in CI"]
async fn concurrent_get_individual_accounts() {
    let server = TestServer::new().await;
    let client = Client::new();

    const NUM_CLIENTS: u16 = 50;

    // Create accounts with specific balances
    for client_id in 1..=NUM_CLIENTS {
        let request = TransactionRequest::Deposit {
            client_id,
            transaction_id: client_id as u32,
            amount: format!("{}.00", client_id * 10).parse().unwrap(),
        };

        let response = client
            .post(server.url("/transactions"))
            .json(&request)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    // Fetch all accounts concurrently multiple times
    const READS_PER_ACCOUNT: usize = 20;
    let mut handles = Vec::with_capacity((NUM_CLIENTS as usize) * READS_PER_ACCOUNT);

    for client_id in 1..=NUM_CLIENTS {
        for _ in 0..READS_PER_ACCOUNT {
            let client = client.clone();
            let url = server.url(&format!("/accounts/{}", client_id));
            let expected_balance = Decimal::from(client_id * 10);

            let handle = tokio::spawn(async move {
                let response = client.get(&url).send().await.unwrap();
                assert!(response.status().is_success());

                let account: AccountResponse = response.json().await.unwrap();
                assert_eq!(account.client, client_id);
                assert_eq!(account.total, expected_balance);
                true
            });

            handles.push(handle);
        }
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let successful = results.iter().filter(|r| *r.as_ref().unwrap()).count();
    assert_eq!(successful, (NUM_CLIENTS as usize) * READS_PER_ACCOUNT);
}
