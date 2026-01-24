// Підключаємо наші нові модулі
mod config;
mod types;
mod utils;

use anyhow::Result;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Gauge, List, ListItem, ListState, Tabs, Table, Row, Cell, Clear},
    style::{Color, Modifier, Style},
};
use std::{fs, io, net::TcpStream, process::Command, sync::mpsc, thread, time::{Duration, Instant}, collections::VecDeque};
use tui_textarea::{TextArea, CursorMove};
use sysinfo::System;
use encoding_rs::IBM866;
use arboard::Clipboard;

// Використовуємо типи з наших нових файлів
use crate::config::AppConfig;
use crate::types::{ServerStatus, AppEvent, EditorMode, ActiveView};
use crate::utils::{generate_sparkline, centered_rect};

fn main() -> Result<()> {
    // --- ІНІЦІАЛІЗАЦІЯ ---
    let file_names = vec!["notes.txt", "todo.txt", "logs.txt"];
    let titles = vec![" 1.Notes ", " 2.Todo ", " 3.Logs "];

    let mut textareas = Vec::new();
    for filename in &file_names {
        let content = fs::read_to_string(filename).unwrap_or_default();
        let mut ta = TextArea::new(content.lines().map(|s| s.to_string()).collect());
        ta.set_max_histories(10000);
        ta.set_block(Block::default().borders(Borders::ALL));
        ta.set_search_style(Style::default().bg(Color::Yellow).fg(Color::Black));
        textareas.push(ta);
    }

    let config_path = "config.json";
    let config_data = fs::read_to_string(config_path).unwrap_or_else(|_| {
        r#"{ "targets": [], "commands": [] }"#.to_string()
    });

    let config: AppConfig = serde_json::from_str(&config_data).unwrap_or(AppConfig { targets: vec![], commands: vec![] });

    let targets_for_thread = config.targets.clone();
    let commands = config.commands.clone();

    let mut list_state = ListState::default();
    if !commands.is_empty() {
        list_state.select(Some(0));
    }

    let mut active_view = ActiveView::Editor(EditorMode::Notes);
    let (tx, rx) = mpsc::channel::<AppEvent>();
    let mut clipboard = Clipboard::new().ok();

    let tx_monitor = tx.clone();

    // --- ФОНОВИЙ ПОТІК (МОНІТОРИНГ) ---
    thread::spawn(move || {
        let mut statuses: Vec<ServerStatus> = targets_for_thread.iter().map(|t| ServerStatus {
            name: t.name.clone(),
            is_online: false,
            latency: 0,
            history: VecDeque::from(vec![0; 20]),
        }).collect();

        loop {
            for (i, target) in targets_for_thread.iter().enumerate() {
                let start = Instant::now();
                let (online, lat) = match TcpStream::connect_timeout(&target.address.parse().unwrap_or("0.0.0.0:0".parse().unwrap()), Duration::from_millis(500)) {
                    Ok(_) => (true, start.elapsed().as_millis()),
                    Err(_) => (false, 0),
                };

                let status = &mut statuses[i];
                status.is_online = online;
                status.latency = lat;

                let history_val = if status.is_online { status.latency } else { 999 };
                status.history.pop_front();
                status.history.push_back(history_val);
            }
            let _ = tx_monitor.send(AppEvent::ServerUpdate(statuses.clone()));
            thread::sleep(Duration::from_secs(1));
        }
    });

    let mut sys = System::new_all();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut server_data: Vec<ServerStatus> = Vec::new();
    let mut should_redraw = true;
    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();
    let mut is_selecting = false;
    let mut files_modified = vec![false, false, false];

    loop {
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::ServerUpdate(data) => {
                    server_data = data;
                    should_redraw = true;
                }
                AppEvent::LogOutput(text) => {
                    let log_textarea = &mut textareas[2];
                    log_textarea.insert_str(text);
                    log_textarea.insert_str("\n--------------------------\n");
                    files_modified[2] = true;
                    should_redraw = true;
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            sys.refresh_all();
            should_redraw = true;
            last_tick = Instant::now();
        }

        if should_redraw {
            let global_cpu_usage = sys.global_cpu_info().cpu_usage();
            let used_mem = sys.used_memory();
            let total_mem = sys.total_memory();
            let mem_percentage = if total_mem > 0 { (used_mem as f64 / total_mem as f64) * 100.0 } else { 0.0 };

            terminal.draw(|f| {
                let main_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                    .split(f.area());

                // ✅ НОВИЙ ВАРІАНТ (Адаптивний)
                let left_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(10),  // Таблиця серверів забирає ВСЕ вільне місце (мінімум 10 рядків)
                        Constraint::Length(6) // Нижня панель (System) завжди фіксована — 6 рядків
                    ])
                    .split(main_chunks[0]);

                // --- TABLE ---
                let header_cells = ["Server", "Ping", "Status", "History"]
                    .iter()
                    .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));
                let header = Row::new(header_cells).height(1).bottom_margin(1);

                let rows = server_data.iter().map(|item| {
                    let ping_text = if item.is_online { format!("{}ms", item.latency) } else { "---".to_string() };
                    let status_symbol = if item.is_online { "🟢" } else { "🔴" };
                    let color = if !item.is_online { Color::Red }
                    else if item.latency > 100 { Color::Yellow }
                    else { Color::Green };
                    let sparkline = generate_sparkline(&item.history);

                    let cells = vec![
                        Cell::from(item.name.clone()),
                        Cell::from(ping_text),
                        Cell::from(status_symbol),
                        Cell::from(sparkline).style(Style::default().fg(color)),
                    ];
                    Row::new(cells).height(1)
                });

                let table = Table::new(rows, [
                    Constraint::Length(12),
                    Constraint::Length(6),
                    Constraint::Length(4),
                    Constraint::Min(10),
                ])
                    .header(header)
                    .block(Block::default().borders(Borders::ALL).title(" 📡 Servers "));
                f.render_widget(table, left_chunks[0]);

                // --- SYSTEM ---
                let sys_block = Block::default().title(" 💻 System ").borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow));
                f.render_widget(sys_block, left_chunks[1]);
                let sys_area = left_chunks[1].inner(Margin { vertical: 1, horizontal: 1 });
                let gauge_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Length(2)])
                    .split(sys_area);

                f.render_widget(Gauge::default().gauge_style(Style::default().fg(Color::Green)).ratio((global_cpu_usage as f64 / 100.0).clamp(0.0, 1.0)).label(format!("CPU: {:.1}%", global_cpu_usage)), gauge_chunks[0]);
                f.render_widget(Gauge::default().gauge_style(Style::default().fg(Color::Magenta)).ratio((mem_percentage / 100.0).clamp(0.0, 1.0)).label(format!("RAM: {:.1}%", mem_percentage)), gauge_chunks[2]);

                let right_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(3), Constraint::Min(0)])
                    .split(main_chunks[1]);

                // --- TABS ---
                let (current_file_idx, is_actions_active) = match active_view {
                    ActiveView::Editor(mode) => (mode as usize, false),
                    ActiveView::Search { mode_return_to, .. } => (mode_return_to as usize, false),
                    ActiveView::Actions => (0, true),
                    ActiveView::InputPopup { .. } => (0, true),
                };

                let file_tabs = Tabs::new(titles.clone())
                    .block(Block::default().borders(Borders::BOTTOM))
                    .select(if !is_actions_active { current_file_idx } else { 99 })
                    .highlight_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));

                let header_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
                    .split(right_chunks[0]);

                f.render_widget(file_tabs, header_chunks[0]);

                let action_status = if is_actions_active {
                    Paragraph::new(" [TAB] ACTIONS ").style(Style::default().fg(Color::Black).bg(Color::Yellow))
                } else {
                    Paragraph::new(" [TAB] to Actions ")
                };
                f.render_widget(action_status, header_chunks[1]);

                // --- MAIN CONTENT ---
                match &active_view {
                    ActiveView::Editor(mode) | ActiveView::Search { mode_return_to: mode, .. } => {
                        let idx = *mode as usize;
                        f.render_widget(&textareas[idx], right_chunks[1]);
                    }
                    ActiveView::Actions | ActiveView::InputPopup { .. } => {
                        let items: Vec<ListItem> = commands
                            .iter()
                            .map(|i| ListItem::new(i.name.clone()).style(Style::default().fg(Color::White)))
                            .collect();

                        let list = List::new(items)
                            .block(Block::default().borders(Borders::ALL).title(" Оберіть команду "))
                            .highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD))
                            .highlight_symbol(">> ");

                        f.render_stateful_widget(list, right_chunks[1], &mut list_state);
                    }
                }

                // --- SEARCH BAR ---
                if let ActiveView::Search { query, .. } = &active_view {
                    let area = right_chunks[1];
                    let search_area = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(0), Constraint::Length(3)])
                        .split(area)[1];

                    let search_block = Paragraph::new(format!("Search: {}", query))
                        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)))
                        .style(Style::default().fg(Color::Yellow).bg(Color::Black));

                    f.render_widget(Clear, search_area);
                    f.render_widget(search_block, search_area);
                }

                // --- POPUP ---
                if let ActiveView::InputPopup { input_buffer, .. } = &active_view {
                    let area = centered_rect(60, 20, f.area());
                    f.render_widget(Clear, area);

                    let popup_block = Paragraph::new(input_buffer.clone())
                        .block(Block::default().borders(Borders::ALL).title(" Введіть аргумент (IP/Host) "))
                        .style(Style::default().fg(Color::Yellow).bg(Color::Black));

                    f.render_widget(popup_block, area);
                }
            })?;

            should_redraw = false;
        }

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());

        if event::poll(timeout)? {
            let evt = event::read()?;

            match evt {
                Event::Paste(data) => {
                    if let ActiveView::Editor(mode) = active_view {
                        should_redraw = true;
                        let idx = mode as usize;
                        textareas[idx].insert_str(data);
                        files_modified[idx] = true;
                    }
                }

                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    should_redraw = true;
                    let mut change_view = None;

                    if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('q') || key.code == KeyCode::Char('й')) {
                        break;
                    }

                    match &mut active_view {
                        // === РЕЖИМ ПОШУКУ ===
                        ActiveView::Search { mode_return_to, query } => {
                            let idx = *mode_return_to as usize;
                            match key.code {
                                KeyCode::Esc => {
                                    textareas[idx].set_search_pattern("").ok();
                                    change_view = Some(ActiveView::Editor(*mode_return_to));
                                }
                                KeyCode::Enter => {
                                    textareas[idx].search_forward(false);
                                }
                                KeyCode::Backspace => {
                                    query.pop();
                                    textareas[idx].set_search_pattern(query.as_str()).ok();
                                }
                                KeyCode::Char(c) => {
                                    query.push(c);
                                    textareas[idx].set_search_pattern(query.as_str()).ok();
                                    textareas[idx].search_forward(false);
                                }
                                _ => {}
                            }
                        }

                        // === РЕЖИМ POPUP ВВОДУ ===
                        ActiveView::InputPopup { command_idx, input_buffer } => {
                            match key.code {
                                KeyCode::Enter => {
                                    let idx_copy = *command_idx;
                                    if idx_copy < commands.len() {
                                        let cmd_struct = commands[idx_copy].clone();
                                        let buffer_clone = input_buffer.clone();

                                        let final_args: Vec<String> = cmd_struct.args.iter()
                                            .map(|arg| if arg == "%INPUT%" { buffer_clone.clone() } else { arg.clone() })
                                            .collect();

                                        change_view = Some(ActiveView::Editor(EditorMode::Logs));
                                        let log_textarea = &mut textareas[2];
                                        log_textarea.insert_str(format!("\n--- Executing (Async): {} ({}) ---\n", cmd_struct.name, buffer_clone));
                                        files_modified[2] = true;

                                        let tx_cmd = tx.clone();
                                        let cmd_exe = cmd_struct.cmd.clone();

                                        thread::spawn(move || {
                                            let output = Command::new(cmd_exe).args(final_args).output();
                                            let mut result_text = String::new();
                                            match output {
                                                Ok(o) => {
                                                    let (decoded_str, _, _) = IBM866.decode(&o.stdout);
                                                    result_text.push_str(&decoded_str);
                                                    if !o.stderr.is_empty() {
                                                        let (err_str, _, _) = IBM866.decode(&o.stderr);
                                                        result_text.push_str("\nERROR:\n");
                                                        result_text.push_str(&err_str);
                                                    }
                                                },
                                                Err(e) => { result_text.push_str(&format!("Failed to run: {}", e)); }
                                            }
                                            let _ = tx_cmd.send(AppEvent::LogOutput(result_text));
                                        });
                                    }
                                }
                                KeyCode::Esc => { change_view = Some(ActiveView::Actions); }
                                KeyCode::Backspace => { input_buffer.pop(); }
                                KeyCode::Char(c) => { input_buffer.push(c); }
                                _ => {}
                            }
                        }

                        // === РЕЖИМ РЕДАКТОРА ===
                        ActiveView::Editor(mode) => {
                            let idx = *mode as usize;
                            let textarea = &mut textareas[idx];

                            if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('f') || key.code == KeyCode::Char('а')) {
                                change_view = Some(ActiveView::Search {
                                    mode_return_to: *mode,
                                    query: String::new()
                                });
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('с')) {
                                textarea.copy();
                                let text = textarea.yank_text();
                                if !text.is_empty() { if let Some(cb) = &mut clipboard { let _ = cb.set_text(text); } }
                            }
                            else if (key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) ||
                                (key.modifiers == KeyModifiers::ALT && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) {
                                if let Some(cb) = &mut clipboard {
                                    if let Ok(text) = cb.get_text() { textarea.insert_str(text); files_modified[idx] = true; }
                                }
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('x') || key.code == KeyCode::Char('ч')) {
                                textarea.cut();
                                let text = textarea.yank_text();
                                if let Some(cb) = &mut clipboard { let _ = cb.set_text(text); }
                                files_modified[idx] = true;
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('z') || key.code == KeyCode::Char('я')) {
                                textarea.undo(); files_modified[idx] = true;
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('y') || key.code == KeyCode::Char('н')) {
                                textarea.redo(); files_modified[idx] = true;
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('a') || key.code == KeyCode::Char('ф')) {
                                textarea.move_cursor(CursorMove::Top);
                                textarea.move_cursor(CursorMove::Head);
                                textarea.start_selection();
                                textarea.move_cursor(CursorMove::Bottom);
                                textarea.move_cursor(CursorMove::End);
                                is_selecting = true;
                            }
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Left {
                                if !is_selecting { textarea.start_selection(); is_selecting = true; }
                                textarea.move_cursor(CursorMove::WordBack);
                            }
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Right {
                                if !is_selecting { textarea.start_selection(); is_selecting = true; }
                                textarea.move_cursor(CursorMove::WordForward);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Left {
                                textarea.cancel_selection(); is_selecting = false;
                                textarea.move_cursor(CursorMove::WordBack);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Right {
                                textarea.cancel_selection(); is_selecting = false;
                                textarea.move_cursor(CursorMove::WordForward);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Backspace {
                                textarea.delete_word(); is_selecting = false; files_modified[idx] = true;
                            }
                            else {
                                match key.code {
                                    KeyCode::Esc => break,
                                    KeyCode::Tab => {
                                        change_view = Some(ActiveView::Actions);
                                    }
                                    KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => {
                                        change_view = Some(ActiveView::Editor(EditorMode::Notes));
                                    }
                                    KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => {
                                        change_view = Some(ActiveView::Editor(EditorMode::Todo));
                                    }
                                    KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => {
                                        change_view = Some(ActiveView::Editor(EditorMode::Logs));
                                    }
                                    KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace |
                                    KeyCode::Delete => {
                                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT { is_selecting = false; }
                                        textarea.input(key); files_modified[idx] = true;
                                    },
                                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT { is_selecting = false; }
                                        textarea.input(key);
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // === РЕЖИМ МЕНЮ ДІЙ ===
                        ActiveView::Actions => {
                            match key.code {
                                KeyCode::Esc => {
                                    change_view = Some(ActiveView::Editor(EditorMode::Notes));
                                }
                                KeyCode::Tab => {
                                    change_view = Some(ActiveView::Editor(EditorMode::Notes));
                                }
                                KeyCode::Down => {
                                    if !commands.is_empty() {
                                        let i = match list_state.selected() {
                                            Some(i) => if i >= commands.len() - 1 { 0 } else { i + 1 },
                                            None => 0,
                                        };
                                        list_state.select(Some(i));
                                    }
                                }
                                KeyCode::Up => {
                                    if !commands.is_empty() {
                                        let i = match list_state.selected() {
                                            Some(i) => if i == 0 { commands.len() - 1 } else { i - 1 },
                                            None => 0,
                                        };
                                        list_state.select(Some(i));
                                    }
                                }
                                KeyCode::Enter => {
                                    if let Some(i) = list_state.selected() {
                                        if i < commands.len() {
                                            let cmd_struct = commands[i].clone();

                                            if cmd_struct.args.contains(&"%INPUT%".to_string()) {
                                                change_view = Some(ActiveView::InputPopup {
                                                    command_idx: i,
                                                    input_buffer: String::new()
                                                });
                                            } else {
                                                change_view = Some(ActiveView::Editor(EditorMode::Logs));
                                                let log_textarea = &mut textareas[2];
                                                log_textarea.insert_str(format!("\n--- Executing (Async): {} ---\n", cmd_struct.name));
                                                files_modified[2] = true;

                                                let tx_cmd = tx.clone();
                                                let cmd_exe = cmd_struct.cmd.clone();
                                                let cmd_args = cmd_struct.args.clone();

                                                thread::spawn(move || {
                                                    let output = Command::new(cmd_exe).args(cmd_args).output();
                                                    let mut result_text = String::new();
                                                    match output {
                                                        Ok(o) => {
                                                            let (decoded_str, _, _) = IBM866.decode(&o.stdout);
                                                            result_text.push_str(&decoded_str);
                                                            if !o.stderr.is_empty() {
                                                                let (err_str, _, _) = IBM866.decode(&o.stderr);
                                                                result_text.push_str("\nERROR:\n");
                                                                result_text.push_str(&err_str);
                                                            }
                                                        },
                                                        Err(e) => { result_text.push_str(&format!("Failed to run: {}", e)); }
                                                    }
                                                    let _ = tx_cmd.send(AppEvent::LogOutput(result_text));
                                                });
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    if let Some(new_view) = change_view {
                        active_view = new_view;
                    }
                }
                _ => {}
            }
        }

        for (i, modified) in files_modified.iter_mut().enumerate() {
            if *modified {
                let text_to_save = textareas[i].lines().join("\n");
                fs::write(file_names[i], text_to_save).ok();
                *modified = false;
            }
        }
    }

    for (i, filename) in file_names.iter().enumerate() {
        let text_to_save = textareas[i].lines().join("\n");
        fs::write(filename, text_to_save)?;
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste)?;
    terminal.show_cursor()?;

    Ok(())
}