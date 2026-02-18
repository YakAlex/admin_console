use std::fs;
use std::sync::mpsc::Sender;
use std::time::Instant;
use tui_textarea::TextArea;
use ratatui::widgets::{Block, Borders, ListState, TableState};
use ratatui::style::{Color, Style};
use arboard::Clipboard;

use crate::config::AppConfig;
use crate::types::{ServerStatus, AppEvent, EditorMode, ActiveView, MonitorCommand, Task};
use crate::utils::parse_tasks_from_text;

pub struct App<'a> {
    pub textareas: Vec<TextArea<'a>>,
    pub file_names: Vec<&'static str>,
    pub titles: Vec<&'static str>,

    pub config: AppConfig,
    pub tasks: Vec<Task>,
    pub server_data: Vec<ServerStatus>,

    pub active_view: ActiveView,
    pub list_state: ListState,
    pub table_state: TableState,

    pub tx_to_monitor: Sender<MonitorCommand>,
    pub tx_main: Sender<AppEvent>, // <--- Додали канал для UI подій

    pub clipboard: Option<Clipboard>,
    pub last_user_activity: Instant,
    pub files_modified: Vec<bool>,
    pub tasks_modified: bool,

    pub should_quit: bool,
    pub should_redraw: bool,
    pub is_selecting: bool,
}

impl<'a> App<'a> {
    pub fn new(tx_to_monitor: Sender<MonitorCommand>, tx_main: Sender<AppEvent>) -> Self {
        let file_names = vec!["notes.txt", "todo.txt", "logs.txt"];
        let titles = vec![" 1.Notes ", " 2.Todo ", " 3.Logs "];

        // 1. Ініціалізація текстових полів
        let mut textareas = Vec::new();
        for filename in &file_names {
            let content = fs::read_to_string(filename).unwrap_or_default();
            if fs::metadata(filename).is_err() {
                fs::write(filename, &content).ok();
            }
            let mut ta = TextArea::new(content.lines().map(|s| s.to_string()).collect());
            ta.set_max_histories(10000);
            ta.set_block(Block::default().borders(Borders::ALL));
            ta.set_search_style(Style::default().bg(Color::Yellow).fg(Color::Black));
            textareas.push(ta);
        }

        // 2. Завантаження конфігу
        let config_path = "config.json";
        let config_data = fs::read_to_string(config_path).unwrap_or_else(|_| { r#"{ "targets": [], "commands": [] }"#.to_string() });
        let config: AppConfig = serde_json::from_str(&config_data).unwrap_or_else(|_| { AppConfig { targets: vec![], commands: vec![] } });

        // 3. Завантаження завдань (Todo)
        let todo_exists = std::path::Path::new("todo.txt").exists();
        let todo_content = fs::read_to_string("todo.txt").unwrap_or_default();
        let mut tasks = parse_tasks_from_text(&todo_content);

        if !todo_exists && tasks.is_empty() {
            let tasks_path = "tasks.json";
            let tasks_data = fs::read_to_string(tasks_path).unwrap_or_else(|_| "[]".to_string());
            tasks = serde_json::from_str(&tasks_data).unwrap_or(Vec::new());
        }
        let _ = fs::write("tasks.json", serde_json::to_string_pretty(&tasks).unwrap_or_default());

        // 4. Стан UI
        let mut list_state = ListState::default();
        if !config.commands.is_empty() { list_state.select(Some(0)); }

        let mut table_state = TableState::default();
        if !config.targets.is_empty() { table_state.select(Some(0)); }

        Self {
            textareas,
            file_names,
            titles,
            config,
            tasks,
            server_data: Vec::new(),
            active_view: ActiveView::Editor(EditorMode::Notes),
            list_state,
            table_state,
            tx_to_monitor,
            tx_main,
            clipboard: Clipboard::new().ok(),
            last_user_activity: Instant::now(),
            files_modified: vec![false, false, false],
            tasks_modified: false,
            should_quit: false,
            should_redraw: true,
            is_selecting: false,
        }
    }

    pub fn sync_todo_from_text(&mut self) {
        if self.files_modified[1] {
            let content = self.textareas[1].lines().join("\n");
            self.tasks = parse_tasks_from_text(&content);
            self.tasks_modified = true;
            let _ = self.tx_to_monitor.send(MonitorCommand::UpdateTasks(self.tasks.clone()));
        }
    }
}