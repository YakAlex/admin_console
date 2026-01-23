use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
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
    // --- ІНІЦІАЛІЗАЦІЯ ---
    let notes_path = "notes.txt";
    let notes_content = fs::read_to_string(notes_path).unwrap_or_default();
    let mut textarea = TextArea::new(notes_content.lines().map(|s| s.to_string()).collect());
    textarea.set_block(Block::default().borders(Borders::ALL).title(""));

    let config_path = "config.json";
    let config_data = fs::read_to_string(config_path).unwrap_or_else(|_| {
        r#"{ "targets": [], "commands": [] }"#.to_string()
    });

    let config: AppConfig = serde_json::from_str(&config_data).unwrap_or(AppConfig { targets: vec![], commands: vec![] });

    let targets = config.targets.clone();
    let commands = config.commands.clone();

    let mut list_state = ListState::default();
    if !commands.is_empty() {
        list_state.select(Some(0));
    }

    let mut active_tab = ActiveTab::Notes;
    let (tx, rx) = mpsc::channel();

    // Фоновий потік (Мережа)
    thread::spawn(move || {
        loop {
            let mut report = String::new();
            report.push_str("   TARGET       | STATUS | PING \n");
            report.push_str("----------------+--------+------\n");

            for target in &targets {
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
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut monitor_output = "Scanning network...".to_string();

    // Змінні для оптимізації та логіки
    let mut should_redraw = true;
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = Instant::now();

    // <--- НОВА ЗМІННА: Прапорець, чи ми зараз виділяємо текст
    let mut is_selecting = false;

    loop {
        // 1. ЛОГІКА ТАЙМЕРА
        if last_tick.elapsed() >= tick_rate {
            sys.refresh_all();
            if let Ok(new_report) = rx.try_recv() {
                monitor_output = new_report;
            }
            should_redraw = true;
            last_tick = Instant::now();
        }

        // 2. МАЛЮВАННЯ
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

        // 3. ОБРОБКА ПОДІЙ
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());

        if event::poll(timeout)? {
            let evt = event::read()?;

            match evt {
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
                            // 1. ВИДІЛЕННЯ (Ctrl + Shift + Arrows)
                            if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Left {
                                // Якщо ми ще не виділяли, ставимо "якір"
                                if !is_selecting {
                                    textarea.start_selection();
                                    is_selecting = true;
                                }
                                textarea.move_cursor(CursorMove::WordBack);
                            }
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Right {
                                if !is_selecting {
                                    textarea.start_selection();
                                    is_selecting = true;
                                }
                                textarea.move_cursor(CursorMove::WordForward);
                            }
                            // 2. РУХ БЕЗ ВИДІЛЕННЯ (Ctrl + Arrows)
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Left {
                                textarea.cancel_selection();
                                is_selecting = false; // Скидаємо прапорець
                                textarea.move_cursor(CursorMove::WordBack);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Right {
                                textarea.cancel_selection();
                                is_selecting = false; // Скидаємо прапорець
                                textarea.move_cursor(CursorMove::WordForward);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Backspace {
                                textarea.delete_word();
                                is_selecting = false;
                            }
                            // 3. ЗВИЧАЙНИЙ ВВІД
                            else {
                                match key.code {
                                    KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace |
                                    KeyCode::Delete | KeyCode::Tab |
                                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                                        // При будь-якому вводі або простому русі виділення скидається
                                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                                            // Якщо просто стрілки або Shift+Буква - це введення/навігація, скидаємо "режим виділення слів"
                                            // Хоча tui-textarea сама тримає виділення при Shift+Arrows (посимвольно),
                                            // наш прапорець is_selecting для слів треба скинути, якщо ми клікаємо щось інше.
                                            // Але tui-textarea сама скидає виділення при move_cursor без start_selection.
                                            is_selecting = false;
                                        }
                                        textarea.input(key);
                                    },
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
    }

    let text_to_save = textarea.lines().join("\n");
    fs::write(notes_path, text_to_save)?;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}