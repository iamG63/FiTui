use crossterm::event::KeyCode;
use ratatui::{
    prelude::*,
    widgets::{BarChart, Block, Paragraph},
};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    app::App,
    models::{MonthlyBudget, MonthlyIncome, Tag, Transaction, TransactionType},
    money,
    theme::Theme,
};
pub struct StatsSnapshot {
    pub earned: f64,
    pub spent: f64,
    pub balance: f64,

    pub per_tag: HashMap<Tag, f64>,
    pub monthly_history: Vec<(String, f64, f64)>,

    pub tx_count: usize,
    pub largest: Option<Transaction>,
    pub smallest: Option<Transaction>,
    pub top_tags: Vec<(Tag, f64)>,
}

impl StatsSnapshot {
    pub fn new(transactions: &[Transaction]) -> Self {
        let earned = calculate_earned(transactions);
        let spent = calculate_spent(transactions);
        let balance = earned - spent;

        let per_tag = calculate_spent_per_tag(transactions);
        let monthly_history = calculate_monthly_history(transactions);

        let tx_count = transactions
            .iter()
            .filter(|tx| tx.related_transaction_id.is_none())
            .count();
        let largest = get_largest_transaction(transactions);
        let smallest = get_smallest_transaction(transactions);

        let top_tags = get_top_tags(&per_tag);

        Self {
            earned,
            spent,
            balance,
            per_tag,
            monthly_history,
            tx_count,
            largest,
            smallest,
            top_tags,
        }
    }
}

// ============================================================================
// Stats calculation functions
// ============================================================================

/// Calculate total earned transactions from app transactions
pub fn calculate_earned(transactions: &[Transaction]) -> f64 {
    transactions
        .iter()
        .filter(|tx| tx.kind == TransactionType::Credit && tx.related_transaction_id.is_none())
        .map(|tx| tx.amount)
        .sum()
}

/// Calculate total spent transactions from app transactions
pub fn new_with_archived_spent(transactions: &[Transaction], archived_spent: f64) -> StatsSnapshot {
    let mut snapshot = StatsSnapshot::new(transactions);
    // Balance reflects live transactions, not archived historical spending.
    snapshot.balance = snapshot.earned - calculate_spent(transactions);
    snapshot
}

pub fn calculate_spent(transactions: &[Transaction]) -> f64 {
    transactions
        .iter()
        .filter(|tx| tx.kind == TransactionType::Debit && tx.related_transaction_id.is_none())
        .map(|tx| tx.amount)
        .sum()
}

/// Build a map of spending per tag from all debit transactions
pub fn calculate_spent_per_tag(transactions: &[Transaction]) -> HashMap<Tag, f64> {
    let mut map = HashMap::new();
    for tx in transactions
        .iter()
        .filter(|tx| tx.kind == TransactionType::Debit && tx.related_transaction_id.is_none())
    {
        *map.entry(tx.tag.clone()).or_insert(0.0) += tx.amount;
    }
    map
}

/// Get the largest transaction by amount
pub fn get_largest_transaction(transactions: &[Transaction]) -> Option<Transaction> {
    transactions
        .iter()
        .filter(|tx| tx.related_transaction_id.is_none())
        .max_by(|a, b| {
            a.amount
                .partial_cmp(&b.amount)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
}

/// Get the smallest transaction by amount
pub fn get_smallest_transaction(transactions: &[Transaction]) -> Option<Transaction> {
    transactions
        .iter()
        .filter(|tx| tx.related_transaction_id.is_none())
        .min_by(|a, b| {
            a.amount
                .partial_cmp(&b.amount)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
}

/// Get top tags sorted by spending amount
pub fn get_top_tags(per_tag: &HashMap<Tag, f64>) -> Vec<(Tag, f64)> {
    let mut top_tags: Vec<(Tag, f64)> = per_tag.iter().map(|(t, v)| (t.clone(), *v)).collect();
    top_tags.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    top_tags
}

/// Calculate monthly history grouped by month (YYYY-MM)
/// Returns up to 6 most recent months with (month, earned, spent) tuples
pub fn calculate_monthly_history(transactions: &[Transaction]) -> Vec<(String, f64, f64)> {
    let mut monthly_map: BTreeMap<String, (f64, f64)> = BTreeMap::new();

    for tx in transactions
        .iter()
        .filter(|tx| tx.related_transaction_id.is_none())
    {
        let month = if tx.date.len() >= 7 {
            tx.date[..7].to_string()
        } else {
            tx.date.clone()
        };

        let entry = monthly_map.entry(month).or_insert((0.0, 0.0));

        match tx.kind {
            TransactionType::Credit => {
                entry.0 += tx.amount;
            }
            TransactionType::Debit => {
                entry.1 += tx.amount;
            }
        }
    }

    monthly_map
        .into_iter()
        .rev()
        .take(6)
        .map(|(m, (e, s))| (m, e, s))
        .collect()
}

// ============================================================================
// Stats UI rendering functions
// ============================================================================

pub fn draw_stats_view(
    f: &mut Frame,
    area: Rect,
    snapshot: &StatsSnapshot,
    transactions: &[Transaction],
    theme: &Theme,
    currency: &str,
    configured_tags: &[Tag],
    monthly_budgets: &[MonthlyBudget],
    monthly_income: Option<&MonthlyIncome>,
) {
    let current_month = chrono::Local::now().format("%Y-%m").to_string();
    let monthly_transactions: Vec<Transaction> = transactions
        .iter()
        .filter(|tx| tx.date.get(..7) == Some(current_month.as_str()))
        .cloned()
        .collect();

    let earned = calculate_earned(&monthly_transactions);
    let spent = calculate_spent(&monthly_transactions);
    let balance = earned - spent;
    let monthly_per_tag = calculate_spent_per_tag(&monthly_transactions);
    let per_tag = &monthly_per_tag;
    let monthly_history = &snapshot.monthly_history;

    let tx_count = snapshot.tx_count;

    // Clone only the Options (for now)
    let largest = snapshot.largest.clone();
    let smallest = snapshot.smallest.clone();

    let top_tags = &snapshot.top_tags;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    // Split main stats area into top charts and bottom breakdown
    let top_bottom = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(1)])
        .split(layout[0]);

    let charts_area = top_bottom[0];
    let breakdown_area = top_bottom[1];

    // Monthly history chart uses the full width; Top Tags is hidden
    // because Spending Breakdown already shows category totals.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(100)])
        .split(charts_area);

    // Prepare monthly bars and sparkline
    let mut month_labels: Vec<String> = Vec::new();
    let mut earned_vals: Vec<u64> = Vec::new();
    let mut spent_vals: Vec<u64> = Vec::new();
    for (m, e, s) in monthly_history.iter().rev() {
        month_labels.push(m.clone());
        earned_vals.push((*e).round().abs() as u64);
        spent_vals.push((*s).round().abs() as u64);
    }

    // Labels as &str for BarChart
    let month_label_refs: Vec<&str> = month_labels.iter().map(|s| s.as_str()).collect();

    // Monthly earned bar chart
    let monthly_earned: Vec<(&str, u64)> = month_label_refs
        .iter()
        .zip(earned_vals.iter())
        .map(|(l, v)| (*l, *v))
        .collect();

    let max_month = earned_vals
        .iter()
        .chain(spent_vals.iter())
        .copied()
        .max()
        .unwrap_or(0);

    let earned_chart = BarChart::default()
        .data(&monthly_earned)
        .block(theme.block("Monthly Earned"))
        .max(max_month.max(1))
        .bar_width(7)
        .bar_gap(1)
        .bar_style(Style::default().fg(theme.credit));

    f.render_widget(earned_chart, cols[0]);

    // Below charts: breakdown paragraph (reuse existing content builder for details)
    let breakdown_lines = build_stats_content(
        earned,
        spent,
        balance,
        per_tag,
        configured_tags,
        monthly_history,
        tx_count,
        &monthly_transactions,
        largest,
        smallest,
        top_tags,
        theme,
        currency,
        monthly_budgets,
        monthly_income,
    );
    let breakdown = Paragraph::new(breakdown_lines)
        .block(theme.block("Details"))
        .alignment(Alignment::Left);

    f.render_widget(breakdown, breakdown_area);

    // Enhanced footer
    let footer_block = Block::default()
        .borders(ratatui::widgets::Borders::TOP)
        .border_style(Style::default().fg(theme.subtle))
        .style(Style::default().bg(Color::Reset))
        .padding(ratatui::widgets::Padding::new(1, 1, 0, 0));

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("  [Esc] Back", Style::default().fg(theme.muted)),
        Span::styled("  ", Style::default()),
        Span::styled("[Tab/←→] Switch view", Style::default().fg(theme.muted)),
    ]))
    .block(footer_block)
    .alignment(Alignment::Left);

    f.render_widget(footer, layout[1]);
}

fn calculate_bucket_spent(transactions: &[Transaction], bucket: &str, month: &str) -> f64 {
    let parents_with_children: HashSet<i32> = transactions
        .iter()
        .filter_map(|tx| tx.related_transaction_id)
        .collect();

    transactions
        .iter()
        .filter(|tx| {
            tx.kind == TransactionType::Debit
                && tx.bucket == bucket
                && tx.date.get(..7) == Some(month)
                && (tx.related_transaction_id.is_some() || !parents_with_children.contains(&tx.id))
        })
        .map(|tx| tx.amount)
        .sum()
}

fn calculate_tracked_bucket_displayed_spent(budget: &MonthlyBudget, actual_spent: f64) -> f64 {
    budget
        .remaining_override
        .map(|remaining| budget.budget - remaining + actual_spent)
        .unwrap_or(actual_spent)
}

fn calculate_tracked_bucket_displayed_remaining(budget: &MonthlyBudget, actual_spent: f64) -> f64 {
    budget.budget - calculate_tracked_bucket_displayed_spent(budget, actual_spent)
}

fn build_stats_content(
    earned: f64,
    spent: f64,
    balance: f64,
    per_tag: &HashMap<Tag, f64>,
    configured_tags: &[Tag],
    _monthly_history: &[(String, f64, f64)],
    _tx_count: usize,
    transactions: &[Transaction],
    largest: Option<Transaction>,
    smallest: Option<Transaction>,
    _top_tags: &[(Tag, f64)],
    theme: &Theme,
    currency: &str,
    monthly_budgets: &[MonthlyBudget],
    monthly_income: Option<&MonthlyIncome>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Monthly budget snapshot from SQLite
    if let Some(income) = monthly_income {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(
                "  Monthly Dashboard: {} | Income: {} | Last edited: {}",
                income.month,
                money::format_vnd({
                    let transaction_income: f64 = transactions
                        .iter()
                        .filter(|tx| {
                            tx.kind == crate::models::TransactionType::Credit
                                && tx.date.get(..7) == Some(income.month.as_str())
                        })
                        .map(|tx| tx.amount)
                        .sum();
                    income.income + transaction_income
                }),
                income.last_edited
            ),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(""));

        lines.push(Line::raw(format!(
            "     {:<20} {:>10} {:>11} {:>13}  {:>14}",
            "Bucket", "Budget", "Spent", "Remaining", "Status",
        )));

        for budget in monthly_budgets {
            let spent_text = if budget.spent_tracking {
                let actual_spent =
                    calculate_bucket_spent(transactions, &budget.bucket, &budget.month);

                let displayed_spent =
                    if budget.bucket == "Baby" || budget.bucket == "Electricity Baby" {
                        budget
                            .spent_override
                            .map(|baseline| baseline + actual_spent)
                            .unwrap_or(actual_spent)
                            .min(budget.budget)
                    } else if budget.bucket == "Date" {
                        budget.budget - budget.remaining_override.unwrap_or(0.0)
                    } else if matches!(budget.bucket.as_str(), "Personal" | "Travel" | "Saving") {
                        calculate_tracked_bucket_displayed_spent(budget, actual_spent)
                    } else {
                        actual_spent
                    };

                money::format_vnd(displayed_spent)
            } else {
                "-".to_string()
            };

            let remaining = if budget.spent_tracking
                && matches!(budget.bucket.as_str(), "Baby" | "Electricity Baby")
            {
                let actual_spent =
                    calculate_bucket_spent(transactions, &budget.bucket, &budget.month);
                let displayed_spent = budget
                    .spent_override
                    .map(|baseline| baseline + actual_spent)
                    .unwrap_or(actual_spent)
                    .min(budget.budget);
                money::format_vnd((budget.budget - displayed_spent).max(0.0))
            } else if matches!(budget.bucket.as_str(), "Personal" | "Travel" | "Saving")
                && budget.spent_tracking
            {
                let actual_spent =
                    calculate_bucket_spent(transactions, &budget.bucket, &budget.month);
                money::format_vnd(calculate_tracked_bucket_displayed_remaining(
                    budget,
                    actual_spent,
                ))
            } else {
                budget
                    .remaining_override
                    .map(money::format_vnd)
                    .unwrap_or_else(|| "-".to_string())
            };

            lines.push(Line::raw(format!(
                "     {:<20} {:>10} {:>11} {:>13}  {:>14}",
                budget.bucket,
                money::format_vnd(budget.budget),
                spent_text,
                remaining,
                budget.status,
            )));
        }

        lines.push(Line::raw(""));
    }

    lines.push(Line::raw(""));

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "  Spending Breakdown by Category",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::raw(""));

    let configured_spending: HashMap<Tag, f64> = configured_tags
        .iter()
        .filter_map(|tag| per_tag.get(tag).map(|amount| (tag.clone(), *amount)))
        .collect();
    if configured_spending.is_empty() {
        lines.push(Line::styled(
            "     No spending data available yet.",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ));
    } else {
        lines.extend(create_tag_breakdown_section(
            &configured_spending,
            theme,
            currency,
        ));
    }

    lines.push(Line::raw(""));
    lines
}

fn create_overview_section(
    earned: f64,
    spent: f64,
    balance: f64,
    theme: &Theme,
    currency: &str,
) -> Vec<Line<'static>> {
    let balance_color = if balance >= 0.0 {
        theme.credit
    } else {
        theme.debit
    };
    let savings_rate = if earned > 0.0 {
        (((earned - spent) / earned) * 100.0).max(0.0)
    } else {
        0.0
    };

    vec![
        Line::styled(
            "  Financial Overview:",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::raw("     Total Earned  : "),
            Span::styled(
                format!("{}{}", currency, money::format_vnd(earned)),
                Style::default()
                    .fg(theme.credit)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("     Total Spent   : "),
            Span::styled(
                format!("{}{}", currency, money::format_vnd(spent)),
                Style::default().fg(theme.debit),
            ),
        ]),
        Line::from(vec![
            Span::raw("     Balance       : "),
            Span::styled(
                format!("{}{}", currency, money::format_vnd(balance)),
                Style::default()
                    .fg(balance_color)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ),
        ]),
        Line::from(vec![
            Span::raw("     Savings Rate  : "),
            Span::styled(
                format!("{:>9.1}%", savings_rate),
                Style::default()
                    .fg(if savings_rate > 20.0 {
                        theme.credit
                    } else {
                        theme.accent
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ]
}

fn create_tag_breakdown_section(
    per_tag: &HashMap<Tag, f64>,
    theme: &Theme,
    currency: &str,
) -> Vec<Line<'static>> {
    let mut tag_vec: Vec<_> = per_tag.iter().collect();
    tag_vec.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());

    let max_spent = tag_vec.first().map(|(_, v)| **v).unwrap_or(0.0);
    let total_spent: f64 = per_tag.values().sum();

    let mut lines = Vec::new();

    for (tag, &amount) in tag_vec {
        let percentage = if total_spent > 0.0 {
            (amount / total_spent) * 100.0
        } else {
            0.0
        };

        lines.push(create_tag_bar(
            tag.as_str(),
            amount,
            percentage,
            max_spent,
            theme,
            currency,
        ));
    }

    lines
}

fn create_tag_bar(
    tag: &str,
    amount: f64,
    percentage: f64,
    max_amount: f64,
    theme: &Theme,
    currency: &str,
) -> Line<'static> {
    let bar_width = calculate_bar_width(amount, max_amount);
    let bar = "█".repeat(bar_width);
    let empty_bar = "░".repeat((20usize).saturating_sub(bar_width));

    Line::from(vec![
        Span::raw("     "),
        Span::styled(
            format!("#{:>14}", tag),
            Style::default()
                .fg(theme.accent_soft)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::raw(" "),
        Span::styled(bar, Style::default().fg(theme.debit)),
        Span::styled(empty_bar, Style::default().fg(theme.subtle)),
        Span::raw("  "),
        Span::styled(
            format!("{}{}", currency, money::format_vnd(amount)),
            Style::default().fg(theme.foreground),
        ),
        Span::raw(" "),
        Span::styled(
            format!("({:>5.1}%)", percentage),
            Style::default().fg(theme.muted),
        ),
    ])
}

fn calculate_bar_width(amount: f64, max_amount: f64) -> usize {
    if max_amount > 0.0 {
        ((amount / max_amount) * 20.0).round() as usize
    } else {
        0
    }
}

// ============================================================================
// Stats input handler
// ============================================================================

pub fn handle_stats(app: &mut App, key: KeyCode) -> bool {
    match key {
        KeyCode::Esc => {
            app.mode = crate::app::Mode::Normal;
        }
        _ => {}
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Tag, Transaction, TransactionType};

    fn tx(
        id: i32,
        source: &str,
        amount: f64,
        kind: TransactionType,
        tag: &str,
        date: &str,
    ) -> Transaction {
        Transaction {
            id,
            source: source.to_string(),
            amount,
            kind,
            tag: Tag::from_str(tag),
            bucket: "Baby".into(),
            date: date.to_string(),
            related_transaction_id: None,
        }
    }

    #[test]
    fn travel_and_saving_spent_and_remaining_update_with_new_transaction() {
        for bucket in ["Travel", "Saving"] {
            let mut transaction = tx(
                if bucket == "Travel" { 10 } else { 11 },
                "test",
                100_000.0,
                TransactionType::Debit,
                "other",
                "2026-02-10",
            );
            transaction.bucket = bucket.to_string();

            let transactions = vec![transaction];
            let budget = MonthlyBudget {
                month: "2026-02".to_string(),
                bucket: bucket.to_string(),
                budget: 1_000_000.0,
                spent_tracking: true,
                spent_override: None,
                remaining_override: Some(700_000.0),
                status: "Within budget".to_string(),
            };

            let actual_spent = calculate_bucket_spent(&transactions, bucket, "2026-02");

            assert_eq!(actual_spent, 100_000.0);
            assert_eq!(
                calculate_tracked_bucket_displayed_spent(&budget, actual_spent),
                400_000.0,
                "{bucket}: displayed spent should include the new transaction"
            );
            assert_eq!(
                calculate_tracked_bucket_displayed_remaining(&budget, actual_spent),
                600_000.0,
                "{bucket}: remaining should decrease after the new transaction"
            );
        }
    }

    #[test]
    fn personal_spent_and_remaining_update_with_new_transaction() {
        let mut personal_tx = tx(
            1,
            "test",
            100_000.0,
            TransactionType::Debit,
            "personal",
            "2026-02-10",
        );
        personal_tx.bucket = "Personal".into();

        let transactions = vec![personal_tx];
        let budget = MonthlyBudget {
            month: "2026-02".to_string(),
            bucket: "Personal".to_string(),
            budget: 1_175_000.0,
            spent_tracking: true,
            spent_override: None,
            remaining_override: Some(-145_000.0),
            status: "Over budget".to_string(),
        };

        let actual_spent = calculate_bucket_spent(&transactions, "Personal", "2026-02");

        assert_eq!(actual_spent, 100_000.0);
        assert_eq!(
            calculate_tracked_bucket_displayed_spent(&budget, actual_spent),
            1_420_000.0
        );
        assert_eq!(
            calculate_tracked_bucket_displayed_remaining(&budget, actual_spent),
            -245_000.0
        );
    }

    #[test]
    fn waterfall_children_do_not_double_count_in_stats() {
        let mut parent = tx(
            1,
            "test",
            250.0,
            TransactionType::Debit,
            "food",
            "2026-10-09",
        );
        parent.bucket = "Baby".into();

        let mut baby_leg = tx(
            2,
            "test",
            100.0,
            TransactionType::Debit,
            "food",
            "2026-10-09",
        );
        baby_leg.bucket = "Baby".into();
        baby_leg.related_transaction_id = Some(1);

        let mut electric_leg = tx(
            3,
            "test",
            150.0,
            TransactionType::Debit,
            "food",
            "2026-10-09",
        );
        electric_leg.bucket = "Electric".into();
        electric_leg.related_transaction_id = Some(1);

        let transactions = vec![parent, baby_leg, electric_leg];

        assert_eq!(calculate_spent(&transactions), 250.0);
        assert_eq!(
            calculate_spent_per_tag(&transactions).get(&Tag("food".into())),
            Some(&250.0)
        );

        let history = calculate_monthly_history(&transactions);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].2, 250.0);

        let snapshot = StatsSnapshot::new(&transactions);
        assert_eq!(snapshot.tx_count, 1);
        assert_eq!(
            get_largest_transaction(&transactions).unwrap().amount,
            250.0
        );
        assert_eq!(
            get_smallest_transaction(&transactions).unwrap().amount,
            250.0
        );
    }

    #[test]
    fn calculate_earned_and_spent() {
        let transactions = vec![
            tx(
                1,
                "a",
                200.0,
                TransactionType::Credit,
                "salary",
                "2026-02-01",
            ),
            tx(2, "b", 50.0, TransactionType::Debit, "food", "2026-02-02"),
            tx(3, "c", 25.0, TransactionType::Debit, "misc", "2026-02-03"),
        ];

        let earned = calculate_earned(&transactions);
        let spent = calculate_spent(&transactions);

        assert_eq!(earned, 200.0);
        assert_eq!(spent, 75.0);
    }

    #[test]
    fn monthly_history_groups_and_orders() {
        let transactions = vec![
            tx(1, "a", 100.0, TransactionType::Credit, "x", "2026-01-05"),
            tx(2, "b", 30.0, TransactionType::Debit, "y", "2026-02-10"),
            tx(3, "c", 50.0, TransactionType::Credit, "x", "2026-02-15"),
            tx(4, "d", 20.0, TransactionType::Debit, "z", "2025-12-31"),
        ];

        let months = calculate_monthly_history(&transactions);

        // Expect descending months (latest first)
        assert!(months.len() >= 3);
        assert_eq!(months[0].0, "2026-02");
        // Check earned/spent values for 2026-02
        let feb = &months[0];
        assert_eq!(feb.1, 50.0); // earned
        assert_eq!(feb.2, 30.0); // spent
    }

    #[test]
    fn top_tags_sort_descending() {
        let mut map = std::collections::HashMap::new();
        map.insert(Tag::from_str("a"), 10.0);
        map.insert(Tag::from_str("b"), 50.0);
        map.insert(Tag::from_str("c"), 30.0);

        let top = get_top_tags(&map);
        assert_eq!(top[0].0, Tag::from_str("b"));
        assert_eq!(top[1].0, Tag::from_str("c"));
        assert_eq!(top[2].0, Tag::from_str("a"));
    }

    #[test]
    fn calculate_bar_width_handles_zero_max() {
        assert_eq!(calculate_bar_width(0.0, 0.0), 0);
        assert!(calculate_bar_width(5.0, 10.0) > 0);
    }
}
