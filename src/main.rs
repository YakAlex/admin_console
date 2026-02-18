mod config;
mod types;
mod utils;
mod monitor;
mod ui;
mod app;
mod inputs; // <--- Підключаємо inputs.rs

use anyhow::Result;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, backend::CrosstermBackend, Terminal};
use std::{fs, io, sync::mpsc, time::{Duration, Instant}};
use chrono::Local;
use tui_textarea::TextArea;
use ratatui::widgets::{Block, Borders};

use crate::types::{AppEvent, MonitorCommand};
use crate::monitor::start_monitor;
use crate::app::App;

fn main() -> Result<()> {
    // 1. Канали
    let (tx, rx) = mpsc::channel::<AppEvent>();
    let (tx_to_monitor, rx_from_main) = mpsc::channel::<MonitorCommand>();

    // 2. Ініціалізація App (завантажує все сама)
    // Передаємо tx (для UI) і tx_to_monitor (для команд)
    let mut app = App::new(tx_to_monitor.clone(), tx.clone());

    // 3. Запуск монітора
    start_monitor(app.config.targets.clone(), app.tasks.clone(), tx.clone(), rx_from_main);

    // 4. Термінал
    let is_raw_mode = enable_raw_mode().is_ok();
    let mut stdout = io::stdout();
    if is_raw_mode {
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();

    loop {
        // --- СИНХРОНІЗАЦІЯ ---
        app.sync_todo_from_text();

        // --- ПОДІЇ ВІД ПОТОКІВ ---
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::ServerUpdate(data) => { app.server_data = data; app.should_redraw = true; }
                AppEvent::LogOutput(text) => {
                    let log_textarea = &mut app.textareas[2];
                    if text.starts_with('[') { log_textarea.insert_str(text); }
                    else { log_textarea.insert_str(format!("[{}] Output:\n{}", Local::now().format("%H:%M:%S"), text)); }
                    log_textarea.insert_str("\n-------------------------------------------\n");
                    app.files_modified[2] = true; app.should_redraw = true;
                }
                AppEvent::TaskCompleted(title) => {
                    // Логіка галочки [x]
                    let todo_textarea = &mut app.textareas[1];
                    let old_lines = todo_textarea.lines().to_vec();
                    let mut new_lines = Vec::new();
                    let mut modified = false;

                    for line in old_lines {
                        if line.contains(&title) && line.trim().starts_with("- [") && !line.contains("[x]") && !line.contains("[X]") {
                            let start_bracket = line.find('[').unwrap_or(0);
                            let end_bracket = line.find(']').unwrap_or(line.len());
                            if end_bracket > start_bracket {
                                let new_line = format!("{}[x]{}", &line[..start_bracket], &line[end_bracket + 1..]);
                                new_lines.push(new_line);
                                modified = true;
                            } else { new_lines.push(line); }
                        } else { new_lines.push(line); }
                    }

                    if modified {
                        *todo_textarea = TextArea::new(new_lines);
                        todo_textarea.set_block(Block::default().borders(Borders::ALL));

                        app.files_modified[1] = true;
                        app.sync_todo_from_text();
                        app.should_redraw = true;
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate { app.should_redraw = true; last_tick = Instant::now(); }

        // --- МАЛЮВАННЯ ---
        if app.should_redraw {
            terminal.draw(|f| {
                ui::draw(f, &mut app);
            })?;
            app.should_redraw = false;
        }

        // --- ВВІД (ЧЕРЕЗ inputs.rs) ---
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            let evt = event::read()?;
            match evt {
                Event::Paste(data) => {
                    app.last_user_activity = Instant::now();
                    if let crate::types::ActiveView::Editor(mode) = app.active_view {
                        app.textareas[mode as usize].insert_str(data);
                        app.files_modified[mode as usize] = true; app.should_redraw = true;
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.last_user_activity = Instant::now();
                    app.should_redraw = true;

                    // ВЕСЬ match ПЕРЕЇХАВ СЮДИ:
                    inputs::handle_input(key, &mut app);
                }
                _ => {}
            }
        }

        // --- АВТОЗБЕРЕЖЕННЯ ---
        if app.last_user_activity.elapsed() >= Duration::from_secs(30) {
            for (i, modified) in app.files_modified.iter_mut().enumerate() {
                if *modified {
                    let text_to_save = app.textareas[i].lines().join("\n");
                    fs::write(app.file_names[i], text_to_save).ok();
                    *modified = false;
                }
            }
            if app.tasks_modified {
                let _ = fs::write("tasks.json", serde_json::to_string_pretty(&app.tasks).unwrap_or_default());
                app.tasks_modified = false;
            }
        }

        if app.should_quit { break; }
    }

    // --- ФІНАЛЬНЕ ЗБЕРЕЖЕННЯ ---
    for (i, filename) in app.file_names.iter().enumerate() { let text_to_save = app.textareas[i].lines().join("\n"); fs::write(filename, text_to_save)?; }
    if app.tasks_modified { let _ = fs::write("tasks.json", serde_json::to_string_pretty(&app.tasks).unwrap_or_default()); }

    if is_raw_mode {
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste)?;
    }
    terminal.show_cursor()?;
    Ok(())
}