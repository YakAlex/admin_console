use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Table, Row, Cell, Tabs, TableState, Clear},
    style::{Color, Modifier, Style},
};
use tui_textarea::TextArea;
use crate::types::{ServerStatus, ActiveView, EditorMode, Task, WizardStep};
use crate::config::AdminCommand;
use crate::utils::centered_rect;

pub fn draw(
    f: &mut Frame,
    textareas: &Vec<TextArea>,
    server_data: &Vec<ServerStatus>,
    tasks: &Vec<Task>,
    active_view: &ActiveView,
    table_state: &mut TableState,
    list_state: &mut ListState,
    commands: &Vec<AdminCommand>,
    titles: &Vec<&str>,
) {
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(f.area());

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(8)])
        .split(main_chunks[0]);

    // --- TABLE (SERVERS) ---
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
    f.render_stateful_widget(table, left_chunks[0], table_state);

    // --- SCHEDULE (LEFT BOTTOM) ---
    let mut active_tasks: Vec<&Task> = tasks.iter().filter(|t| !t.completed).collect();
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

    for (i, task) in active_tasks.iter().take(5).enumerate() {
        let has_time = !task.time.is_empty();
        if i > 0 && !has_time && !first_untimed_seen {
            items.push(ListItem::new(" ──────────────────────").style(Style::default().fg(Color::DarkGray)));
            first_untimed_seen = true;
        }
        let (prefix, style) = if has_time {
            (format!(" ⏰ {} │ ", task.time), Style::default().fg(Color::Yellow))
        } else {
            (" 📝  --   │ ".to_string(), Style::default().fg(Color::Cyan))
        };
        let title = if task.title.len() > 18 { format!("{}..", &task.title[..18]) } else { task.title.clone() };
        items.push(ListItem::new(format!("{}{}", prefix, title)).style(style));
        if !has_time { first_untimed_seen = true; }
    }

    let list_widget = if items.is_empty() {
        List::new(vec![ListItem::new("   (No active tasks)").style(Style::default().fg(Color::DarkGray))])
    } else {
        List::new(items)
    };
    f.render_widget(list_widget.block(Block::default().borders(Borders::ALL).title(" 📅 Schedule ")), left_chunks[1]);

    // --- RIGHT SIDE (TABS & CONTENT) ---
    let right_chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(3), Constraint::Min(0)]).split(main_chunks[1]);

    let (current_file_idx, is_actions_active) = match active_view {
        ActiveView::Editor(mode) => (*mode as usize, false),
        ActiveView::Search { mode_return_to, .. } => (*mode_return_to as usize, false),
        ActiveView::Actions => (0, true),
        ActiveView::InputPopup { .. } => (0, true),
        ActiveView::TodoWizard { .. } => (1, true),
    };

    let file_tabs = Tabs::new(titles.clone())
        .block(Block::default().borders(Borders::BOTTOM))
        .select(if !is_actions_active { current_file_idx } else { 99 })
        .highlight_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
    f.render_widget(file_tabs, right_chunks[0]);

    let action_status = if is_actions_active { Paragraph::new(" [TAB] ACTIONS ").style(Style::default().fg(Color::Black).bg(Color::Yellow)) } else { Paragraph::new(" [TAB] Actions | [ALT+T] New Task") };
    f.render_widget(action_status, Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(70), Constraint::Percentage(30)]).split(right_chunks[0])[1]);

    // --- CONTENT SWITCHER ---
    match active_view {
        ActiveView::Editor(mode) | ActiveView::Search { mode_return_to: mode, .. } => {
            f.render_widget(&textareas[*mode as usize], right_chunks[1]);
        }
        ActiveView::Actions | ActiveView::InputPopup { .. } => {
            let items: Vec<ListItem> = commands.iter().map(|i| ListItem::new(i.name.clone()).style(Style::default().fg(Color::White))).collect();
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" Оберіть команду "))
                .highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD))
                .highlight_symbol(">> ");
            f.render_stateful_widget(list, right_chunks[1], list_state);
        }
        ActiveView::TodoWizard { .. } => {
            f.render_widget(&textareas[1], right_chunks[1]);
        }
    }

    // --- POPUPS ---
    if let ActiveView::Search { query, .. } = active_view {
        let search_area = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(0), Constraint::Length(3)]).split(right_chunks[1])[1];
        f.render_widget(Clear, search_area);
        f.render_widget(Paragraph::new(format!("Search: {}", query)).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan))).style(Style::default().fg(Color::Yellow).bg(Color::Black)), search_area);
    }
    if let ActiveView::InputPopup { input_buffer, .. } = active_view {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, area);
        f.render_widget(Paragraph::new(input_buffer.clone()).block(Block::default().borders(Borders::ALL).title(" Введіть аргумент (IP/Host) ")).style(Style::default().fg(Color::Yellow).bg(Color::Black)), area);
    }
    if let ActiveView::TodoWizard { step, buffer, temp_title, .. } = active_view {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, area);
        let (title, content) = match step {
            WizardStep::Title => (" 1/3: Назва завдання ", format!("Введіть назву:\n\n> {}", buffer)),
            WizardStep::Description => (" 2/3: Опис ", format!("Назва: {}\n\nВведіть опис (можна пустий):\n> {}", temp_title, buffer)),
            WizardStep::Time => (" 3/3: Час нагадування ", format!("Назва: {}\n\nВведіть час (HH:MM) або Enter щоб пропустити:\n> {}", temp_title, buffer)),
        };
        let block = Paragraph::new(content).block(Block::default().borders(Borders::ALL).title(title)).style(Style::default().fg(Color::Cyan).bg(Color::Black));
        f.render_widget(block, area);
    }
}