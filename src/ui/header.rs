use ratatui::{prelude::*, widgets::Paragraph};

use crate::{money, theme::Theme};

pub fn draw_header(
    f: &mut Frame,
    area: Rect,
    earned: f64,
    spent: f64,
    balance: f64,
    theme: &Theme,
    currency: &str,
) {
    let _ = earned;
    let _ = (earned, balance, currency);
    f.render_widget(build_spent_panel(spent, currency, theme), area);
}

fn build_earned_panel(earned: f64, currency: &str, theme: &Theme) -> Paragraph<'static> {
    let content = vec![
        Line::from(vec![
            Span::styled(
                "↑ ",
                Style::default()
                    .fg(theme.credit)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("EARNED", theme.title()),
        ]),
        Line::raw(""),
        Line::styled(
            format!("{}{}", currency, money::format_vnd(earned)),
            Style::default()
                .fg(theme.credit)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    Paragraph::new(content)
        .block(theme.panel())
        .alignment(Alignment::Center)
}

fn build_balance_panel(balance: f64, currency: &str, theme: &Theme) -> Paragraph<'static> {
    let balance_color = calculate_balance_color(balance, theme);
    let balance_symbol = if balance >= 0.0 { "✓" } else { "⚠" };

    let content = vec![
        Line::from(vec![
            Span::styled(
                balance_symbol,
                Style::default()
                    .fg(balance_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled("BALANCE", theme.title()),
        ]),
        Line::raw(""),
        Line::styled(
            format!("{}{}", currency, money::format_vnd(balance)),
            Style::default()
                .fg(balance_color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ];

    Paragraph::new(content)
        .block(theme.panel())
        .alignment(Alignment::Center)
}

fn build_spent_panel(spent: f64, currency: &str, theme: &Theme) -> Paragraph<'static> {
    let content = vec![
        Line::from(vec![
            Span::styled(
                "↓ ",
                Style::default()
                    .fg(theme.debit)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("SPENT", theme.title()),
        ]),
        Line::raw(""),
        Line::styled(
            format!("{}{}", currency, money::format_vnd(spent)),
            Style::default().fg(theme.debit),
        ),
    ];

    Paragraph::new(content)
        .block(theme.panel())
        .alignment(Alignment::Center)
}

fn calculate_balance_color(balance: f64, theme: &Theme) -> Color {
    if balance >= 0.0 {
        theme.credit
    } else {
        theme.debit
    }
}
