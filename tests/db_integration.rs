use chrono::Datelike;

use FiTui::{
    db,
    models::{RecurringInterval, Tag, TransactionType},
};

#[test]
fn full_transaction_lifecycle() {
    let conn = db::init_in_memory().expect("init in-memory");

    // Add
    db::add_transaction(
        &conn,
        "pay",
        123.45,
        TransactionType::Credit,
        &Tag::from_str("salary"),
        "Baby",
        "2026-02-23",
    )
    .unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(txs.len(), 1);

    let id = txs[0].id;

    // Update
    db::update_transaction(
        &conn,
        id,
        "pay-2",
        200.0,
        TransactionType::Credit,
        &Tag::from_str("salary"),
        "Baby",
        "2026-02-24",
    )
    .unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(txs[0].source, "pay-2");
    assert_eq!(txs[0].amount, 200.0);

    // Delete
    db::delete_transaction(&conn, id).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert!(txs.is_empty());
}

#[test]
fn income_allocates_budgets_and_reverses_on_delete() {
    let conn = db::init_in_memory().expect("init db");
    let tag = Tag::from_str("salary");

    db::add_transaction(
        &conn,
        "salary 1",
        10_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-10-05",
    )
    .unwrap();

    let expected = [
        ("Personal", 2_500_000.0),
        ("Saving", 2_500_000.0),
        ("Travel", 1_250_000.0),
        ("Baby", 1_250_000.0),
        ("Electricity Baby", 1_250_000.0),
        ("Date", 1_250_000.0),
    ];
    for (bucket, amount) in expected {
        let actual: f64 = conn
            .query_row(
                "SELECT COALESCE((SELECT budget FROM budgets WHERE month = ?1 AND bucket = ?2), 0.0)",
                ("2026-10", bucket),
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            (actual - amount).abs() < 0.01,
            "wrong allocation for {bucket}: {actual}"
        );
    }
    assert_eq!(db::total_earned(&conn).unwrap(), 10_000_000.0);

    db::add_transaction(
        &conn,
        "salary 2",
        2_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-10-15",
    )
    .unwrap();
    let personal: f64 = conn
        .query_row(
            "SELECT COALESCE((SELECT budget FROM budgets WHERE month = ?1 AND bucket = ?2), 0.0)",
            ("2026-10", "Personal"),
            |row| row.get(0),
        )
        .unwrap();
    assert!((personal - 3_000_000.0).abs() < 0.01);

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions WHERE source = ?1",
            ["salary 1"],
            |row| row.get(0),
        )
        .unwrap();
    db::delete_transaction(&conn, id).unwrap();

    let personal: f64 = conn
        .query_row(
            "SELECT COALESCE((SELECT budget FROM budgets WHERE month = ?1 AND bucket = ?2), 0.0)",
            ("2026-10", "Personal"),
            |row| row.get(0),
        )
        .unwrap();
    assert!((personal - 500_000.0).abs() < 0.01);
    assert_eq!(db::total_earned(&conn).unwrap(), 2_000_000.0);
}

#[test]
fn recurring_insertion_simulation() {
    let conn = db::init_in_memory().expect("init in-memory");

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let current_month = format!(
        "{:04}-{:02}",
        chrono::Local::now().year(),
        chrono::Local::now().month()
    );

    // Add a monthly recurring entry starting today
    db::add_recurring_entry(
        &conn,
        "rent",
        500.0,
        TransactionType::Debit,
        &Tag::from_str("housing"),
        &RecurringInterval::Monthly,
        &today,
    )
    .unwrap();

    // Run insert logic
    db::insert_recurring_transactions(&conn).unwrap();

    // A transaction should be created for today
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].date, today);
    assert_eq!(txs[0].source, "rent");

    // Recurring entry's last_inserted_date should be updated to current month
    let entries = db::get_recurring_entries(&conn).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].last_inserted_date, current_month);
}

#[test]
fn recurring_intervals_behavior() {
    let conn = db::init_in_memory().expect("init in-memory");

    let now = chrono::Local::now();
    let today = now.format("%Y-%m-%d").to_string();

    // Helper to reset last_inserted_date for a recurring entry (simulate time passing)
    let reset_last_inserted = |id: i32, reset_val: &str| {
        conn.execute(
            "UPDATE recurring_entries SET last_inserted_date = ?1 WHERE id = ?2",
            (reset_val, id),
        )
        .unwrap();
    };

    // === TEST DAILY ===
    db::add_recurring_entry(
        &conn,
        "daily-item",
        10.0,
        TransactionType::Debit,
        &Tag::from_str("test"),
        &RecurringInterval::Daily,
        &today,
    )
    .unwrap();
    let daily_entries = db::get_recurring_entries(&conn).unwrap();
    let daily_id = daily_entries
        .iter()
        .find(|e| e.source == "daily-item")
        .unwrap()
        .id;

    // First run: should insert
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(txs.len(), 1);

    // Second run same day: should NOT insert (already inserted today)
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(txs.len(), 1, "daily should not insert twice on same day");

    // Simulate next day by resetting last_inserted_date
    reset_last_inserted(daily_id, ""); // Clear the date as if it's a new day
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(txs.len(), 2, "daily should insert on new day");

    // === TEST WEEKLY ===
    let daily_txs_count = db::get_transactions(&conn).unwrap().len();

    db::add_recurring_entry(
        &conn,
        "weekly-item",
        20.0,
        TransactionType::Debit,
        &Tag::from_str("test"),
        &RecurringInterval::Weekly,
        &today,
    )
    .unwrap();
    let weekly_entries = db::get_recurring_entries(&conn).unwrap();
    let weekly_id = weekly_entries
        .iter()
        .find(|e| e.source == "weekly-item")
        .unwrap()
        .id;

    // First run: should insert
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(
        txs.len(),
        daily_txs_count + 1,
        "weekly should insert on matching day"
    );

    // Second run same week: should NOT insert
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(
        txs.len(),
        daily_txs_count + 1,
        "weekly should not insert twice in same week"
    );

    // Simulate next week by resetting (in real app, time would change, but we're testing the state machine)
    reset_last_inserted(weekly_id, "");
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(
        txs.len(),
        daily_txs_count + 2,
        "weekly should insert next week"
    );

    // === TEST MONTHLY ===
    let weekly_txs_count = db::get_transactions(&conn).unwrap().len();

    db::add_recurring_entry(
        &conn,
        "monthly-item",
        30.0,
        TransactionType::Debit,
        &Tag::from_str("test"),
        &RecurringInterval::Monthly,
        &today,
    )
    .unwrap();
    let monthly_entries = db::get_recurring_entries(&conn).unwrap();
    let monthly_id = monthly_entries
        .iter()
        .find(|e| e.source == "monthly-item")
        .unwrap()
        .id;

    // First run: should insert
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(
        txs.len(),
        weekly_txs_count + 1,
        "monthly should insert on matching day"
    );

    // Second run same month: should NOT insert
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(
        txs.len(),
        weekly_txs_count + 1,
        "monthly should not insert twice in same month"
    );

    // Simulate next month by resetting last_inserted_date
    reset_last_inserted(monthly_id, "");
    db::insert_recurring_transactions(&conn).unwrap();
    let txs = db::get_transactions(&conn).unwrap();
    assert_eq!(
        txs.len(),
        weekly_txs_count + 2,
        "monthly should insert next month"
    );
}

#[test]
fn migration_safety_test() {
    // Create a raw in-memory connection and craft an old-schema recurring_entries table
    let conn = rusqlite::Connection::open_in_memory().expect("open in memory");

    conn.execute(
        "CREATE TABLE recurring_entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            amount REAL NOT NULL,
            kind TEXT NOT NULL,
            tag TEXT NOT NULL,
            last_inserted_month TEXT NOT NULL DEFAULT ''
        )",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO recurring_entries (source, amount, kind, tag, last_inserted_month) VALUES ('x', 10.0, 'debit', 'a', '2026-02')",
        [],
    ).unwrap();

    // Run migration
    db::migrate_recurring_entries_schema(&conn).unwrap();

    // After migration, last_inserted_date should exist and be migrated
    let migrated: String = conn
        .query_row(
            "SELECT last_inserted_date FROM recurring_entries WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(migrated, "2026-02");

    // New columns interval, original_date, and active should exist (no error when selecting)
    let _: String = conn
        .query_row(
            "SELECT interval FROM recurring_entries WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let _: String = conn
        .query_row(
            "SELECT original_date FROM recurring_entries WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let active: i32 = conn
        .query_row(
            "SELECT active FROM recurring_entries WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(active == 0 || active == 1);
}

#[test]
fn waterfall_persists_lineage_and_exact_allocation() {
    let conn = db::init_in_memory().expect("init in-memory");

    for (bucket, budget) in [
        ("Baby", 100.0),
        ("Electricity Baby", 200.0),
        ("Date", 300.0),
    ] {
        conn.execute(
            "INSERT INTO budgets (month, bucket, budget) VALUES (?1, ?2, ?3)",
            ("2026-02", bucket, budget),
        )
        .unwrap();
    }

    db::add_transaction_with_waterfall(
        &conn,
        "test expense",
        250.0,
        TransactionType::Debit,
        &Tag::from_str("test"),
        "Baby",
        "2026-02-23",
    )
    .expect("waterfall should succeed");

    let parent_id: i32 = conn
        .query_row(
            "SELECT id FROM transactions WHERE related_transaction_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let parent_amount: f64 = conn
        .query_row(
            "SELECT amount FROM transactions WHERE id = ?1",
            [parent_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(parent_amount, 250.0);

    let child_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE related_transaction_id = ?1",
            [parent_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(child_count, 2);

    let baby_leg: f64 = conn
        .query_row(
            "SELECT amount FROM transactions WHERE related_transaction_id = ?1 AND bucket = ?2",
            rusqlite::params![parent_id, "Baby"],
            |row| row.get(0),
        )
        .unwrap();
    let electric_leg: f64 = conn
        .query_row(
            "SELECT amount FROM transactions WHERE related_transaction_id = ?1 AND bucket = ?2",
            rusqlite::params![parent_id, "Electricity Baby"],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(baby_leg, 100.0);
    assert_eq!(electric_leg, 150.0);
    assert_eq!(baby_leg + electric_leg, parent_amount);
}

#[test]
fn waterfall_uses_spent_override_for_baby_capacities() {
    let conn = db::init_in_memory().expect("init in-memory");

    for (bucket, budget, spent_override) in [
        ("Baby", 100.0, Some(100.0)),
        ("Electricity Baby", 100.0, Some(100.0)),
        ("Date", 300.0, None),
    ] {
        conn.execute(
            "INSERT INTO budgets (month, bucket, budget, spent_override)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["2026-02", bucket, budget, spent_override],
        )
        .unwrap();
    }

    db::add_transaction_with_waterfall(
        &conn,
        "override exhausted baby",
        100.0,
        TransactionType::Debit,
        &Tag::from_str("test"),
        "Baby",
        "2026-02-23",
    )
    .expect("waterfall should route the full amount to Date");

    let legs: Vec<(String, f64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT bucket, amount FROM transactions
                 WHERE related_transaction_id = (
                     SELECT id FROM transactions
                     WHERE source = 'override exhausted baby'
                       AND related_transaction_id IS NULL
                 )
                 ORDER BY id",
            )
            .unwrap();

        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };

    assert_eq!(legs, vec![("Date".to_string(), 100.0)]);
}

#[test]
fn electricity_baby_waterfall_persists_date_leg() {
    let conn = db::init_in_memory().expect("init in-memory");

    for (bucket, budget) in [("Electricity Baby", 100.0), ("Date", 200.0)] {
        conn.execute(
            "INSERT INTO budgets (month, bucket, budget) VALUES (?1, ?2, ?3)",
            ("2026-02", bucket, budget),
        )
        .unwrap();
    }

    db::add_transaction_with_waterfall(
        &conn,
        "electricity baby expense",
        250.0,
        TransactionType::Debit,
        &Tag::from_str("test"),
        "Electricity Baby",
        "2026-02-23",
    )
    .expect("waterfall should succeed");

    let parent_id: i32 = conn
        .query_row(
            "SELECT id FROM transactions WHERE related_transaction_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let legs: Vec<(String, f64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT bucket, amount FROM transactions
                 WHERE related_transaction_id = ?1 ORDER BY id",
            )
            .unwrap();

        stmt.query_map([parent_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };

    assert_eq!(
        legs,
        vec![
            ("Electricity Baby".to_string(), 100.0),
            ("Date".to_string(), 150.0),
        ]
    );
    assert_eq!(legs.iter().map(|(_, amount)| amount).sum::<f64>(), 250.0);
}

#[test]
fn waterfall_date_absorbs_overflow_and_persists_exact_allocation() {
    let conn = db::init_in_memory().expect("init in-memory");

    for bucket in ["Baby", "Electricity Baby", "Date"] {
        conn.execute(
            "INSERT INTO budgets (month, bucket, budget) VALUES (?1, ?2, ?3)",
            ("2026-02", bucket, 100.0),
        )
        .unwrap();
    }

    db::add_transaction_with_waterfall(
        &conn,
        "date overflow",
        301.0,
        TransactionType::Debit,
        &Tag::from_str("test"),
        "Baby",
        "2026-02-23",
    )
    .expect("Date should absorb overflow beyond its budget");

    let parent_id: i32 = conn
        .query_row(
            "SELECT id FROM transactions
             WHERE source = 'date overflow' AND related_transaction_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let legs: Vec<(String, f64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT bucket, amount FROM transactions
                 WHERE related_transaction_id = ?1 ORDER BY id",
            )
            .unwrap();

        stmt.query_map([parent_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };

    assert_eq!(
        legs,
        vec![
            ("Baby".to_string(), 100.0),
            ("Electricity Baby".to_string(), 100.0),
            ("Date".to_string(), 101.0),
        ]
    );
    assert_eq!(legs.iter().map(|(_, amount)| amount).sum::<f64>(), 301.0);

    let date_spent: f64 = conn
        .query_row(
            "SELECT SUM(amount) FROM transactions
             WHERE kind = 'debit' AND bucket = 'Date'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(date_spent, 101.0);
}

#[test]
fn income_update_reverses_old_month_and_allocates_new_amount() {
    let conn = db::init_in_memory().expect("init db");
    let tag = Tag::from_str("salary");

    db::add_transaction(
        &conn,
        "salary",
        10_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-10-05",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions WHERE source = ?1",
            ["salary"],
            |row| row.get(0),
        )
        .unwrap();

    db::update_transaction(
        &conn,
        id,
        "salary revised",
        6_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-11-05",
    )
    .unwrap();

    let expected = [
        ("Personal", 1_500_000.0),
        ("Saving", 1_500_000.0),
        ("Travel", 750_000.0),
        ("Baby", 750_000.0),
        ("Electricity Baby", 750_000.0),
        ("Date", 750_000.0),
    ];

    for (bucket, amount) in expected {
        let old_month: f64 = conn
            .query_row(
                "SELECT COALESCE((SELECT budget FROM budgets WHERE month = ?1 AND bucket = ?2), 0.0)",
                ("2026-10", bucket),
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            (old_month - 0.0).abs() < 0.01,
            "October {bucket} should be reversed, got {old_month}"
        );

        let new_month: f64 = conn
            .query_row(
                "SELECT COALESCE((SELECT budget FROM budgets WHERE month = ?1 AND bucket = ?2), 0.0)",
                ("2026-11", bucket),
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            (new_month - amount).abs() < 0.01,
            "November {bucket}: expected {amount}, got {new_month}"
        );
    }

    assert_eq!(db::total_earned(&conn).unwrap(), 6_000_000.0);
}

#[test]
fn income_update_to_expense_reverses_allocations() {
    let conn = db::init_in_memory().expect("init db");
    let tag = Tag::from_str("salary");

    db::add_transaction(
        &conn,
        "salary",
        8_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-12-05",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions WHERE source = ?1",
            ["salary"],
            |row| row.get(0),
        )
        .unwrap();

    db::update_transaction(
        &conn,
        id,
        "converted expense",
        1_000_000.0,
        TransactionType::Debit,
        &tag,
        "Personal",
        "2026-12-06",
    )
    .unwrap();

    let buckets = [
        "Personal",
        "Saving",
        "Travel",
        "Baby",
        "Electricity Baby",
        "Date",
    ];
    for bucket in buckets {
        let budget: f64 = conn
            .query_row(
                "SELECT COALESCE((SELECT budget FROM budgets WHERE month = ?1 AND bucket = ?2), 0.0)",
                ("2026-12", bucket),
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            budget.abs() < 0.01,
            "{bucket} should return to zero after Income becomes Expense, got {budget}"
        );
    }

    assert_eq!(db::total_earned(&conn).unwrap(), 0.0);
}

#[test]
fn income_repeated_updates_keep_allocations_correct() {
    let conn = db::init_in_memory().expect("init db");
    let tag = Tag::from_str("salary");

    db::add_transaction(
        &conn,
        "salary",
        4_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-12-05",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions WHERE source = ?1",
            ["salary"],
            |row| row.get(0),
        )
        .unwrap();

    db::update_transaction(
        &conn,
        id,
        "salary revised",
        8_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-12-05",
    )
    .unwrap();
    db::update_transaction(
        &conn,
        id,
        "salary revised again",
        2_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-12-05",
    )
    .unwrap();

    let expected = [
        ("Personal", 500_000.0),
        ("Saving", 500_000.0),
        ("Travel", 250_000.0),
        ("Baby", 250_000.0),
        ("Electricity Baby", 250_000.0),
        ("Date", 250_000.0),
    ];

    for (bucket, amount) in expected {
        let budget: f64 = conn
            .query_row(
                "SELECT COALESCE((SELECT budget FROM budgets WHERE month = ?1 AND bucket = ?2), 0.0)",
                ("2026-12", bucket),
                |row| row.get(0),
            )
            .unwrap();

        assert!(
            (budget - amount).abs() < 0.01,
            "{bucket}: expected {amount}, got {budget}"
        );
    }

    assert_eq!(db::total_earned(&conn).unwrap(), 2_000_000.0);
}

#[test]
fn income_delete_failure_preserves_transaction_and_allocations() {
    let conn = db::init_in_memory().expect("init db");
    let tag = Tag::from_str("salary");

    db::add_transaction(
        &conn,
        "salary",
        4_000_000.0,
        TransactionType::Credit,
        &tag,
        "All",
        "2026-12-05",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions WHERE source = ?1",
            ["salary"],
            |row| row.get(0),
        )
        .unwrap();

    conn.execute_batch("CREATE TRIGGER reject_income_delete BEFORE DELETE ON transactions WHEN OLD.source = 'salary' BEGIN SELECT RAISE(ABORT, 'forced delete failure'); END;").unwrap();

    assert!(db::delete_transaction(&conn, id).is_err());

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "Income transaction must remain after failed delete"
    );

    let expected = [
        ("Personal", 1_000_000.0),
        ("Saving", 1_000_000.0),
        ("Travel", 500_000.0),
        ("Baby", 500_000.0),
        ("Electricity Baby", 500_000.0),
        ("Date", 500_000.0),
    ];

    for (bucket, amount) in expected {
        let budget: f64 = conn
            .query_row(
                "SELECT COALESCE((SELECT budget FROM budgets WHERE month = ?1 AND bucket = ?2), 0.0)",
                ("2026-12", bucket),
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            (budget - amount).abs() < 0.01,
            "{bucket}: expected {amount}, got {budget}"
        );
    }

    assert_eq!(db::total_earned(&conn).unwrap(), 4_000_000.0);
}

#[test]
fn waterfall_update_reallocates_and_preserves_total() {
    let conn = db::init_in_memory().expect("init in-memory");
    for (bucket, budget) in [
        ("Baby", 100.0),
        ("Electricity Baby", 100.0),
        ("Date", 300.0),
    ] {
        conn.execute(
            "INSERT INTO budgets (month, bucket, budget) VALUES (?1, ?2, ?3)",
            ("2026-02", bucket, budget),
        )
        .unwrap();
    }

    let tag = Tag::from_str("test");
    db::add_transaction_with_waterfall(
        &conn,
        "edit waterfall",
        150.0,
        TransactionType::Debit,
        &tag,
        "Baby",
        "2026-02-23",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions
         WHERE source = 'edit waterfall' AND related_transaction_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();

    db::update_transaction(
        &conn,
        id,
        "edit waterfall",
        250.0,
        TransactionType::Debit,
        &tag,
        "Baby",
        "2026-02-23",
    )
    .expect("update should succeed");

    let (parent_amount, child_total): (f64, f64) = conn
        .query_row(
            "SELECT p.amount,
         (SELECT COALESCE(SUM(c.amount), 0) FROM transactions c
          WHERE c.related_transaction_id = p.id)
         FROM transactions p WHERE p.id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    assert_eq!(parent_amount, 250.0);
    assert_eq!(child_total, parent_amount);
}

#[test]
fn waterfall_update_respects_spent_override() {
    let conn = db::init_in_memory().expect("init in-memory");
    for (bucket, budget, spent_override) in [
        ("Baby", 100.0, Some(100.0)),
        ("Electricity Baby", 100.0, Some(100.0)),
        ("Date", 300.0, None),
    ] {
        conn.execute(
            "INSERT INTO budgets (month, bucket, budget, spent_override)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["2026-02", bucket, budget, spent_override],
        )
        .unwrap();
    }

    let tag = Tag::from_str("test");
    db::add_transaction_with_waterfall(
        &conn,
        "override edit",
        50.0,
        TransactionType::Debit,
        &tag,
        "Baby",
        "2026-02-23",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions
         WHERE source = 'override edit' AND related_transaction_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();

    db::update_transaction(
        &conn,
        id,
        "override edit",
        80.0,
        TransactionType::Debit,
        &tag,
        "Baby",
        "2026-02-23",
    )
    .expect("update should route to Date");

    let baby_total: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM transactions
         WHERE kind = 'debit' AND bucket = 'Baby'
         AND substr(date, 1, 7) = '2026-02'
         AND (related_transaction_id IS NOT NULL OR NOT EXISTS (
             SELECT 1 FROM transactions c
             WHERE c.related_transaction_id = transactions.id
         ))",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(baby_total, 0.0);
}

#[test]
fn waterfall_update_failure_rolls_back_parent_and_legs() {
    let conn = db::init_in_memory().expect("init in-memory");
    for (bucket, budget) in [("Baby", 50.0), ("Electricity Baby", 50.0), ("Date", 50.0)] {
        conn.execute(
            "INSERT INTO budgets (month, bucket, budget) VALUES (?1, ?2, ?3)",
            ("2026-02", bucket, budget),
        )
        .unwrap();
    }

    let tag = Tag::from_str("test");
    db::add_transaction_with_waterfall(
        &conn,
        "rollback edit",
        100.0,
        TransactionType::Debit,
        &tag,
        "Baby",
        "2026-02-23",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions
         WHERE source = 'rollback edit' AND related_transaction_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let before: Vec<(String, f64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT bucket, amount FROM transactions
             WHERE related_transaction_id = ?1 ORDER BY id",
            )
            .unwrap();
        stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };

    // Force a database error after the parent and legs have been modified
    // inside the transaction, so rollback can be verified deterministically.
    conn.execute(
        "DELETE FROM budgets WHERE month = ?1 AND bucket = ?2",
        ("2026-02", "Date"),
    )
    .unwrap();

    assert!(
        db::update_transaction(
            &conn,
            id,
            "rollback edit changed",
            200.0,
            TransactionType::Debit,
            &tag,
            "Baby",
            "2026-02-23",
        )
        .is_err()
    );

    let parent: (String, f64) = conn
        .query_row(
            "SELECT source, amount FROM transactions WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(parent, ("rollback edit".to_string(), 100.0));

    let after: Vec<(String, f64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT bucket, amount FROM transactions
             WHERE related_transaction_id = ?1 ORDER BY id",
            )
            .unwrap();
        stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(after, before);
}

#[test]
fn date_expense_update_adjusts_remaining_override() {
    let conn = db::init_in_memory().expect("init in-memory");
    conn.execute(
        "INSERT INTO budgets (month, bucket, budget, remaining_override)
         VALUES ('2026-02', 'Date', 500.0, 200.0)",
        [],
    )
    .unwrap();

    let tag = Tag::from_str("test");
    db::add_transaction(
        &conn,
        "date edit",
        40.0,
        TransactionType::Debit,
        &tag,
        "Date",
        "2026-02-10",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions WHERE source = 'date edit'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    db::update_transaction(
        &conn,
        id,
        "date edit",
        70.0,
        TransactionType::Debit,
        &tag,
        "Date",
        "2026-02-10",
    )
    .unwrap();

    let remaining: f64 = conn
        .query_row(
            "SELECT remaining_override FROM budgets
         WHERE month = '2026-02' AND bucket = 'Date'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(remaining, 130.0);
}

#[test]
fn waterfall_update_across_months_restores_old_date_and_debits_new_date() {
    let conn = db::init_in_memory().expect("init in-memory");
    for (month, date_remaining) in [("2026-02", 200.0), ("2026-03", 300.0)] {
        for (bucket, budget) in [("Baby", 30.0), ("Electricity Baby", 20.0), ("Date", 500.0)] {
            conn.execute(
                "INSERT INTO budgets (month, bucket, budget, remaining_override)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    month,
                    bucket,
                    budget,
                    if bucket == "Date" {
                        Some(date_remaining)
                    } else {
                        None::<f64>
                    }
                ],
            )
            .unwrap();
        }
    }

    let tag = Tag::from_str("test");
    db::add_transaction_with_waterfall(
        &conn,
        "move month",
        100.0,
        TransactionType::Debit,
        &tag,
        "Baby",
        "2026-02-10",
    )
    .unwrap();

    let id: i32 = conn
        .query_row(
            "SELECT id FROM transactions
         WHERE source = 'move month' AND related_transaction_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();

    db::update_transaction(
        &conn,
        id,
        "move month",
        120.0,
        TransactionType::Debit,
        &tag,
        "Baby",
        "2026-03-10",
    )
    .unwrap();

    let remaining: Vec<(String, f64)> = conn
        .prepare(
            "SELECT month, remaining_override FROM budgets
         WHERE bucket = 'Date' ORDER BY month",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();

    assert_eq!(
        remaining,
        vec![
            ("2026-02".to_string(), 200.0),
            ("2026-03".to_string(), 230.0),
        ]
    );
}
