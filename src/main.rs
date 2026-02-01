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
    widgets::{Block, Borders, Paragraph, Gauge, List, ListItem, ListState, Tabs, Table, Row, Cell, Clear, TableState},
    style::{Color, Modifier, Style},
};
use std::{fs, io, net::TcpStream, process::Command, sync::mpsc, thread, time::{Duration, Instant}, collections::VecDeque};
use tui_textarea::{TextArea, CursorMove};

use encoding_rs::IBM866;
use arboard::Clipboard;
use chrono::Local;
use notify_rust::Notification;

use crate::config::AppConfig;
use crate::types::{ServerStatus, AppEvent, EditorMode, ActiveView, MonitorCommand, Task, WizardStep};
use crate::utils::{centered_rect, is_valid_time, parse_tasks_from_text};

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

    // --- ЗАВАНТАЖЕННЯ ЗАВДАНЬ (JSON) ---
    let tasks_path = "tasks.json";
    let tasks_data = fs::read_to_string(tasks_path).unwrap_or_else(|_| "[]".to_string());
    // Якщо файл порожній або битий, створюємо пустий список
    let mut tasks: Vec<Task> = serde_json::from_str(&tasks_data).unwrap_or(Vec::new());

    // Копії для передачі в потік
    let tasks_for_thread = tasks.clone();
    let targets_for_thread = config.targets.clone();
    let commands = config.commands.clone();

    let mut list_state = ListState::default();
    if !commands.is_empty() { list_state.select(Some(0)); }

    let mut table_state = TableState::default();
    if !targets_for_thread.is_empty() { table_state.select(Some(0)); }

    let mut active_view = ActiveView::Editor(EditorMode::Notes);
    let (tx, rx) = mpsc::channel::<AppEvent>();
    // Канал для комунікації з монітором
    let (tx_to_monitor, rx_from_main) = mpsc::channel::<MonitorCommand>();

    let mut clipboard = Clipboard::new().ok();
    let tx_monitor = tx.clone();
    let mut last_user_activity = Instant::now(); // Коли користувач востаннє щось натискав
    let mut tasks_modified = false;              // Чи змінювали ми список завдань

    // --- ФОНОВИЙ ПОТІК (МОНІТОРИНГ + НАГАДУВАННЯ) ---
    thread::spawn(move || {
        let mut statuses: Vec<ServerStatus> = targets_for_thread.iter().map(|t| ServerStatus {
            name: t.name.clone(),
            is_online: false,
            latency: 0,
            history: VecDeque::from(vec![0; 20]),
        }).collect();

        // Локальна копія завдань та змінна часу
        let mut thread_tasks = tasks_for_thread;
        let mut last_checked_minute = String::new();

        let mut current_targets = targets_for_thread.clone();
        let mut previous_online_status: Vec<bool> = vec![true; current_targets.len()];

        loop {
            // 1. Отримуємо оновлення від Main
            while let Ok(cmd) = rx_from_main.try_recv() {
                match cmd {
                    MonitorCommand::UpdateTargets(new_targets) => {
                        current_targets = new_targets;
                    }
                    MonitorCommand::UpdateTasks(new_tasks) => {
                        thread_tasks = new_tasks; // Оновлюємо список завдань
                    }
                }
            }

            // 2. ПІНГ СЕРВЕРІВ
            for (i, target) in current_targets.iter().enumerate() {
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

                if previous_online_status[i] && !online {
                    let timestamp = Local::now().format("%H:%M:%S");
                    let log_msg = format!("[{}] 🔴 ALERT: Server '{}' went OFFLINE!", timestamp, target.name);
                    let _ = tx_monitor.send(AppEvent::LogOutput(log_msg));
                    Notification::new().summary("SERVER DOWN ⚠️").body(&format!("Увага! Сервер '{}' перестав відповідати.", target.name)).appname("Admin Console").show().ok();
                }
                previous_online_status[i] = online;
            }
            let _ = tx_monitor.send(AppEvent::ServerUpdate(statuses.clone()));

            // 3. ПЕРЕВІРКА НАГАДУВАНЬ
            let current_time_str = Local::now().format("%H:%M").to_string();

            if current_time_str != last_checked_minute {
                for task in &thread_tasks {
                    // Перевіряємо, чи настав час і чи завдання ще не виконане
                    if !task.completed && !task.time.is_empty() && task.time == current_time_str {
                        // 1. Показуємо повідомлення
                        Notification::new()
                            .summary(&format!("🔔 Reminder: {}", task.title))
                            .body(&task.description)
                            .appname("Admin Console")
                            .show()
                            .ok();

                        // 2. Надсилаємо сигнал у головний потік, щоб помітити завдання як виконане
                        let _ = tx_monitor.send(AppEvent::TaskCompleted(task.title.clone()));
                    }
                }
                last_checked_minute = current_time_str;
            }

            thread::sleep(Duration::from_secs(1));
        }
    });


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
        // --- СИНХРОНІЗАЦІЯ (TEXT -> JSON) ---
        // Якщо текст Todo (індекс 1) змінився, ми перечитуємо tasks
        // Це покриває і ручне редагування, і додавання через Wizard
        if files_modified[1] {
            let content = textareas[1].lines().join("\n");
            tasks = parse_tasks_from_text(&content);
            tasks_modified = true; // Щоб потім зберегти json

            // Одразу оновлюємо монітор, щоб нагадування працювали коректно
            let _ = tx_to_monitor.send(MonitorCommand::UpdateTasks(tasks.clone()));
        }
        // --- ОБРОБКА ПОДІЙ З КАНАЛУ ---
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::ServerUpdate(data) => {
                    server_data = data;
                    should_redraw = true;
                }
                AppEvent::LogOutput(text) => {
                    let log_textarea = &mut textareas[2];
                    if text.starts_with('[') {
                        log_textarea.insert_str(text);
                    } else {
                        let timestamp = Local::now().format("%H:%M:%S");
                        log_textarea.insert_str(format!("[{}] Output:\n{}", timestamp, text));
                    }
                    log_textarea.insert_str("\n-------------------------------------------\n");
                    files_modified[2] = true;
                    should_redraw = true;
                }

                AppEvent::TaskCompleted(title) => {
                    let todo_textarea = &mut textareas[1];
                    let old_lines = todo_textarea.lines().to_vec();

                    let mut new_lines = Vec::new();
                    let mut modified = false;

                    for line in old_lines {
                        // Шукаємо невиконане завдання з такою назвою
                        if line.contains(&title) && line.trim().starts_with("- [") && !line.contains("[x]") && !line.contains("[X]") {
                            let start_bracket = line.find('[').unwrap_or(0);
                            let end_bracket = line.find(']').unwrap_or(line.len());

                            if end_bracket > start_bracket {
                                // Замінюємо час на [x]
                                let new_line = format!("{}[x]{}", &line[..start_bracket], &line[end_bracket + 1..]);
                                new_lines.push(new_line);
                                modified = true;
                            } else {
                                new_lines.push(line);
                            }
                        } else {
                            new_lines.push(line);
                        }
                    }

                    if modified {
                        // 1. Оновлюємо візуальний текст
                        *todo_textarea = TextArea::new(new_lines);
                        todo_textarea.set_block(Block::default().borders(Borders::ALL));

                        // 2. ПРИМУСОВА СИНХРОНІЗАЦІЯ (ТУТ БУЛА ПРОБЛЕМА)
                        // Ми не чекаємо циклу, а парсимо текст прямо зараз
                        let content = todo_textarea.lines().join("\n");
                        tasks = parse_tasks_from_text(&content);

                        // 3. Відправляємо оновлений список монітору (щоб він перестав нагадувати)
                        let _ = tx_to_monitor.send(MonitorCommand::UpdateTasks(tasks.clone()));

                        // 4. Позначаємо, що треба зберегти JSON і Текст
                        files_modified[1] = true;
                        tasks_modified = true;
                        should_redraw = true;
                    }
                }

            }
        }

        if last_tick.elapsed() >= tick_rate {
            should_redraw = true;
            last_tick = Instant::now();
        }

        if should_redraw {
            terminal.draw(|f| {
                let main_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                    .split(f.area());

                // ЗМІНА 1: Збільшили висоту нижнього блоку з 6 до 8, щоб влізло 5 рядків
                let left_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(10), Constraint::Length(8)])
                    .split(main_chunks[0]);

                // --- TABLE (СЕРВЕРИ) ---
                let header_cells = ["Server", "Ping", "Status"].iter().map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));
                let header = Row::new(header_cells).height(1).bottom_margin(1);
                let rows = server_data.iter().map(|item| {
                    let ping_text = if item.is_online { format!("{}ms", item.latency) } else { "---".to_string() };
                    let status_symbol = if item.is_online { "🟢" } else { "🔴" };
                    let color = if !item.is_online { Color::Red } else if item.latency > 100 { Color::Yellow } else { Color::Green };
                    let cells = vec![
                        Cell::from(item.name.clone()),
                        Cell::from(ping_text).style(Style::default().fg(color)),
                        Cell::from(status_symbol),
                    ];
                    Row::new(cells).height(1)
                });
                let table = Table::new(rows, [Constraint::Percentage(50), Constraint::Percentage(30), Constraint::Min(10)])
                    .header(header)
                    .block(Block::default().borders(Borders::ALL).title(" 📡 Servers "));
                f.render_stateful_widget(table, left_chunks[0], &mut table_state);

                // --- SCHEDULE (НОВИЙ БЛОК ЗАМІСТЬ SYSTEM) ---
                let mut active_tasks: Vec<&Task> = tasks.iter().filter(|t| !t.completed).collect();

                // Сортування (Час -> Без часу)
                active_tasks.sort_by(|a, b| {
                    let a_has_time = !a.time.is_empty();
                    let b_has_time = !b.time.is_empty();
                    if a_has_time && b_has_time { a.time.cmp(&b.time) }
                    else if a_has_time { std::cmp::Ordering::Less }
                    else if b_has_time { std::cmp::Ordering::Greater }
                    else { a.title.cmp(&b.title) }
                });

                let mut items = Vec::new();
                let mut first_untimed_seen = false;

                // Беремо до 6 елементів, щоб якщо є розділювач, він вліз
                for (i, task) in active_tasks.iter().take(5).enumerate() {
                    let has_time = !task.time.is_empty();

                    // --- РОЗДІЛЮВАЧ ---
                    // Якщо ми перейшли від "з часом" до "без часу" — малюємо лінію
                    if i > 0 && !has_time && !first_untimed_seen {
                        items.push(ListItem::new(" ──────────────────────").style(Style::default().fg(Color::DarkGray)));
                        first_untimed_seen = true;
                    }

                    // --- ФОРМАТУВАННЯ ---
                    let (prefix, style) = if has_time {
                        // ⏰ 14:00 (разом з іконкою займає фіксовану ширину)
                        (format!(" ⏰ {} │ ", task.time), Style::default().fg(Color::Yellow))
                    } else {
                        // 📝  --   (підганяємо пробіли, щоб вертикальна лінія │ співпала)
                        (" 📝  --   │ ".to_string(), Style::default().fg(Color::Cyan))
                    };

                    // Обрізаємо назву, якщо вона довга
                    let title = if task.title.len() > 18 {
                        format!("{}..", &task.title[..18])
                    } else {
                        task.title.clone()
                    };

                    items.push(ListItem::new(format!("{}{}", prefix, title)).style(style));

                    // Якщо це перше завдання без часу (і воно найперше в списку), помічаємо це
                    if !has_time { first_untimed_seen = true; }
                }

                let list_widget = if items.is_empty() {
                    List::new(vec![ListItem::new("   (No active tasks)").style(Style::default().fg(Color::DarkGray))])
                } else {
                    List::new(items)
                };

                let schedule_block = list_widget
                    .block(Block::default().borders(Borders::ALL).title(" 📅 Schedule "))
                    .highlight_style(Style::default().add_modifier(Modifier::BOLD));

                f.render_widget(schedule_block, left_chunks[1]);

                let right_chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(3), Constraint::Min(0)]).split(main_chunks[1]);

                // TABS
                let (current_file_idx, is_actions_active) = match active_view {
                    ActiveView::Editor(mode) => (mode as usize, false),
                    ActiveView::Search { mode_return_to, .. } => (mode_return_to as usize, false),
                    ActiveView::Actions => (0, true),
                    ActiveView::InputPopup { .. } => (0, true),
                    // Підсвічуємо вкладку Todo, якщо ми в режимі створення завдання
                    ActiveView::TodoWizard { .. } => (1, true),
                };
                let file_tabs = Tabs::new(titles.clone()).block(Block::default().borders(Borders::BOTTOM)).select(if !is_actions_active { current_file_idx } else { 99 }).highlight_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
                f.render_widget(file_tabs, right_chunks[0]);

                let action_status = if is_actions_active { Paragraph::new(" [TAB] ACTIONS ").style(Style::default().fg(Color::Black).bg(Color::Yellow)) } else { Paragraph::new(" [TAB] Actions | [ALT+T] New Task") };
                f.render_widget(action_status, Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(70), Constraint::Percentage(30)]).split(right_chunks[0])[1]);

                // MAIN CONTENT
                match &active_view {
                    ActiveView::Editor(mode) | ActiveView::Search { mode_return_to: mode, .. } => {
                        f.render_widget(&textareas[*mode as usize], right_chunks[1]);
                    }
                    ActiveView::Actions | ActiveView::InputPopup { .. } => {
                        let items: Vec<ListItem> = commands.iter().map(|i| ListItem::new(i.name.clone()).style(Style::default().fg(Color::White))).collect();
                        let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Оберіть команду ")).highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD)).highlight_symbol(">> ");
                        f.render_stateful_widget(list, right_chunks[1], &mut list_state);
                    }
                    // Якщо відкритий візард, на фоні показуємо список Todo
                    ActiveView::TodoWizard { .. } => {
                        f.render_widget(&textareas[1], right_chunks[1]);
                    }
                }

                // POPUPS (Search, Input)
                if let ActiveView::Search { query, .. } = &active_view {
                    let search_area = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(0), Constraint::Length(3)]).split(right_chunks[1])[1];
                    f.render_widget(Clear, search_area);
                    f.render_widget(Paragraph::new(format!("Search: {}", query)).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan))).style(Style::default().fg(Color::Yellow).bg(Color::Black)), search_area);
                }
                if let ActiveView::InputPopup { input_buffer, .. } = &active_view {
                    let area = centered_rect(60, 20, f.area());
                    f.render_widget(Clear, area);
                    f.render_widget(Paragraph::new(input_buffer.clone()).block(Block::default().borders(Borders::ALL).title(" Введіть аргумент (IP/Host) ")).style(Style::default().fg(Color::Yellow).bg(Color::Black)), area);
                }

                // --- TODO WIZARD POPUP (ВІЗУАЛІЗАЦІЯ) ---
                if let ActiveView::TodoWizard { step, buffer, temp_title, .. } = &active_view {
                    let area = centered_rect(60, 20, f.area());
                    f.render_widget(Clear, area);

                    let (title, content) = match step {
                        WizardStep::Title => (" 1/3: Назва завдання ", format!("Введіть назву:\n\n> {}", buffer)),
                        WizardStep::Description => (" 2/3: Опис ", format!("Назва: {}\n\nВведіть опис (можна пустий):\n> {}", temp_title, buffer)),
                        WizardStep::Time => (" 3/3: Час нагадування ", format!("Назва: {}\n\nВведіть час (HH:MM) або Enter щоб пропустити:\n> {}", temp_title, buffer)),
                    };

                    let block = Paragraph::new(content)
                        .block(Block::default().borders(Borders::ALL).title(title))
                        .style(Style::default().fg(Color::Cyan).bg(Color::Black));
                    f.render_widget(block, area);
                }

            })?;
            should_redraw = false;
        }

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
                    // --- GLOBAL SHORTCUT: ALT + T (ЗАПУСК ВІЗАРДА) ---
                    if key.modifiers == KeyModifiers::ALT && (key.code == KeyCode::Char('t') || key.code == KeyCode::Char('е')) {
                        change_view = Some(ActiveView::TodoWizard {
                            step: WizardStep::Title,
                            buffer: String::new(),
                            temp_title: String::new(),
                            temp_desc: String::new()
                        });
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
                                    if idx_copy < commands.len() {
                                        let cmd_struct = commands[idx_copy].clone();
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

                        // --- TODO WIZARD LOGIC (ОБРОБКА ВВОДУ) ---
                        ActiveView::TodoWizard { step, buffer, temp_title, temp_desc } => {
                            match key.code {
                                KeyCode::Esc => { change_view = Some(ActiveView::Editor(EditorMode::Todo)); }
                                KeyCode::Backspace => { buffer.pop(); }
                                KeyCode::Char(c) => { buffer.push(c); }
                                KeyCode::Enter => {
                                    match step {
                                        WizardStep::Title => {
                                            if !buffer.is_empty() {
                                                *temp_title = buffer.clone();
                                                buffer.clear();
                                                *step = WizardStep::Description;
                                            }
                                        }
                                        WizardStep::Description => {
                                            *temp_desc = buffer.clone();
                                            buffer.clear();
                                            *step = WizardStep::Time;
                                        }
                                        WizardStep::Time => {
                                            // 1. ВАЛІДАЦІЯ: Перевіряємо час
                                            // Якщо час валідний — виконуємо логіку.
                                            // Якщо ні — просто ігноруємо натискання Enter (користувач залишиться на цьому етапі)
                                            if is_valid_time(&buffer) {
                                                let time_str = buffer.trim().to_string();

                                                // 2. Формуємо рядок для todo.txt (БЕЗ іконок, просто [14:00])
                                                let display_str = if time_str.is_empty() {
                                                    format!("- [ ] {}\n      {}", temp_title, temp_desc)
                                                } else {
                                                    format!("- [{}] {}\n      {}", time_str, temp_title, temp_desc)
                                                };

                                                // 3. Додаємо ТІЛЬКИ в текстовий редактор
                                                let todo_area = &mut textareas[1];
                                                todo_area.move_cursor(CursorMove::Bottom);
                                                // Додаємо відступ, якщо файл не пустий
                                                if !todo_area.lines().is_empty() { todo_area.insert_str("\n"); }
                                                todo_area.insert_str(display_str);

                                                // Ставимо прапорець, що файл змінився
                                                files_modified[1] = true;

                                                // Виходимо з візарда
                                                change_view = Some(ActiveView::Editor(EditorMode::Todo));
                                            }
                                            // else { тут нічого не робимо, користувач далі редагує buffer }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }

                        ActiveView::Editor(mode) => {
                            let idx = *mode as usize;
                            let textarea = &mut textareas[idx];

                            // --- CTRL+F (Пошук) ---
                            if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('f') || key.code == KeyCode::Char('а')) {
                                change_view = Some(ActiveView::Search { mode_return_to: *mode, query: String::new() });
                            }
                            // --- CTRL+C (Копіювати) ---
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('с')) {
                                textarea.copy();
                                if let Some(cb) = &mut clipboard { let _ = cb.set_text(textarea.yank_text()); }
                            }
                            // --- CTRL+V (Вставити) ---
                            else if (key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) ||
                                (key.modifiers == KeyModifiers::ALT && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) {
                                if let Some(cb) = &mut clipboard {
                                    if let Ok(text) = cb.get_text() { textarea.insert_str(text); files_modified[idx] = true; }
                                }
                            }
                            // --- CTRL+X (Вирізати) ---
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('x') || key.code == KeyCode::Char('ч')) {
                                textarea.cut();
                                if let Some(cb) = &mut clipboard { let _ = cb.set_text(textarea.yank_text()); }
                                files_modified[idx] = true;
                            }
                            // --- CTRL+Z (Undo) ---
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('z') || key.code == KeyCode::Char('я')) {
                                textarea.undo(); files_modified[idx] = true;
                            }
                            // --- CTRL+Y (Redo) ---
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('y') || key.code == KeyCode::Char('н')) {
                                textarea.redo(); files_modified[idx] = true;
                            }
                            // --- CTRL+A (Виділити все) [ПОВЕРНУЛИ] ---
                            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('a') || key.code == KeyCode::Char('ф')) {
                                textarea.move_cursor(CursorMove::Top);
                                textarea.move_cursor(CursorMove::Head);
                                textarea.start_selection();
                                textarea.move_cursor(CursorMove::Bottom);
                                textarea.move_cursor(CursorMove::End);
                                is_selecting = true;
                            }
                            // --- CTRL+SHIFT+ARROWS (Виділення по словах) [ПОВЕРНУЛИ] ---
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Left {
                                if !is_selecting { textarea.start_selection(); is_selecting = true; }
                                textarea.move_cursor(CursorMove::WordBack);
                            }
                            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Right {
                                if !is_selecting { textarea.start_selection(); is_selecting = true; }
                                textarea.move_cursor(CursorMove::WordForward);
                            }
                            // --- CTRL+ARROWS (Рух по словах) [ПОВЕРНУЛИ] ---
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Left {
                                textarea.cancel_selection(); is_selecting = false;
                                textarea.move_cursor(CursorMove::WordBack);
                            }
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Right {
                                textarea.cancel_selection(); is_selecting = false;
                                textarea.move_cursor(CursorMove::WordForward);
                            }
                            // --- CTRL+BACKSPACE (Видалити слово) [ПОВЕРНУЛИ] ---
                            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Backspace {
                                textarea.delete_word(); is_selecting = false; files_modified[idx] = true;
                            }
                            // --- ЗВИЧАЙНИЙ ВВІД ---
                            else {
                                match key.code {
                                    KeyCode::Esc => break,
                                    KeyCode::Tab => { change_view = Some(ActiveView::Actions); }
                                    KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                                    KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Todo)); }
                                    KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Logs)); }
                                    KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete => {
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
                        ActiveView::Actions => {
                            match key.code {
                                KeyCode::Esc => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                                KeyCode::Tab => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                                KeyCode::Down => { if !commands.is_empty() { let i = match list_state.selected() { Some(i) => if i >= commands.len() - 1 { 0 } else { i + 1 }, None => 0, }; list_state.select(Some(i)); } }
                                KeyCode::Up => { if !commands.is_empty() { let i = match list_state.selected() { Some(i) => if i == 0 { commands.len() - 1 } else { i - 1 }, None => 0, }; list_state.select(Some(i)); } }
                                KeyCode::Enter => {
                                    if let Some(i) = list_state.selected() {
                                        if i < commands.len() {
                                            let cmd_struct = commands[i].clone();
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

        // --- АВТОЗБЕРЕЖЕННЯ (30 СЕКУНД ТИШІ) ---
        if last_user_activity.elapsed() >= Duration::from_secs(30) {

            // 1. Зберігаємо текстові файли (Notes, Todo, Logs)
            for (i, modified) in files_modified.iter_mut().enumerate() {
                if *modified {
                    let text_to_save = textareas[i].lines().join("\n");
                    // Використовуємо .ok(), щоб не крашити програму при помилці запису
                    fs::write(file_names[i], text_to_save).ok();
                    *modified = false; // Скидаємо прапорець
                }
            }

            if tasks_modified {
                let _ = fs::write("tasks.json", serde_json::to_string_pretty(&tasks).unwrap_or_default());
                tasks_modified = false;
            }
        }
    }

    for (i, filename) in file_names.iter().enumerate() {
        let text_to_save = textareas[i].lines().join("\n");
        fs::write(filename, text_to_save)?;
    }

    if tasks_modified {
        let _ = fs::write("tasks.json", serde_json::to_string_pretty(&tasks).unwrap_or_default());
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste)?;
    terminal.show_cursor()?;

    Ok(())
}