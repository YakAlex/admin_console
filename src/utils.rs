use ratatui::prelude::*;
use crate::types::Task;

// --- UI Helper ---
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

// --- LOGIC: Validation ---

/// Перевіряє, чи є рядок коректним часом у форматі HH:MM.
/// Вимоги: 5 символів, наявність ':', години 0-23, хвилини 0-59.
pub fn is_valid_time(s: &str) -> bool {
    if s.len() != 5 { return false; }

    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 { return false; }

    match (parts[0].parse::<u8>(), parts[1].parse::<u8>()) {
        (Ok(h), Ok(m)) => h < 24 && m < 60,
        _ => false,
    }
}

// --- LOGIC: Parsing ---

/// Парсить текст файлу todo.txt у структуру Task.
/// Реалізує стійкість до помилок: невалідний час не зникає, а стає частиною тексту.
pub fn parse_tasks_from_text(content: &str) -> Vec<Task> {
    let mut tasks = Vec::new();
    // Використовуємо Option, щоб коректно обробляти багаторядкові описи
    let mut current_task: Option<Task> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        // Шукаємо початок нового завдання "- ["
        if let Some(start_bracket) = trimmed.find("- [") {
            // 1. Якщо ми вже парсили завдання, зберігаємо його в список
            if let Some(t) = current_task.take() {
                tasks.push(t);
            }

            let rest = &trimmed[start_bracket + 3..]; // Обрізаємо "- ["

            // Шукаємо закриваючу дужку ']'
            if let Some(end_bracket) = rest.find(']') {
                let content_inside = rest[..end_bracket].trim(); // Те, що в дужках
                let description_part = rest[end_bracket + 1..].trim(); // Те, що після дужок

                let mut time = String::new();
                let mut completed = false;
                let mut title = description_part.to_string();

                // ЛОГІКА ОБРОБКИ СТАТУСУ ТА ЧАСУ
                if content_inside.eq_ignore_ascii_case("x") {
                    completed = true;
                } else if is_valid_time(content_inside) {
                    // Тільки якщо це ДІЙСНО час (наприклад, 14:00), ми його плануємо
                    time = content_inside.to_string();
                } else if !content_inside.is_empty() {
                    // Якщо там написано "25:00" або "abc", це не час.
                    // Ми повертаємо це в назву, щоб користувач бачив свою помилку.
                    title = format!("[{}] {}", content_inside, title);
                }

                current_task = Some(Task {
                    title,
                    description: String::new(),
                    time,
                    completed,
                });
            }
        }
        // Якщо рядок не починається з "- [", це продовження опису попереднього завдання
        else if let Some(ref mut t) = current_task {
            if !t.description.is_empty() {
                t.description.push('\n');
            }
            t.description.push_str(trimmed);
        }
    }

    // Не забуваємо додати останнє завдання
    if let Some(t) = current_task {
        tasks.push(t);
    }

    tasks
}