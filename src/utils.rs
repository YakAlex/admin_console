use ratatui::prelude::*;
use crate::types::Task;

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

// Перевірка, чи ввів користувач правильний час (HH:MM)
pub fn is_valid_time(input: &str) -> bool {
    let s = input.trim();
    if s.is_empty() { return true; } // Пустий час дозволяємо

    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 { return false; }

    let h = parts[0].parse::<u8>();
    let m = parts[1].parse::<u8>();

    match (h, m) {
        (Ok(hours), Ok(minutes)) => hours < 24 && minutes < 60,
        _ => false,
    }
}

// Головна функція синхронізації: Текст -> Список завдань
pub fn parse_tasks_from_text(content: &str) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut current_task: Option<Task> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        // Шукаємо початок завдання "- ["
        if let Some(start_bracket) = trimmed.find("- [") {
            // Якщо ми парсили попереднє завдання, зберігаємо його
            if let Some(t) = current_task.take() { tasks.push(t); }

            let rest = &trimmed[start_bracket + 3..]; // Пропускаємо "- ["

            // Шукаємо закриваючу дужку
            if let Some(end_bracket) = rest.find(']') {
                let content_inside = &rest[..end_bracket]; // Це "14:00", "x", " " тощо
                let title_part = &rest[end_bracket + 1..].trim();

                // 1. Визначаємо статус виконання
                // Якщо всередині є 'x' або 'X' — завдання виконане
                let completed = content_inside.to_lowercase().contains('x');

                // 2. Визначаємо час
                // Якщо це не 'x' і не пусто, пробуємо парсити як час
                let time = if !completed && !content_inside.trim().is_empty() {
                    content_inside.trim().to_string()
                } else {
                    String::new()
                };

                current_task = Some(Task {
                    title: title_part.to_string(),
                    description: String::new(), // Опис поки пустий, заповнимо далі якщо є
                    time,
                    completed,
                });
            }
        }
        // Якщо рядок не починається з "- [", це продовження опису попереднього завдання
        else if let Some(ref mut t) = current_task {
            if !t.description.is_empty() { t.description.push('\n'); }
            t.description.push_str(trimmed);
        }
    }

    if let Some(t) = current_task { tasks.push(t); }
    tasks
}