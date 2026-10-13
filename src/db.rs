#![allow(dead_code)]
use chrono::Datelike;
use rusqlite::{Connection, OptionalExtension, Result};
use std::collections::HashMap;
use std::fs;

use directories::ProjectDirs;

use crate::models::{
    MonthlyBudget, MonthlyIncome, RecurringEntry, RecurringInterval, Tag, Transaction,
    TransactionType,
};

/// Initialize the database from a provided path. Useful for tests (`:memory:`) or custom locations.
pub fn init_db_with_path(path: &std::path::Path) -> Result<Connection> {
    println!("Database location: {:?}", path);

    let conn = Connection::open(path)?;

    // Create schema on first run if it doesn't exist yet
    conn.execute(
        "CREATE TABLE IF NOT EXISTS transactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            amount REAL NOT NULL,
            kind TEXT NOT NULL,
            tag TEXT NOT NULL,
            date TEXT NOT NULL
        )",
        [],
    )?;

    // Create recurring entries table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS recurring_entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            amount REAL NOT NULL,
            kind TEXT NOT NULL,
            tag TEXT NOT NULL,
            interval TEXT NOT NULL DEFAULT 'monthly',
            original_date TEXT NOT NULL,
            last_inserted_date TEXT NOT NULL DEFAULT '',
            active INTEGER NOT NULL DEFAULT 1
        )",
        [],
    )?;

    // Migrate existing recurring_entries table if it has old schema
    migrate_recurring_entries_schema(&conn)?;
    migrate_transactions_schema(&conn)?;
    migrate_transaction_month_edits_schema(&conn)?;

    migrate_budgets_schema(&conn)?;
    migrate_monthly_income_schema(&conn)?;
    migrate_income_budget_allocations_schema(&conn)?;

    Ok(conn)
}

/// Migrate existing transactions table to include bucket.
pub fn migrate_transactions_schema(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(transactions)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut has_bucket = false;
    let mut has_related_transaction_id = false;

    for row in rows {
        match row?.as_str() {
            "bucket" => has_bucket = true,
            "related_transaction_id" => has_related_transaction_id = true,
            _ => {}
        }
    }
    drop(stmt);

    if !has_bucket {
        conn.execute(
            "ALTER TABLE transactions ADD COLUMN bucket TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }

    if !has_related_transaction_id {
        conn.execute(
            "ALTER TABLE transactions ADD COLUMN related_transaction_id INTEGER",
            [],
        )?;
    }

    Ok(())
}

/// Ensure the monthly budgets table exists.
pub fn migrate_budgets_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS budgets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            month TEXT NOT NULL,
            bucket TEXT NOT NULL,
            budget REAL NOT NULL,
            spent_tracking INTEGER NOT NULL DEFAULT 1,
            remaining_override REAL,
            status TEXT NOT NULL DEFAULT 'confirmed',
            UNIQUE(month, bucket)
        )",
        [],
    )?;
    let mut stmt = conn.prepare("PRAGMA table_info(budgets)")?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

    let mut has_spent_override = false;
    for column in columns {
        if column? == "spent_override" {
            has_spent_override = true;
            break;
        }
    }
    drop(stmt);

    if !has_spent_override {
        conn.execute("ALTER TABLE budgets ADD COLUMN spent_override REAL", [])?;
    }

    Ok(())
}

/// Ensure the monthly income table exists.
pub fn migrate_income_budget_allocations_schema(conn: &Connection) -> Result<()> {
    conn.execute("CREATE TABLE IF NOT EXISTS income_budget_allocations (income_transaction_id INTEGER NOT NULL, month TEXT NOT NULL, bucket TEXT NOT NULL, amount REAL NOT NULL, PRIMARY KEY (income_transaction_id, month, bucket))", [])?;
    Ok(())
}

pub fn migrate_monthly_income_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS monthly_income (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            month TEXT NOT NULL UNIQUE,
            income REAL NOT NULL,
            status TEXT NOT NULL DEFAULT 'confirmed'
        )",
        [],
    )?;

    Ok(())
}

/// Read monthly budgets without modifying transaction history.
pub fn get_monthly_budgets(conn: &Connection, month: &str) -> Result<Vec<MonthlyBudget>> {
    let mut stmt = conn.prepare(
        "SELECT month, bucket, budget, spent_tracking, spent_override,
                remaining_override, status
         FROM budgets WHERE month = ?1 ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([month], |row| {
        Ok(MonthlyBudget {
            month: row.get(0)?,
            bucket: row.get(1)?,
            budget: row.get(2)?,
            spent_tracking: row.get::<_, i32>(3)? != 0,
            spent_override: row.get(4)?,
            remaining_override: row.get(5)?,
            status: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// Track transaction changes for each affected month.
fn migrate_transaction_month_edits_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS transaction_month_edits (
            month TEXT PRIMARY KEY,
            updated_at TEXT NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS transactions_month_edit_insert
        AFTER INSERT ON transactions BEGIN
            INSERT INTO transaction_month_edits(month, updated_at)
            VALUES (substr(NEW.date, 1, 7), datetime('now', 'localtime'))
            ON CONFLICT(month) DO UPDATE SET updated_at = excluded.updated_at;
        END;
        CREATE TRIGGER IF NOT EXISTS transactions_month_edit_delete
        AFTER DELETE ON transactions BEGIN
            INSERT INTO transaction_month_edits(month, updated_at)
            VALUES (substr(OLD.date, 1, 7), datetime('now', 'localtime'))
            ON CONFLICT(month) DO UPDATE SET updated_at = excluded.updated_at;
        END;
        CREATE TRIGGER IF NOT EXISTS transactions_month_edit_update
        AFTER UPDATE ON transactions BEGIN
            INSERT INTO transaction_month_edits(month, updated_at)
            VALUES (substr(OLD.date, 1, 7), datetime('now', 'localtime'))
            ON CONFLICT(month) DO UPDATE SET updated_at = excluded.updated_at;
            INSERT INTO transaction_month_edits(month, updated_at)
            VALUES (substr(NEW.date, 1, 7), datetime('now', 'localtime'))
            ON CONFLICT(month) DO UPDATE SET updated_at = excluded.updated_at;
        END;",
    )?;
    Ok(())
}

/// Read monthly income without modifying transaction history.
pub fn get_monthly_income(conn: &Connection, month: &str) -> Result<Option<MonthlyIncome>> {
    conn.query_row(
        "SELECT month, income, status,
                  COALESCE(
                      (SELECT strftime('%Y-%m-%d %H:%M', updated_at)
                       FROM transaction_month_edits
                       WHERE month = monthly_income.month),
                      'Never'
                  )
           FROM monthly_income WHERE month = ?1",
        [month],
        |row| {
            Ok(MonthlyIncome {
                month: row.get(0)?,
                income: row.get(1)?,
                status: row.get(2)?,
                last_edited: row.get(3)?,
            })
        },
    )
    .optional()
}

/// Initialize DB in-memory for tests.
pub fn init_in_memory() -> Result<Connection> {
    init_db_with_path(std::path::Path::new(":memory:"))
}

pub fn init_db() -> Result<Connection> {
    let proj_dirs =
        ProjectDirs::from("com", "ayan", "fitui").expect("Could not determine data directory");

    let data_dir = proj_dirs.data_dir();
    fs::create_dir_all(data_dir).expect("Failed to create data directory");

    let db_path = data_dir.join("budget.db");

    init_db_with_path(&db_path)
}

/// Migrate old recurring_entries table to new schema with interval and original_date columns
pub fn migrate_recurring_entries_schema(conn: &Connection) -> Result<()> {
    // First, check if the old last_inserted_month column exists
    let has_old_column = conn
        .prepare("SELECT last_inserted_month FROM recurring_entries LIMIT 1")
        .map(|_| true)
        .unwrap_or(false);

    // Check if the new last_inserted_date column exists
    let has_last_inserted_date = conn
        .prepare("SELECT last_inserted_date FROM recurring_entries LIMIT 1")
        .map(|_| true)
        .unwrap_or(false);

    // If old column exists but new one doesn't, add the new column and migrate data
    if has_old_column && !has_last_inserted_date {
        let _ = conn.execute(
            "ALTER TABLE recurring_entries ADD COLUMN last_inserted_date TEXT NOT NULL DEFAULT ''",
            [],
        );

        // Migrate last_inserted_month data to last_inserted_date
        let _ = conn.execute(
            "UPDATE recurring_entries SET last_inserted_date = COALESCE(last_inserted_month, '') WHERE 1=1",
            [],
        );
    } else if !has_last_inserted_date {
        // Only add the column if neither old nor new exists
        let _ = conn.execute(
            "ALTER TABLE recurring_entries ADD COLUMN last_inserted_date TEXT NOT NULL DEFAULT ''",
            [],
        );
    }

    // Check and add interval column if missing
    let has_interval = conn
        .prepare("SELECT interval FROM recurring_entries LIMIT 1")
        .map(|_| true)
        .unwrap_or(false);

    if !has_interval {
        let _ = conn.execute(
            "ALTER TABLE recurring_entries ADD COLUMN interval TEXT NOT NULL DEFAULT 'monthly'",
            [],
        );
    }

    // Check and add original_date column if missing
    let has_original_date = conn
        .prepare("SELECT original_date FROM recurring_entries LIMIT 1")
        .map(|_| true)
        .unwrap_or(false);

    if !has_original_date {
        let _ = conn.execute(
            "ALTER TABLE recurring_entries ADD COLUMN original_date TEXT NOT NULL DEFAULT ''",
            [],
        );
    }

    // Check and add active column if missing
    let has_active = conn
        .prepare("SELECT active FROM recurring_entries LIMIT 1")
        .map(|_| true)
        .unwrap_or(false);

    if !has_active {
        let _ = conn.execute(
            "ALTER TABLE recurring_entries ADD COLUMN active INTEGER NOT NULL DEFAULT 1",
            [],
        );
    }

    Ok(())
}

pub fn get_transactions(conn: &Connection) -> Result<Vec<Transaction>> {
    let mut stmt = conn.prepare(
        "SELECT id, source, amount, kind, tag, bucket, date, related_transaction_id
         FROM transactions
         ORDER BY date DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(Transaction {
            id: row.get(0)?,
            source: row.get(1)?,
            amount: row.get(2)?,

            // Stored as string in DB, converted back into enum
            kind: TransactionType::from_str(&row.get::<_, String>(3)?),

            // Tags are wrapped in your custom Tag type
            tag: Tag::from_str(&row.get::<_, String>(4)?),
            bucket: row.get(5)?,
            date: row.get(6)?,
            related_transaction_id: row.get(7)?,
        })
    })?;

    let mut transactions = Vec::new();
    for tx in rows {
        transactions.push(tx?);
    }

    Ok(transactions)
}

pub fn add_transaction(
    conn: &Connection,
    source: &str,
    amount: f64,
    kind: TransactionType,
    tag: &Tag,
    bucket: &str,
    date: &str,
) -> Result<()> {
    if kind != TransactionType::Credit {
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO transactions (source, amount, kind, tag, bucket, date)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (source, amount, kind.as_str(), tag.as_str(), bucket, date),
        )?;

        if kind == TransactionType::Debit && bucket == "Date" {
            let month = date
                .get(..7)
                .filter(|m| m.as_bytes().get(4) == Some(&b'-'))
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName("Invalid expense date".into())
                })?;

            let changed = tx.execute(
                "UPDATE budgets
                 SET remaining_override = CASE
                     WHEN remaining_override IS NULL THEN NULL
                     ELSE remaining_override - ?1
                 END
                 WHERE month = ?2 AND bucket = 'Date'",
                (amount, month),
            )?;

            if changed != 1 {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "Missing Date budget for {month}"
                )));
            }
        }

        tx.commit()?;
        return Ok(());
    }

    use rusqlite::Error;
    if !amount.is_finite() || amount <= 0.0 {
        return Err(Error::InvalidParameterName(
            "Income amount must be positive and finite".into(),
        ));
    }
    let month = date
        .get(..7)
        .filter(|m| m.as_bytes().get(4) == Some(&b"-"[0]))
        .ok_or_else(|| Error::InvalidParameterName("Invalid income date".into()))?;

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO transactions (source, amount, kind, tag, bucket, date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (source, amount, kind.as_str(), tag.as_str(), bucket, date),
    )?;
    let income_id = tx.last_insert_rowid() as i32;
    let allocations: Vec<(&str, f64)> = if bucket == "All" {
        vec![
            ("Personal", amount * 0.25),
            ("Saving", amount * 0.25),
            ("Travel", amount * 0.125),
            ("Baby", amount * 0.125),
            ("Electricity Baby", amount * 0.125),
            ("Date", amount * 0.125),
        ]
    } else {
        vec![(bucket, amount)]
    };
    for (name, allocated) in allocations {
        tx.execute(
            "INSERT INTO budgets (month, bucket, budget)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(month, bucket)
             DO UPDATE SET budget = budgets.budget + excluded.budget, remaining_override = CASE WHEN budgets.remaining_override IS NULL THEN NULL ELSE budgets.remaining_override + excluded.budget END",
            (month, name, allocated),
        )?;
        tx.execute(
            "INSERT INTO income_budget_allocations
             (income_transaction_id, month, bucket, amount)
             VALUES (?1, ?2, ?3, ?4)",
            (income_id, month, name, allocated),
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn add_transaction_with_waterfall(
    conn: &Connection,
    source: &str,
    amount: f64,
    kind: TransactionType,
    tag: &Tag,
    bucket: &str,
    date: &str,
) -> Result<()> {
    use rusqlite::Error;

    if !amount.is_finite() || amount <= 0.0 {
        return Err(Error::InvalidParameterName(
            "Expense amount must be positive and finite".into(),
        ));
    }

    if kind != TransactionType::Debit || !matches!(bucket, "Baby" | "Electricity Baby") {
        return add_transaction(conn, source, amount, kind, tag, bucket, date);
    }

    let month = date
        .get(..7)
        .filter(|m| m.as_bytes().get(4) == Some(&b'-'))
        .ok_or_else(|| Error::InvalidParameterName("Invalid transaction date".into()))?;

    let tx = conn.unchecked_transaction()?;
    let buckets: &[(&str, &str)] = match bucket {
        "Baby" => &[
            ("Baby", "Baby"),
            ("Electricity Baby", "Electricity Baby"),
            ("Date", "Date"),
        ],
        "Electricity Baby" => &[("Electricity Baby", "Electricity Baby"), ("Date", "Date")],
        _ => unreachable!("Waterfall source bucket validated above"),
    };
    let mut capacities = Vec::new();

    for &(display_bucket, db_bucket) in buckets {
        let budget_data: Option<(f64, Option<f64>)> = tx
            .query_row(
                "SELECT budget, spent_override FROM budgets
                 WHERE month = ?1 AND bucket = ?2",
                (month, db_bucket),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let (budget, spent_override) = budget_data.ok_or_else(|| {
            Error::InvalidParameterName(format!("Missing {db_bucket} budget for {month}"))
        })?;

        let spent: f64 = tx.query_row(
            "SELECT COALESCE(SUM(amount), 0)
             FROM transactions
             WHERE kind = 'debit' AND bucket = ?1 AND substr(date, 1, 7) = ?2
               AND (related_transaction_id IS NOT NULL OR NOT EXISTS (
                   SELECT 1 FROM transactions child
                   WHERE child.related_transaction_id = transactions.id
               ))",
            (db_bucket, month),
            |row| row.get(0),
        )?;

        let effective_spent = if matches!(display_bucket, "Baby" | "Electricity Baby") {
            spent_override
                .map(|baseline| baseline + spent)
                .unwrap_or(spent)
        } else {
            spent
        };

        capacities.push((display_bucket, (budget - effective_spent).max(0.0)));
    }

    let mut remaining = amount;
    let mut legs: Vec<(&str, f64)> = Vec::new();

    let last_bucket = buckets.last().map(|(name, _)| *name);

    for (name, capacity) in capacities {
        if remaining <= 0.0 {
            break;
        }

        // The final bucket (Date) absorbs all overflow.
        let allocated = if Some(name) == last_bucket {
            remaining
        } else {
            remaining.min(capacity)
        };

        if allocated > 0.0 {
            legs.push((name, allocated));
            remaining -= allocated;
        }
    }

    if remaining > 0.005 {
        return Err(Error::InvalidParameterName(format!(
            "Allocation incomplete: {} VND remains",
            crate::money::format_vnd(remaining)
        )));
    }

    let total: f64 = legs.iter().map(|(_, value)| value).sum();
    if (total - amount).abs() > 0.005 {
        return Err(Error::InvalidParameterName(
            "Allocation total mismatch".into(),
        ));
    }

    tx.execute(
        "INSERT INTO transactions
         (source, amount, kind, tag, bucket, date, related_transaction_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
        (source, amount, kind.as_str(), tag.as_str(), bucket, date),
    )?;
    let parent_id = tx.last_insert_rowid();

    for (leg_bucket, leg_amount) in legs {
        tx.execute(
            "INSERT INTO transactions
             (source, amount, kind, tag, bucket, date, related_transaction_id)
             VALUES (?1, ?2, 'debit', ?3, ?4, ?5, ?6)",
            (
                source,
                leg_amount,
                tag.as_str(),
                leg_bucket,
                date,
                parent_id,
            ),
        )?;

        if leg_bucket == "Date" {
            let changed = tx.execute(
                "UPDATE budgets
                 SET remaining_override = CASE
                     WHEN remaining_override IS NULL THEN NULL
                     ELSE remaining_override - ?1
                 END
                 WHERE month = ?2 AND bucket = 'Date'",
                (leg_amount, month),
            )?;
            if changed != 1 {
                return Err(Error::InvalidParameterName(format!(
                    "Missing Date budget for {month}"
                )));
            }
        }
    }

    tx.commit()?;
    Ok(())
}

fn reverse_income_allocations(conn: &Connection, income_id: i32) -> Result<()> {
    let allocations = {
        let mut stmt = conn.prepare(
            "SELECT month, bucket, amount FROM income_budget_allocations
             WHERE income_transaction_id = ?1",
        )?;
        let rows = stmt.query_map([income_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>>>()?
    };

    for (month, bucket, amount) in allocations {
        let changed = conn.execute(
            "UPDATE budgets SET budget = budget - ?1,
             remaining_override = CASE WHEN remaining_override IS NULL THEN NULL ELSE remaining_override - ?1 END
             WHERE month = ?2 AND bucket = ?3",
            (amount, &month, &bucket),
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Cannot reverse income allocation for {month}/{bucket}"
            )));
        }
    }

    conn.execute(
        "DELETE FROM income_budget_allocations WHERE income_transaction_id = ?1",
        [income_id],
    )?;
    Ok(())
}

pub fn delete_transaction(conn: &Connection, id: i32) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    let (kind, bucket, amount, date, parent_id): (String, String, f64, String, Option<i32>) = tx
        .query_row(
            "SELECT kind, bucket, amount, date, related_transaction_id
             FROM transactions WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;

    reverse_income_allocations(&tx, id)?;

    // Restore the Date override before deleting the expense or its allocation legs.
    if kind == "debit" && parent_id.is_none() {
        let date_amount: f64 = if bucket == "Date" {
            amount
        } else {
            tx.query_row(
                "SELECT COALESCE(SUM(amount), 0.0)
                 FROM transactions
                 WHERE related_transaction_id = ?1
                   AND kind = 'debit' AND bucket = 'Date'",
                [id],
                |row| row.get(0),
            )?
        };

        if date_amount > 0.0 {
            let month = date
                .get(..7)
                .filter(|m| m.as_bytes().get(4) == Some(&b'-'))
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName("Invalid transaction date".into())
                })?;

            let changed = tx.execute(
                "UPDATE budgets
                 SET remaining_override = CASE
                     WHEN remaining_override IS NULL THEN NULL
                     ELSE remaining_override + ?1
                 END
                 WHERE month = ?2 AND bucket = 'Date'",
                (date_amount, month),
            )?;

            if changed != 1 {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "Missing Date budget for {month}"
                )));
            }
        }
    }

    // Remove allocation legs first, then the parent.
    tx.execute(
        "DELETE FROM transactions WHERE related_transaction_id = ?1",
        [id],
    )?;
    tx.execute("DELETE FROM transactions WHERE id = ?1", [id])?;

    tx.commit()?;
    Ok(())
}

pub fn update_transaction(
    conn: &Connection,
    id: i32,
    source: &str,
    amount: f64,
    kind: TransactionType,
    tag: &Tag,
    bucket: &str,
    date: &str,
) -> Result<()> {
    if !amount.is_finite() || amount <= 0.0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "Transaction amount must be positive and finite".into(),
        ));
    }

    let tx = conn.unchecked_transaction()?;
    let existing: Option<(String, String, Option<i32>, f64, String)> = tx
        .query_row(
            "SELECT kind, bucket, related_transaction_id, amount, date
             FROM transactions WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;

    let (old_kind, old_bucket, parent_id, old_amount, old_date) = existing.ok_or_else(|| {
        rusqlite::Error::InvalidParameterName(format!("Transaction {id} not found"))
    })?;

    if parent_id.is_some() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Cannot edit an allocation leg directly".into(),
        ));
    }

    reverse_income_allocations(&tx, id)?;

    let has_legs: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM transactions WHERE related_transaction_id = ?1)",
        [id],
        |row| row.get(0),
    )?;

    // Restore the old Date allocation before changing the parent or its legs.
    let old_date_amount: f64 = if old_kind == "debit" && parent_id.is_none() {
        if old_bucket == "Date" {
            old_amount
        } else if has_legs {
            tx.query_row(
                "SELECT COALESCE(SUM(amount), 0.0)
                 FROM transactions
                 WHERE related_transaction_id = ?1
                   AND kind = 'debit' AND bucket = 'Date'",
                [id],
                |row| row.get(0),
            )?
        } else {
            0.0
        }
    } else {
        0.0
    };

    if old_date_amount > 0.0 {
        let old_month = old_date
            .get(..7)
            .filter(|m| m.as_bytes().get(4) == Some(&b'-'))
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("Invalid previous transaction date".into())
            })?;

        let changed = tx.execute(
            "UPDATE budgets
             SET remaining_override = CASE
                 WHEN remaining_override IS NULL THEN NULL
                 ELSE remaining_override + ?1
             END
             WHERE month = ?2 AND bucket = 'Date'",
            (old_date_amount, old_month),
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Missing Date budget for {old_month}"
            )));
        }
    }

    if has_legs
        && (kind != TransactionType::Debit || !matches!(bucket, "Baby" | "Electricity Baby"))
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "Cannot change a waterfall parent to a non-waterfall transaction".into(),
        ));
    }

    tx.execute(
        "UPDATE transactions
         SET source = ?1, amount = ?2, kind = ?3, tag = ?4, bucket = ?5, date = ?6
         WHERE id = ?7",
        (
            source,
            amount,
            kind.as_str(),
            tag.as_str(),
            bucket,
            date,
            id,
        ),
    )?;

    if has_legs {
        tx.execute(
            "DELETE FROM transactions WHERE related_transaction_id = ?1",
            [id],
        )?;
        let month = date
            .get(..7)
            .filter(|m| m.as_bytes().get(4) == Some(&b'-'))
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("Invalid transaction date".into())
            })?;

        let mut remaining = amount;
        let allocation_buckets: &[&str] = match bucket {
            "Baby" => &["Baby", "Electricity Baby", "Date"],
            "Electricity Baby" => &["Electricity Baby", "Date"],
            _ => unreachable!("Waterfall parent bucket validated above"),
        };

        for db_bucket in allocation_buckets {
            let budget_data: Option<(f64, Option<f64>)> = tx
                .query_row(
                    "SELECT budget, spent_override FROM budgets
                     WHERE month = ?1 AND bucket = ?2",
                    (month, db_bucket),
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (budget, spent_override) = budget_data.ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(format!(
                    "Missing {db_bucket} budget for {month}"
                ))
            })?;
            let spent: f64 = tx.query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM transactions
                 WHERE kind = 'debit' AND bucket = ?1 AND substr(date, 1, 7) = ?2
                 AND id != ?3
                 AND (related_transaction_id IS NOT NULL OR NOT EXISTS (
                     SELECT 1 FROM transactions child
                     WHERE child.related_transaction_id = transactions.id
                 ))",
                (db_bucket, month, id),
                |row| row.get(0),
            )?;
            let effective_spent = if matches!(*db_bucket, "Baby" | "Electricity Baby") {
                spent_override.unwrap_or(0.0) + spent
            } else {
                spent
            };
            let available = (budget - effective_spent).max(0.0);
            let allocated = if *db_bucket == "Date" {
                remaining
            } else {
                remaining.min(available)
            };
            if allocated > 0.0 {
                tx.execute(
                    "INSERT INTO transactions
                     (source, amount, kind, tag, bucket, date, related_transaction_id)
                     VALUES (?1, ?2, 'debit', ?3, ?4, ?5, ?6)",
                    (source, allocated, tag.as_str(), db_bucket, date, id),
                )?;

                if *db_bucket == "Date" {
                    let changed = tx.execute(
                        "UPDATE budgets
                         SET remaining_override = CASE
                             WHEN remaining_override IS NULL THEN NULL
                             ELSE remaining_override - ?1
                         END
                         WHERE month = ?2 AND bucket = 'Date'",
                        (allocated, month),
                    )?;
                    if changed != 1 {
                        return Err(rusqlite::Error::InvalidParameterName(format!(
                            "Missing Date budget for {month}"
                        )));
                    }
                }

                remaining -= allocated;
            }
            if remaining <= 0.005 {
                break;
            }
        }
        if remaining > 0.005 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Insufficient budget capacity: {} VND remains",
                crate::money::format_vnd(remaining)
            )));
        }
    } else if old_kind == "debit"
        && old_bucket == "Baby"
        && kind == TransactionType::Debit
        && bucket == "Baby"
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "Cannot safely convert this transaction to a waterfall allocation".into(),
        ));
    }

    // Ordinary Date expenses also change the override when edited.
    if !has_legs && kind == TransactionType::Debit && bucket == "Date" {
        let month = date
            .get(..7)
            .filter(|m| m.as_bytes().get(4) == Some(&b'-'))
            .ok_or_else(|| rusqlite::Error::InvalidParameterName("Invalid expense date".into()))?;

        let changed = tx.execute(
            "UPDATE budgets
             SET remaining_override = CASE
                 WHEN remaining_override IS NULL THEN NULL
                 ELSE remaining_override - ?1
             END
             WHERE month = ?2 AND bucket = 'Date'",
            (amount, month),
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Missing Date budget for {month}"
            )));
        }
    }

    if kind == TransactionType::Credit {
        let month = date
            .get(..7)
            .filter(|m| m.as_bytes().get(4) == Some(&b'-'))
            .ok_or_else(|| rusqlite::Error::InvalidParameterName("Invalid income date".into()))?;
        let allocations: Vec<(&str, f64)> = if bucket == "All" {
            vec![
                ("Personal", amount * 0.25),
                ("Saving", amount * 0.25),
                ("Travel", amount * 0.125),
                ("Baby", amount * 0.125),
                ("Electricity Baby", amount * 0.125),
                ("Date", amount * 0.125),
            ]
        } else {
            vec![(bucket, amount)]
        };
        for (name, allocated) in allocations {
            tx.execute(
"INSERT INTO budgets (month, bucket, budget)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(month, bucket)
                 DO UPDATE SET budget = budgets.budget + excluded.budget, remaining_override = CASE WHEN budgets.remaining_override IS NULL THEN NULL ELSE budgets.remaining_override + excluded.budget END",
(month, name, allocated),
)?;
            tx.execute(
                "INSERT INTO income_budget_allocations
                 (income_transaction_id, month, bucket, amount)
                 VALUES (?1, ?2, ?3, ?4)",
                (id, month, name, allocated),
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

pub fn total_earned(conn: &Connection) -> Result<f64> {
    conn.query_row(
        "SELECT COALESCE(SUM(amount), 0)
         FROM transactions
         WHERE kind = 'credit' AND related_transaction_id IS NULL",
        [],
        |row| row.get(0),
    )
}

pub fn total_spent(conn: &Connection) -> Result<f64> {
    conn.query_row(
        "SELECT COALESCE(SUM(amount), 0)
         FROM transactions
         WHERE kind = 'debit' AND related_transaction_id IS NULL",
        [],
        |row| row.get(0),
    )
}

pub fn spent_per_tag(conn: &Connection) -> Result<HashMap<Tag, f64>> {
    // Aggregate total spending grouped by tag
    let mut stmt = conn.prepare(
        "SELECT tag, COALESCE(SUM(amount), 0)
         FROM transactions
         WHERE kind = 'debit' AND related_transaction_id IS NULL
         GROUP BY tag",
    )?;

    let rows = stmt.query_map([], |row| {
        let tag_str: String = row.get(0)?;
        let total: f64 = row.get(1)?;

        Ok((Tag::from_str(&tag_str), total))
    })?;

    let mut map = HashMap::new();
    for r in rows {
        let (tag, total) = r?;
        map.insert(tag, total);
    }

    Ok(map)
}
// Recurring entry functions
pub fn get_recurring_entries(conn: &Connection) -> Result<Vec<RecurringEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, source, amount, kind, tag, interval, original_date, last_inserted_date, active
         FROM recurring_entries
         ORDER BY id DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(RecurringEntry {
            id: row.get(0)?,
            source: row.get(1)?,
            amount: row.get(2)?,
            kind: TransactionType::from_str(&row.get::<_, String>(3)?),
            tag: Tag::from_str(&row.get::<_, String>(4)?),
            interval: RecurringInterval::from_str(&row.get::<_, String>(5)?),
            original_date: row.get(6)?,
            last_inserted_date: row.get(7)?,
            active: row.get::<_, i32>(8)? != 0,
        })
    })?;

    let mut entries = Vec::new();
    for entry in rows {
        entries.push(entry?);
    }

    Ok(entries)
}

pub fn add_recurring_entry(
    conn: &Connection,
    source: &str,
    amount: f64,
    kind: TransactionType,
    tag: &Tag,
    interval: &RecurringInterval,
    original_date: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO recurring_entries (source, amount, kind, tag, interval, original_date, last_inserted_date, active)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        (
            source,
            amount,
            kind.as_str(),
            tag.as_str(),
            interval.as_str(),
            original_date,
            "", // Empty string indicates it hasn't been inserted yet
            1,
        ),
    )?;

    Ok(())
}

pub fn delete_recurring_entry(conn: &Connection, id: i32) -> Result<()> {
    conn.execute("DELETE FROM recurring_entries WHERE id = ?1", [id])?;
    Ok(())
}

pub fn toggle_recurring_entry(conn: &Connection, id: i32, active: bool) -> Result<()> {
    conn.execute(
        "UPDATE recurring_entries SET active = ?1 WHERE id = ?2",
        (if active { 1 } else { 0 }, id),
    )?;
    Ok(())
}

// Auto-insert recurring entries based on their interval
pub fn insert_recurring_transactions(conn: &Connection) -> Result<()> {
    let now = chrono::Local::now();
    let today_str = now.format("%Y-%m-%d").to_string();
    let current_week = format!("{:04}-W{:02}", now.year(), now.iso_week().week());
    let current_month = format!("{:04}-{:02}", now.year(), now.month());

    // Get all active recurring entries
    let mut stmt = conn.prepare(
        "SELECT id, source, amount, kind, tag, interval, original_date, last_inserted_date
         FROM recurring_entries
         WHERE active = 1",
    )?;

    let entries: Vec<_> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Process each recurring entry
    for (rec_id, source, amount, kind, tag, interval_str, original_date, last_inserted_date) in
        entries
    {
        let interval = RecurringInterval::from_str(&interval_str);
        let kind_enum = TransactionType::from_str(&kind);
        let tag_obj = Tag::from_str(&tag);

        let should_insert = match interval {
            RecurringInterval::Daily => {
                // Insert if we haven't inserted today
                last_inserted_date != today_str
            }
            RecurringInterval::Weekly => {
                // Extract day of week from original date
                if let Ok(original_ndt) =
                    chrono::NaiveDate::parse_from_str(&original_date, "%Y-%m-%d")
                {
                    let original_dow = original_ndt.weekday();
                    let today_dow = now.weekday();

                    // Check if this is the same day of the week and hasn't been inserted this week
                    original_dow == today_dow && last_inserted_date != current_week
                } else {
                    false
                }
            }
            RecurringInterval::Monthly => {
                // Extract day of month from original date
                if let Ok(original_ndt) =
                    chrono::NaiveDate::parse_from_str(&original_date, "%Y-%m-%d")
                {
                    let original_day = original_ndt.day();
                    let today_day = now.day();

                    // Check if this is the same day of month and hasn't been inserted this month
                    original_day == today_day && last_inserted_date != current_month
                } else {
                    false
                }
            }
        };

        if should_insert {
            // Insert as a transaction with today's date
            add_transaction(conn, &source, amount, kind_enum, &tag_obj, "", &today_str)?;

            // Update the last_inserted_date based on interval
            let new_last_inserted = match interval {
                RecurringInterval::Daily => today_str.clone(),
                RecurringInterval::Weekly => current_week.clone(),
                RecurringInterval::Monthly => current_month.clone(),
            };

            conn.execute(
                "UPDATE recurring_entries SET last_inserted_date = ?1 WHERE id = ?2",
                (new_last_inserted, rec_id),
            )?;
        }
    }

    Ok(())
}

// Keep the old function name for backwards compatibility
pub fn insert_recurring_for_month(conn: &Connection, _current_month: &str) -> Result<()> {
    insert_recurring_transactions(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::{RecurringInterval, Tag, TransactionType};

    fn setup_conn() -> Connection {
        init_in_memory().expect("failed to init in-memory db")
    }

    #[test]
    fn totals_are_calculated() {
        let conn = setup_conn();

        add_transaction(
            &conn,
            "pay",
            100.0,
            TransactionType::Credit,
            &Tag::from_str("salary"),
            "Baby",
            "2026-02-23",
        )
        .unwrap();
        add_transaction(
            &conn,
            "buy",
            40.0,
            TransactionType::Debit,
            &Tag::from_str("food"),
            "Baby",
            "2026-02-23",
        )
        .unwrap();

        let earned = total_earned(&conn).unwrap();
        let spent = total_spent(&conn).unwrap();

        assert_eq!(earned, 100.0);
        assert_eq!(spent, 40.0);

        let per_tag = spent_per_tag(&conn).unwrap();
        assert_eq!(
            per_tag.get(&Tag::from_str("food")).copied().unwrap_or(0.0),
            40.0
        );
    }

    #[test]
    fn recurring_roundtrip() {
        let conn = setup_conn();

        add_recurring_entry(
            &conn,
            "rent",
            500.0,
            TransactionType::Debit,
            &Tag::from_str("housing"),
            &RecurringInterval::Monthly,
            "2026-02-23",
        )
        .unwrap();

        let entries = get_recurring_entries(&conn).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, "rent");
        assert_eq!(entries[0].amount, 500.0);
        assert_eq!(entries[0].interval, RecurringInterval::Monthly);
    }

    #[test]
    fn monthly_budget_queries_roundtrip() {
        let conn = setup_conn();

        conn.execute(
            "INSERT INTO budgets
             (month, bucket, budget, spent_tracking, remaining_override, status)
             VALUES ('2026-10', 'Baby', 487500, 1, 0, 'confirmed')",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO monthly_income (month, income, status)
             VALUES ('2026-10', 4700000, 'confirmed')",
            [],
        )
        .unwrap();

        let budgets = get_monthly_budgets(&conn, "2026-10").unwrap();
        assert_eq!(budgets.len(), 1);
        assert_eq!(budgets[0].bucket, "Baby");
        assert_eq!(budgets[0].budget, 487500.0);

        let income = get_monthly_income(&conn, "2026-10")
            .unwrap()
            .expect("income should exist");
        assert_eq!(income.income, 4700000.0);

        assert!(get_monthly_budgets(&conn, "2026-11").unwrap().is_empty());
        assert!(get_monthly_income(&conn, "2026-11").unwrap().is_none());
    }
}

/// Returns archived historical spending, or an error if no archive exists.
pub fn get_archived_total_spent(conn: &Connection) -> Result<f64> {
    conn.query_row(
        "SELECT value FROM stats_archive WHERE key = 'total_spent'",
        [],
        |row| row.get(0),
    )
}
