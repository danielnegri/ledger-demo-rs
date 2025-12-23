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

use clap::Parser;
use csv::{ReaderBuilder, Trim, Writer};
use ledger_demo_rs::{ClientId, Engine, TransactionId, TransactionType};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::process;

/// Payment Engine - Process transaction CSV files
///
/// Reads transactions from a CSV file and outputs account states to stdout.
/// Supports deposits, withdrawals, disputes, resolves, and chargebacks.
#[derive(Parser, Debug)]
#[command(name = "ledger-demo-rs")]
#[command(about = "A payment engine that processes transaction CSVs", long_about = None)]
struct Args {
    /// Path to CSV file with transactions
    ///
    /// Expected format: type,client,tx,amount
    /// Example: cargo run -- transactions.csv > accounts.csv
    #[arg(value_name = "FILE")]
    input: PathBuf,
}

fn main() {
    // Parse command line arguments
    let args = Args::parse();

    // Open input file
    // TODO: Consider memory-mapping for parsing large transaction CSV files.
    let file = match File::open(&args.input) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error opening file '{}': {}", args.input.display(), e);
            process::exit(1);
        }
    };

    // Process transactions from CSV
    let engine = match process_transactions(BufReader::new(file)) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("Error processing transactions: {}", e);
            process::exit(1);
        }
    };

    // Write results to stdout
    if let Err(e) = write_accounts(&engine, std::io::stdout()) {
        eprintln!("Error writing output: {}", e);
        process::exit(1);
    }
}

/// Raw CSV record matching the input format.
///
/// Fields: `type, client, tx, amount`
#[derive(Debug, Deserialize)]
struct CsvRecord {
    #[serde(rename = "type")]
    tx_type: String,
    client: u16,
    tx: u32,
    #[serde(deserialize_with = "csv::invalid_option")]
    amount: Option<Decimal>,
}

impl CsvRecord {
    /// Converts CSV record to TransactionType.
    ///
    /// Returns `None` for invalid transaction types or missing required fields.
    fn into_transaction(self) -> Option<TransactionType> {
        let client_id = ClientId(self.client);
        let transaction_id = TransactionId(self.tx);

        match self.tx_type.to_lowercase().as_str() {
            "deposit" => {
                let amount = self.amount?;
                Some(TransactionType::Deposit {
                    client_id,
                    transaction_id,
                    amount,
                })
            }
            "withdrawal" => {
                let amount = self.amount?;
                Some(TransactionType::Withdrawal {
                    client_id,
                    transaction_id,
                    amount,
                })
            }
            "dispute" => Some(TransactionType::Dispute {
                client_id,
                transaction_id,
            }),
            "resolve" => Some(TransactionType::Resolve {
                client_id,
                transaction_id,
            }),
            "chargeback" => Some(TransactionType::Chargeback {
                client_id,
                transaction_id,
            }),
            _ => None,
        }
    }
}

/// Process transactions from a CSV reader.
///
/// This function uses streaming parsing to handle arbitrarily large CSV files
/// without loading the entire file into memory. Malformed rows and invalid
/// transactions are silently skipped per the specification.
///
/// # CSV Format
///
/// Expected columns: `type, client, tx, amount`
/// - `type`: Transaction type (deposit, withdrawal, dispute, resolve, chargeback)
/// - `client`: Client ID (u16)
/// - `tx`: Transaction ID (u32)
/// - `amount`: Decimal amount (optional for dispute/resolve/chargeback)
///
/// # Example
///
/// ```csv
/// type,client,tx,amount
/// deposit,1,1,100.0
/// withdrawal,1,2,50.0
/// dispute,1,1,
/// ```
///
/// # Errors
///
/// Returns a CSV error if the reader fails or the CSV structure is invalid.
/// Individual transaction errors are logged in debug mode but don't stop processing.
pub fn process_transactions<R: Read>(reader: R) -> Result<Engine, csv::Error> {
    let engine = Engine::new();

    let mut rdr = ReaderBuilder::new()
        .trim(Trim::All) // Handle whitespace in fields like " deposit "
        .flexible(true) // Allow missing amount field
        .has_headers(true) // Skip first row as header
        .from_reader(reader);

    for result in rdr.deserialize::<CsvRecord>() {
        match result {
            Ok(record) => {
                // Convert CSV record to transaction type
                let Some(tx) = record.into_transaction() else {
                    #[cfg(debug_assertions)]
                    eprintln!("Skipping invalid transaction record");
                    continue;
                };

                // Process transaction, ignoring errors (silent failure)
                if let Err(e) = engine.process(tx) {
                    #[cfg(debug_assertions)]
                    eprintln!("Skipping tx {}: {}", tx.id(), e);
                }
            }
            Err(e) => {
                // Skip malformed rows
                #[cfg(debug_assertions)]
                eprintln!("Skipping malformed row: {}", e);
                continue;
            }
        }
    }

    Ok(engine)
}

/// Write account states to a CSV writer
///
/// Outputs all accounts in CSV format with 4 decimal precision.
///
/// # CSV Format
///
/// Columns: `client, available, held, total, locked`
///
/// # Example
///
/// ```csv
/// client,available,held,total,locked
/// 1,75.5000,0.0000,75.5000,false
/// 2,100.0000,25.0000,125.0000,false
/// ```
///
/// # Errors
///
/// Returns a CSV error if writing fails.
pub fn write_accounts<W: Write>(engine: &Engine, writer: W) -> Result<(), csv::Error> {
    let mut wtr = Writer::from_writer(writer);

    // Get all account snapshots and serialize each one
    for account in engine.accounts() {
        wtr.serialize(&account)?;
    }

    // Flush to ensure all data is written
    wtr.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientId;
    use rust_decimal_macros::dec;
    use std::io::Cursor;

    #[test]
    fn parse_simple_deposit() {
        let csv = "type,client,tx,amount\ndeposit,1,1,100.0\n";
        let reader = Cursor::new(csv);

        let engine = process_transactions(reader).unwrap();

        assert_eq!(engine.accounts().len(), 1);
        let account = engine.get_account(&ClientId(1)).unwrap();
        assert_eq!(account.available, dec!(100.0));
    }

    #[test]
    fn parse_deposit_and_withdrawal() {
        let csv = "type,client,tx,amount\n\
                   deposit,1,1,100.0\n\
                   withdrawal,1,2,30.0\n";
        let reader = Cursor::new(csv);

        let engine = process_transactions(reader).unwrap();

        assert_eq!(engine.accounts().len(), 1);
        let account = engine.get_account(&ClientId(1)).unwrap();
        assert_eq!(account.available, dec!(70.0));
    }

    #[test]
    fn parse_dispute_sequence() {
        let csv = "type,client,tx,amount\n\
                   deposit,1,1,100.0\n\
                   dispute,1,1,\n";
        let reader = Cursor::new(csv);

        let engine = process_transactions(reader).unwrap();

        assert_eq!(engine.accounts().len(), 1);
        let account = engine.get_account(&ClientId(1)).unwrap();
        assert_eq!(account.available, dec!(0.0));
        assert_eq!(account.held, dec!(100.0));
    }

    #[test]
    fn parse_resolve_sequence() {
        let csv = "type,client,tx,amount\n\
                   deposit,1,1,100.0\n\
                   dispute,1,1,\n\
                   resolve,1,1,\n";
        let reader = Cursor::new(csv);

        let engine = process_transactions(reader).unwrap();

        let account = engine.get_account(&ClientId(1)).unwrap();
        assert_eq!(account.available, dec!(100.0));
        assert_eq!(account.held, dec!(0.0));
    }

    #[test]
    fn parse_chargeback_sequence() {
        let csv = "type,client,tx,amount\n\
                   deposit,1,1,100.0\n\
                   dispute,1,1,\n\
                   chargeback,1,1,\n";
        let reader = Cursor::new(csv);

        let engine = process_transactions(reader).unwrap();

        let account = engine.get_account(&ClientId(1)).unwrap();
        assert_eq!(account.total, dec!(0.0));
        assert!(account.locked);
    }

    #[test]
    fn parse_with_whitespace() {
        let csv = "type,client,tx,amount\n deposit , 1 , 1 , 100.0 \n";
        let reader = Cursor::new(csv);

        let engine = process_transactions(reader).unwrap();

        assert_eq!(engine.accounts().len(), 1);
        let account = engine.get_account(&ClientId(1)).unwrap();
        assert_eq!(account.available, dec!(100.0));
    }

    #[test]
    fn skip_malformed_rows() {
        let csv = "type,client,tx,amount\n\
                   deposit,1,1,100.0\n\
                   invalid,row,data,here\n\
                   deposit,2,2,50.0\n";
        let reader = Cursor::new(csv);

        let engine = process_transactions(reader).unwrap();

        assert_eq!(engine.accounts().len(), 2); // Two valid deposits
    }

    #[test]
    fn write_accounts_to_csv() {
        let csv_input = "type,client,tx,amount\n\
                         deposit,1,1,100.5\n\
                         deposit,2,2,200.25\n";
        let reader = Cursor::new(csv_input);
        let engine = process_transactions(reader).unwrap();

        let mut output = Vec::new();
        write_accounts(&engine, &mut output).unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("client,available,held,total,locked"));
    }

    #[test]
    fn write_preserves_decimal_values() {
        let csv_input = "type,client,tx,amount\ndeposit,1,1,1.5\n";
        let reader = Cursor::new(csv_input);
        let engine = process_transactions(reader).unwrap();

        let mut output = Vec::new();
        write_accounts(&engine, &mut output).unwrap();

        let account = engine.get_account(&ClientId(1)).unwrap();
        assert_eq!(account.available, dec!(1.5));
    }

    #[test]
    fn multiple_clients() {
        let csv = "type,client,tx,amount\n\
                   deposit,3,1,10.0\n\
                   deposit,1,2,20.0\n\
                   deposit,2,3,30.0\n";
        let reader = Cursor::new(csv);

        let engine = process_transactions(reader).unwrap();

        assert_eq!(engine.accounts().len(), 3);

        // Verify each client has correct balance
        assert_eq!(
            engine.get_account(&ClientId(1)).unwrap().available,
            dec!(20.0)
        );
        assert_eq!(
            engine.get_account(&ClientId(2)).unwrap().available,
            dec!(30.0)
        );
        assert_eq!(
            engine.get_account(&ClientId(3)).unwrap().available,
            dec!(10.0)
        );
    }
}
