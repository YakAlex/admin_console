use std::collections::VecDeque;
use ratatui::prelude::*;

pub fn generate_sparkline(history: &VecDeque<u128>) -> String {
    history.iter().map(|&val| {
        if val == 0 { " " }
        else if val < 20 { " " }
        else if val < 50 { "▂" }
        else if val < 100 { "▃" }
        else if val < 200 { "▅" }
        else { "▇" }
    }).collect()
}

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