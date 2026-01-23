use anyhow::Result;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Gauge, List, ListItem, ListState, Tabs},
    style::{Color, Modifier, Style},
};
use std::{fs, io, net::TcpStream, process::Command, sync::mpsc, thread, time::{Duration, Instant}};
use tui_textarea::{TextArea, CursorMove};
use sysinfo::{System, Networks};
use encoding_rs::IBM866;
use serde::Deserialize;
use arboard::Clipboard;

#[derive(Clone, Deserialize)]
struct Target {
    name: String,
    address: String,
}

#[derive(Clone, Deserialize)]
struct AdminCommand {
    name: String,
    cmd: String,
    args: Vec<String>,
}

#[derive(Deserialize)]
struct AppConfig {
    targets: Vec<Target>,
    commands: Vec<AdminCommand>,
}

#[derive(PartialEq)]
enum ActiveTab {
    Notes,
    Actions,
}

fn main() -> Result<()> {
    // --- 1. ІНІЦІАЛІЗАЦІЯ ---
    let notes_path = "notes.txt";
    let notes_content = fs::read_to_string(notes_path).unwrap_or_default();

    let mut textarea = TextArea::new(notes_content.lines().map(|s| s.to_string()).collect());
    textarea.set_max_histories(10000);
    textarea.set_block(Block::default().borders(Borders::ALL).title(""));

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

    let mut active_tab = ActiveTab::Notes;
    let (tx, rx) = mpsc::channel();

    let mut clipboard = Clipboard::new().ok();

    // Фоновий потік (Мережа)
    thread::spawn(move || {
        loop {
            let mut report = String::new();
            report.push_str("   TARGET       | STATUS | PING \n");
            report.push_str("----------------+--------+------\n");

            for target in &targets_for_thread {
                let start = Instant::now();
                match TcpStream::connect_timeout(&target.address.parse().unwrap_or("0.0.0.0:0".parse().unwrap()), Duration::from_millis(1000)) {
                    Ok(_) => {
                        let duration = start.elapsed().as_millis();
                        report.push_str(&format!("{:<15} |  🟢 ON | {}ms\n", target.name, duration));
                    },
                    Err(_) => {
                        report.push_str(&format!("{:<15} |  🔴 OFF| ---\n", target.name));
                    }
                }
            }
            let _ = tx.send(report);
            thread::sleep(Duration::from_secs(2));
        }
    });

    let mut sys = System::new_all();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Вмикаємо Bracketed Paste для правильної вставки
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut monitor_output = "Scanning network...".to_string();

    let mut should_redraw = true;
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = Instant::now();
    let mut is_selecting = false;

    // <--- НОВА ЗМІННА ДЛЯ АВТОЗБЕРЕЖЕННЯ
    let mut notes_modified = false;

    loop {
        if last_tick.elapsed() >= tick_rate {
            sys.refresh_all();
            if let Ok(new_report) = rx.try_recv() {
                monitor_output = new_report;
            }
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
                    .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .split(f.size());

                let left_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(main_chunks[0]);

                let net_block = Block::default().title(" 📡 Network ").borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan));
                f.render_widget(Paragraph::new(monitor_output.clone()).block(net_block), left_chunks[0]);

                let sys_block = Block::default().title(" 💻 System ").borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow));
                f.render_widget(sys_block, left_chunks[1]);

                let sys_area = left_chunks[1].inner(Margin { vertical: 1, horizontal: 1 });
                let gauge_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Length(2)])
                    .split(sys_area);

                let cpu_gauge = Gauge::default()
                    .gauge_style(Style::default().fg(Color::Green))
                    .ratio((global_cpu_usage as f64 / 100.0).clamp(0.0, 1.0))
                    .label(format!("CPU: {:.1}%", global_cpu_usage));
                f.render_widget(cpu_gauge, gauge_chunks[0]);

                let mem_gauge = Gauge::default()
                    .gauge_style(Style::default().fg(Color::Magenta))
                    .ratio((mem_percentage / 100.0).clamp(0.0, 1.0))
                    .label(format!("RAM: {:.1}%", mem_percentage));
                f.render_widget(mem_gauge, gauge_chunks[2]);

                let right_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(3), Constraint::Min(0)])
                    .split(main_chunks[1]);

                let titles = vec![" [1] Нотатки (Edit) ", " [2] Дії (Execute) "];
                let tabs = Tabs::new(titles)
                    .block(Block::default().borders(Borders::ALL).title(" Меню (Tab) "))
                    .select(match active_tab { ActiveTab::Notes => 0, ActiveTab::Actions => 1 })
                    .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
                f.render_widget(tabs, right_chunks[0]);

                match active_tab {
                    ActiveTab::Notes => {
                        f.render_widget(textarea.widget(), right_chunks[1]);
                    }
                    ActiveTab::Actions => {
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
            })?;

            should_redraw = false;
        }

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());

        if event::poll(timeout)? {
            let evt = event::read()?;

            match evt {
                Event::Paste(data) if active_tab == ActiveTab::Notes => {
                    should_redraw = true;
                    textarea.insert_str(data);
                    notes_modified = true; // <--- Текст змінився (Вставка)
                }

                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    should_redraw = true;

                    match key.code {
                        KeyCode::Esc => break,
                        KeyCode::Tab => {
                            active_tab = match active_tab {
                                ActiveTab::Notes => ActiveTab::Actions,
                                ActiveTab::Actions => ActiveTab::Notes,
                            };
                        }

                        // === КЕРУВАННЯ НОТАТКАМИ ===
                        _ if active_tab == ActiveTab::Notes => {

                            // 1. КОПІЮВАННЯ / ВСТАВКА
                            if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('с')) {
                                textarea.copy();
                                let text = textarea.yank_text();
                                if !text.is_empty() {
                                    if let Some(cb) = &mut clipboard {
                                        let _ = cb.set_text(text);
                                    }
                                }
                            }
                            // Paste (Ctrl+V або Alt+V)
                            else if (key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) ||
                                (key.modifiers == KeyModifiers::ALT && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) {
                                if let Some(cb) = &mut clipboard {
                                    if let Ok(text) = cb.get_text() {
                                        textarea.insert_str(text);
                                        notes_modified = true; // <--- Текст змінився (Вставка з буфера)
                                    }
                                }
                            }
                            // Cut (Ctrl+X)
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('x') || key.code == KeyCode::Char('ч')) {
                                textarea.cut();
                                let text = textarea.yank_text();
                                if let Some(cb) = &mut clipboard {
                                    let _ = cb.set_text(text);
                                }
                                notes_modified = true; // <--- Текст змінився (Вирізання)
                            }
                            // Undo (Ctrl+Z)
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('z') || key.code == KeyCode::Char('я')) {
                                textarea.undo();
                                notes_modified = true; // <--- Текст змінився (Undo)
                            }
                            // Redo (Ctrl+Y)
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('y') || key.code == KeyCode::Char('н')) {
                                textarea.redo();
                                notes_modified = true; // <--- Текст змінився (Redo)
                            }
                            // Select All (Ctrl+A)
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('a') || key.code == KeyCode::Char('ф')) {
                                textarea.move_cursor(CursorMove::Top);
                                textarea.move_cursor(CursorMove::Head);
                                textarea.start_selection();
                                textarea.move_cursor(CursorMove::Bottom);
                                textarea.move_cursor(CursorMove::End);
                                is_selecting = true;
                            }

                            // 2. НАВІГАЦІЯ
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Left {
                                if !is_selecting { textarea.start_selection(); is_selecting = true; }
                                textarea.move_cursor(CursorMove::WordBack);
                            }
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Right {
                                if !is_selecting { textarea.start_selection(); is_selecting = true; }
                                textarea.move_cursor(CursorMove::WordForward);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Left {
                                textarea.cancel_selection();
                                is_selecting = false;
                                textarea.move_cursor(CursorMove::WordBack);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Right {
                                textarea.cancel_selection();
                                is_selecting = false;
                                textarea.move_cursor(CursorMove::WordForward);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Backspace {
                                textarea.delete_word();
                                is_selecting = false;
                                notes_modified = true; // <--- Текст змінився (Видалення слова)
                            }
                            // 3. ЗВИЧАЙНИЙ ВВІД
                            else {
                                match key.code {
                                    KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace |
                                    KeyCode::Delete | KeyCode::Tab => {
                                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                                            is_selecting = false;
                                        }
                                        textarea.input(key);
                                        notes_modified = true; // <--- Текст змінився (Друк)
                                    },
                                    // Навігацію не вважаємо зміною
                                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                                            is_selecting = false;
                                        }
                                        textarea.input(key);
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // === КЕРУВАННЯ МЕНЮ ДІЙ ===
                        KeyCode::Down if active_tab == ActiveTab::Actions => {
                            if !commands.is_empty() {
                                let i = match list_state.selected() {
                                    Some(i) => if i >= commands.len() - 1 { 0 } else { i + 1 },
                                    None => 0,
                                };
                                list_state.select(Some(i));
                            }
                        }
                        KeyCode::Up if active_tab == ActiveTab::Actions => {
                            if !commands.is_empty() {
                                let i = match list_state.selected() {
                                    Some(i) => if i == 0 { commands.len() - 1 } else { i - 1 },
                                    None => 0,
                                };
                                list_state.select(Some(i));
                            }
                        }
                        KeyCode::Enter if active_tab == ActiveTab::Actions => {
                            if let Some(i) = list_state.selected() {
                                if i < commands.len() {
                                    let cmd_struct = &commands[i];
                                    textarea.insert_str(format!("\n--- Executing: {} ---\n", cmd_struct.name));
                                    // Тут теж текст змінюється, зберігаємо
                                    notes_modified = true;

                                    let output = Command::new(&cmd_struct.cmd)
                                        .args(&cmd_struct.args)
                                        .output();

                                    match output {
                                        Ok(o) => {
                                            let (decoded_str, _, _) = IBM866.decode(&o.stdout);
                                            textarea.insert_str(decoded_str);
                                            if !o.stderr.is_empty() {
                                                let (err_str, _, _) = IBM866.decode(&o.stderr);
                                                textarea.insert_str("\nERROR:\n");
                                                textarea.insert_str(err_str);
                                            }
                                        },
                                        Err(e) => {
                                            textarea.insert_str(format!("Failed to run: {}", e));
                                        }
                                    }
                                    textarea.insert_str("\n--------------------------\n");
                                    active_tab = ActiveTab::Notes;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // --- АВТОЗБЕРЕЖЕННЯ ---
        // Якщо текст змінився, зберігаємо файл негайно
        if notes_modified {
            let text_to_save = textarea.lines().join("\n");
            // Використовуємо .ok(), щоб не панікувати, якщо файл зайнятий (спробуємо наступного разу)
            fs::write(notes_path, text_to_save).ok();
            notes_modified = false;
        }
    }

    // Фінальне збереження при виході (про всяк випадок)
    let text_to_save = textarea.lines().join("\n");
    fs::write(notes_path, text_to_save)?;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste)?;
    terminal.show_cursor()?;

    Ok(())
}