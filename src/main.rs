mod config;
mod types;
mod utils;
mod monitor; // <--- Підключаємо модуль
mod ui;      // <--- Підключаємо модуль

use anyhow::Result;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::{Block, Borders, ListState, TableState}, style::{Color, Style}};
use std::{fs, io, process::Command, sync::mpsc, thread, time::{Duration, Instant}};
use tui_textarea::{TextArea, CursorMove};
use encoding_rs::IBM866;
use arboard::Clipboard;
use chrono::Local;

use crate::config::AppConfig;
use crate::types::{ServerStatus, AppEvent, EditorMode, ActiveView, MonitorCommand, Task, WizardStep};
use crate::utils::{is_valid_time, parse_tasks_from_text};
// Імпортуємо функції з нових файлів
use crate::monitor::start_monitor;
use crate::ui::draw;

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
    let config_data = fs::read_to_string(config_path).unwrap_or_else(|_| r#"{ "targets": [], "commands": [] }"#.to_string());
    let config: AppConfig = serde_json::from_str(&config_data).unwrap_or(AppConfig { targets: vec![], commands: vec![] });

    // --- ЗАВАНТАЖЕННЯ ДАНИХ (Sync Text -> JSON) ---
    let todo_content = fs::read_to_string("todo.txt").unwrap_or_default();
    let mut tasks = parse_tasks_from_text(&todo_content);

    // Резерв: якщо todo.txt пустий, пробуємо взяти з json
    if tasks.is_empty() {
        let tasks_path = "tasks.json";
        let tasks_data = fs::read_to_string(tasks_path).unwrap_or_else(|_| "[]".to_string());
        tasks = serde_json::from_str(&tasks_data).unwrap_or(Vec::new());
    }

    let mut list_state = ListState::default();
    if !config.commands.is_empty() { list_state.select(Some(0)); }

    let mut table_state = TableState::default();
    if !config.targets.is_empty() { table_state.select(Some(0)); }

    let mut active_view = ActiveView::Editor(EditorMode::Notes);
    let (tx, rx) = mpsc::channel::<AppEvent>();
    let (tx_to_monitor, rx_from_main) = mpsc::channel::<MonitorCommand>();

    let mut clipboard = Clipboard::new().ok();
    let mut last_user_activity = Instant::now();
    let mut files_modified = vec![false, false, false];
    let mut tasks_modified = false;

    // --- ЗАПУСК МОНІТОРА (ЗАМІСТЬ ВЕЛИКОГО БЛОКУ thread::spawn) ---
    // Ми просто викликаємо функцію, передаючи туди копії даних
    start_monitor(config.targets.clone(), tasks.clone(), tx.clone(), rx_from_main);

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

    loop {
        // --- СИНХРОНІЗАЦІЯ (TEXT -> JSON) ---
        if files_modified[1] {
            let content = textareas[1].lines().join("\n");
            tasks = parse_tasks_from_text(&content);
            tasks_modified = true;
            let _ = tx_to_monitor.send(MonitorCommand::UpdateTasks(tasks.clone()));
        }

        // --- ОБРОБКА ПОДІЙ ВІД ПОТОКІВ ---
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::ServerUpdate(data) => { server_data = data; should_redraw = true; }
                AppEvent::LogOutput(text) => {
                    let log_textarea = &mut textareas[2];
                    if text.starts_with('[') { log_textarea.insert_str(text); }
                    else { log_textarea.insert_str(format!("[{}] Output:\n{}", Local::now().format("%H:%M:%S"), text)); }
                    log_textarea.insert_str("\n-------------------------------------------\n");
                    files_modified[2] = true; should_redraw = true;
                }
                AppEvent::TaskCompleted(title) => {
                    // Логіка зміни тексту залишається тут, бо `textareas` живуть у main
                    let todo_textarea = &mut textareas[1];
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

                        // Примусова синхронізація
                        let content = todo_textarea.lines().join("\n");
                        tasks = parse_tasks_from_text(&content);
                        let _ = tx_to_monitor.send(MonitorCommand::UpdateTasks(tasks.clone()));

                        files_modified[1] = true;
                        tasks_modified = true;
                        should_redraw = true;
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate { should_redraw = true; last_tick = Instant::now(); }

        // --- МАЛЮВАННЯ (ЗАМІСТЬ ВЕЛИКОГО БЛОКУ terminal.draw) ---
        if should_redraw {
            terminal.draw(|f| {
                // Викликаємо функцію з ui.rs
                draw(f, &textareas, &server_data, &tasks, &active_view, &mut table_state, &mut list_state, &config.commands, &titles);
            })?;
            should_redraw = false;
        }

        // --- ВВІД КОРИСТУВАЧА (Ця частина залишається великою) ---
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            let evt = event::read()?;
            match evt {
                Event::Paste(data) => {
                    last_user_activity = Instant::now();
                    if let ActiveView::Editor(mode) = active_view {
                        textareas[mode as usize].insert_str(data);
                        files_modified[mode as usize] = true; should_redraw = true;
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    last_user_activity = Instant::now();
                    should_redraw = true;
                    let mut change_view = None;

                    if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('q') || key.code == KeyCode::Char('й')) { break; }
                    if key.modifiers == KeyModifiers::ALT && (key.code == KeyCode::Char('t') || key.code == KeyCode::Char('е')) {
                        change_view = Some(ActiveView::TodoWizard { step: WizardStep::Title, buffer: String::new(), temp_title: String::new(), temp_desc: String::new() });
                    }

                    match &mut active_view {
                        ActiveView::Search { mode_return_to, query } => {
                            let idx = *mode_return_to as usize;
                            match key.code {
                                KeyCode::Esc => { textareas[idx].set_search_pattern("").ok(); change_view = Some(ActiveView::Editor(*mode_return_to)); }
                                KeyCode::Enter => { textareas[idx].search_forward(false); }
                                KeyCode::Backspace => { query.pop(); textareas[idx].set_search_pattern(query.as_str()).ok(); }
                                KeyCode::Char(c) => { query.push(c); textareas[idx].set_search_pattern(query.as_str()).ok(); textareas[idx].search_forward(false); }
                                _ => {}
                            }
                        }
                        ActiveView::InputPopup { command_idx, input_buffer } => {
                            match key.code {
                                KeyCode::Enter => {
                                    let idx_copy = *command_idx;
                                    if idx_copy < config.commands.len() {
                                        let cmd_struct = config.commands[idx_copy].clone();
                                        let buffer_clone = input_buffer.clone();
                                        let final_args: Vec<String> = cmd_struct.args.iter().map(|arg| if arg == "%INPUT%" { buffer_clone.clone() } else { arg.clone() }).collect();
                                        change_view = Some(ActiveView::Editor(EditorMode::Logs));
                                        let tx_cmd = tx.clone();
                                        let cmd_exe = cmd_struct.cmd.clone();
                                        thread::spawn(move || {
                                            let output = Command::new(cmd_exe).args(final_args).output();
                                            let mut result_text = String::new();
                                            match output {
                                                Ok(o) => {
                                                    let (decoded_str, _, _) = IBM866.decode(&o.stdout);
                                                    result_text.push_str(&decoded_str);
                                                    if !o.stderr.is_empty() { let (err_str, _, _) = IBM866.decode(&o.stderr); result_text.push_str("\nERROR:\n"); result_text.push_str(&err_str); }
                                                },
                                                Err(e) => { result_text.push_str(&format!("Failed to run: {}", e)); }
                                            }
                                            let text = result_text.trim();
                                            if !text.is_empty() { let _ = tx_cmd.send(AppEvent::LogOutput(text.to_string())); }
                                        });
                                    }
                                }
                                KeyCode::Esc => { change_view = Some(ActiveView::Actions); }
                                KeyCode::Backspace => { input_buffer.pop(); }
                                KeyCode::Char(c) => { input_buffer.push(c); }
                                _ => {}
                            }
                        }
                        ActiveView::TodoWizard { step, buffer, temp_title, temp_desc } => {
                            match key.code {
                                KeyCode::Esc => { change_view = Some(ActiveView::Editor(EditorMode::Todo)); }
                                KeyCode::Backspace => { buffer.pop(); }
                                KeyCode::Char(c) => { buffer.push(c); }
                                KeyCode::Enter => {
                                    match step {
                                        WizardStep::Title => { if !buffer.is_empty() { *temp_title = buffer.clone(); buffer.clear(); *step = WizardStep::Description; } }
                                        WizardStep::Description => { *temp_desc = buffer.clone(); buffer.clear(); *step = WizardStep::Time; }
                                        WizardStep::Time => {
                                            if is_valid_time(&buffer) {
                                                let time_str = buffer.trim().to_string();
                                                let display_str = if time_str.is_empty() { format!("- [ ] {}\n      {}", temp_title, temp_desc) } else { format!("- [{}] {}\n      {}", time_str, temp_title, temp_desc) };
                                                let todo_area = &mut textareas[1];
                                                todo_area.move_cursor(CursorMove::Bottom);
                                                if !todo_area.lines().is_empty() { todo_area.insert_str("\n"); }
                                                todo_area.insert_str(display_str);
                                                files_modified[1] = true;
                                                change_view = Some(ActiveView::Editor(EditorMode::Todo));
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        ActiveView::Editor(mode) => {
                            let idx = *mode as usize;
                            let textarea = &mut textareas[idx];
                            // ... Тут довгий блок обробки клавіш (Ctrl+C, Ctrl+V, і т.д.) ...
                            // Він залишається без змін, бо це логіка контролера
                            if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('f') || key.code == KeyCode::Char('а')) { change_view = Some(ActiveView::Search { mode_return_to: *mode, query: String::new() }); }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('с')) { textarea.copy(); if let Some(cb) = &mut clipboard { let _ = cb.set_text(textarea.yank_text()); } }
                            else if (key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) || (key.modifiers == KeyModifiers::ALT && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) { if let Some(cb) = &mut clipboard { if let Ok(text) = cb.get_text() { textarea.insert_str(text); files_modified[idx] = true; } } }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('x') || key.code == KeyCode::Char('ч')) { textarea.cut(); if let Some(cb) = &mut clipboard { let _ = cb.set_text(textarea.yank_text()); } files_modified[idx] = true; }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('z') || key.code == KeyCode::Char('я')) { textarea.undo(); files_modified[idx] = true; }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('y') || key.code == KeyCode::Char('н')) { textarea.redo(); files_modified[idx] = true; }
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('a') || key.code == KeyCode::Char('ф')) { textarea.move_cursor(CursorMove::Top); textarea.move_cursor(CursorMove::Head); textarea.start_selection(); textarea.move_cursor(CursorMove::Bottom); textarea.move_cursor(CursorMove::End); is_selecting = true; }
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Left { if !is_selecting { textarea.start_selection(); is_selecting = true; } textarea.move_cursor(CursorMove::WordBack); }
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Right { if !is_selecting { textarea.start_selection(); is_selecting = true; } textarea.move_cursor(CursorMove::WordForward); }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Left { textarea.cancel_selection(); is_selecting = false; textarea.move_cursor(CursorMove::WordBack); }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Right { textarea.cancel_selection(); is_selecting = false; textarea.move_cursor(CursorMove::WordForward); }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Backspace { textarea.delete_word(); is_selecting = false; files_modified[idx] = true; }
                            else {
                                match key.code {
                                    KeyCode::Esc => break,
                                    KeyCode::Tab => { change_view = Some(ActiveView::Actions); }
                                    KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                                    KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Todo)); }
                                    KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Logs)); }
                                    KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete => { if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT { is_selecting = false; } textarea.input(key); files_modified[idx] = true; },
                                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => { if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT { is_selecting = false; } textarea.input(key); }
                                    _ => {}
                                }
                            }
                        }
                        ActiveView::Actions => {
                            match key.code {
                                KeyCode::Esc => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                                KeyCode::Tab => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                                KeyCode::Down => { if !config.commands.is_empty() { let i = match list_state.selected() { Some(i) => if i >= config.commands.len() - 1 { 0 } else { i + 1 }, None => 0, }; list_state.select(Some(i)); } }
                                KeyCode::Up => { if !config.commands.is_empty() { let i = match list_state.selected() { Some(i) => if i == 0 { config.commands.len() - 1 } else { i - 1 }, None => 0, }; list_state.select(Some(i)); } }
                                KeyCode::Enter => {
                                    if let Some(i) = list_state.selected() {
                                        if i < config.commands.len() {
                                            let cmd_struct = config.commands[i].clone();
                                            if cmd_struct.args.contains(&"%INPUT%".to_string()) {
                                                change_view = Some(ActiveView::InputPopup { command_idx: i, input_buffer: String::new() });
                                            } else {
                                                change_view = Some(ActiveView::Editor(EditorMode::Logs));
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
                                                            if !o.stderr.is_empty() { let (err_str, _, _) = IBM866.decode(&o.stderr); result_text.push_str("\nERROR:\n"); result_text.push_str(&err_str); }
                                                        },
                                                        Err(e) => { result_text.push_str(&format!("Failed to run: {}", e)); }
                                                    }
                                                    let text = result_text.trim();
                                                    if !text.is_empty() { let _ = tx_cmd.send(AppEvent::LogOutput(text.to_string())); }
                                                });
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    if let Some(new_view) = change_view { active_view = new_view; }
                }
                _ => {}
            }
        }

        // --- АВТОЗБЕРЕЖЕННЯ ---
        if last_user_activity.elapsed() >= Duration::from_secs(30) {
            for (i, modified) in files_modified.iter_mut().enumerate() {
                if *modified {
                    let text_to_save = textareas[i].lines().join("\n");
                    fs::write(file_names[i], text_to_save).ok();
                    *modified = false;
                }
            }
            if tasks_modified {
                let _ = fs::write("tasks.json", serde_json::to_string_pretty(&tasks).unwrap_or_default());
                tasks_modified = false;
            }
        }
    }

    // --- ФІНАЛЬНЕ ЗБЕРЕЖЕННЯ ---
    for (i, filename) in file_names.iter().enumerate() { let text_to_save = textareas[i].lines().join("\n"); fs::write(filename, text_to_save)?; }
    if tasks_modified { let _ = fs::write("tasks.json", serde_json::to_string_pretty(&tasks).unwrap_or_default()); }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste)?;
    terminal.show_cursor()?;
    Ok(())
}