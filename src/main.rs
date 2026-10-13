#![allow(non_snake_case)]
mod app;
mod config;
mod db;
mod form;
mod handlers;
mod models;
mod money;
mod stats;
mod theme;
mod ui;

use std::io;

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

use ratatui::prelude::*;

use app::App;

fn main() -> io::Result<()> {
    let conn = db::init_db().unwrap();
    let mut finance_month = chrono::Local::now().format("%Y-%m").to_string();
    // Insert recurring entries based on their intervals
    db::insert_recurring_transactions(&conn).unwrap();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(&conn);
    let mut monthly_budgets = db::get_monthly_budgets(&conn, &finance_month).unwrap_or_default();
    let mut monthly_income = db::get_monthly_income(&conn, &finance_month).unwrap_or(None);
    app.dashboard_dirty = false;

    loop {
        let current_month = chrono::Local::now().format("%Y-%m").to_string();
        if current_month != finance_month {
            finance_month = current_month;
            app.dashboard_dirty = true;
        }

        if app.dashboard_dirty {
            monthly_budgets = db::get_monthly_budgets(&conn, &finance_month).unwrap_or_default();
            monthly_income = db::get_monthly_income(&conn, &finance_month).unwrap_or(None);
            app.dashboard_dirty = false;
        }
        let archived_spent = db::get_archived_total_spent(&conn).unwrap_or(0.0);
        let snapshot = stats::new_with_archived_spent(&app.transactions, archived_spent);

        terminal.draw(|f| {
            ui::draw_ui(
                f,
                &app,
                &snapshot,
                &monthly_budgets,
                monthly_income.as_ref(),
            );
        })?;

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let quit = handlers::handle_key(&mut app, key.code, &conn);

                    if quit {
                        break;
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}
