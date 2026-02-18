use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::CursorMove;
use std::process::Command;
use std::thread;
use encoding_rs::IBM866;

use crate::app::App;
use crate::types::{ActiveView, EditorMode, WizardStep, AppEvent};
use crate::utils::is_valid_time;

pub fn handle_input(key: KeyEvent, app: &mut App) {
    let mut change_view = None;

    if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('q') || key.code == KeyCode::Char('й')) {
        app.should_quit = true;
        return;
    }
    if key.modifiers == KeyModifiers::ALT && (key.code == KeyCode::Char('t') || key.code == KeyCode::Char('е')) {
        change_view = Some(ActiveView::TodoWizard { step: WizardStep::Title, buffer: String::new(), temp_title: String::new(), temp_desc: String::new() });
    }

    match &mut app.active_view {
        ActiveView::Search { mode_return_to, query } => {
            let idx = *mode_return_to as usize;
            match key.code {
                KeyCode::Esc => { app.textareas[idx].set_search_pattern("").ok(); change_view = Some(ActiveView::Editor(*mode_return_to)); }
                KeyCode::Enter => { app.textareas[idx].search_forward(false); }
                KeyCode::Backspace => { query.pop(); app.textareas[idx].set_search_pattern(query.as_str()).ok(); }
                KeyCode::Char(c) => { query.push(c); app.textareas[idx].set_search_pattern(query.as_str()).ok(); app.textareas[idx].search_forward(false); }
                _ => {}
            }
        }
        ActiveView::InputPopup { command_idx, input_buffer } => {
            match key.code {
                KeyCode::Enter => {
                    let idx_copy = *command_idx;
                    if idx_copy < app.config.commands.len() {
                        let cmd_struct = app.config.commands[idx_copy].clone();
                        let buffer_clone = input_buffer.clone();
                        let final_args: Vec<String> = cmd_struct.args.iter().map(|arg| if arg == "%INPUT%" { buffer_clone.clone() } else { arg.clone() }).collect();
                        change_view = Some(ActiveView::Editor(EditorMode::Logs));

                        // Клонуємо канал для потоку
                        let tx_cmd = app.tx_main.clone();
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
                            if is_valid_time(&buffer) || buffer.is_empty() {
                                let time_str = buffer.trim().to_string();
                                let display_str = if time_str.is_empty() { format!("- [ ] {}\n      {}", temp_title, temp_desc) } else { format!("- [{}] {}\n      {}", time_str, temp_title, temp_desc) };
                                let todo_area = &mut app.textareas[1];
                                todo_area.move_cursor(CursorMove::Bottom);
                                if !todo_area.lines().is_empty() { todo_area.insert_str("\n"); }
                                todo_area.insert_str(display_str);
                                app.files_modified[1] = true;
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
            let textarea = &mut app.textareas[idx];

            if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('f') || key.code == KeyCode::Char('а')) { change_view = Some(ActiveView::Search { mode_return_to: *mode, query: String::new() }); }
            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('с')) { textarea.copy(); if let Some(cb) = &mut app.clipboard { let _ = cb.set_text(textarea.yank_text()); } }
            else if (key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) || (key.modifiers == KeyModifiers::ALT && (key.code == KeyCode::Char('v') || key.code == KeyCode::Char('м'))) { if let Some(cb) = &mut app.clipboard { if let Ok(text) = cb.get_text() { textarea.insert_str(text); app.files_modified[idx] = true; } } }
            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('x') || key.code == KeyCode::Char('ч')) { textarea.cut(); if let Some(cb) = &mut app.clipboard { let _ = cb.set_text(textarea.yank_text()); } app.files_modified[idx] = true; }
            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('z') || key.code == KeyCode::Char('я')) { textarea.undo(); app.files_modified[idx] = true; }
            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('y') || key.code == KeyCode::Char('н')) { textarea.redo(); app.files_modified[idx] = true; }
            else if key.modifiers == KeyModifiers::CONTROL && (key.code == KeyCode::Char('a') || key.code == KeyCode::Char('ф')) { textarea.move_cursor(CursorMove::Top); textarea.move_cursor(CursorMove::Head); textarea.start_selection(); textarea.move_cursor(CursorMove::Bottom); textarea.move_cursor(CursorMove::End); app.is_selecting = true; }
            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Left { if !app.is_selecting { textarea.start_selection(); app.is_selecting = true; } textarea.move_cursor(CursorMove::WordBack); }
            else if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Right { if !app.is_selecting { textarea.start_selection(); app.is_selecting = true; } textarea.move_cursor(CursorMove::WordForward); }
            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Left { textarea.cancel_selection(); app.is_selecting = false; textarea.move_cursor(CursorMove::WordBack); }
            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Right { textarea.cancel_selection(); app.is_selecting = false; textarea.move_cursor(CursorMove::WordForward); }
            else if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Backspace { textarea.delete_word(); app.is_selecting = false; app.files_modified[idx] = true; }
            else {
                match key.code {
                    KeyCode::Esc => app.should_quit = true,
                    KeyCode::Tab => { change_view = Some(ActiveView::Actions); }
                    KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                    KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Todo)); }
                    KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => { change_view = Some(ActiveView::Editor(EditorMode::Logs)); }
                    KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete => { if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT { app.is_selecting = false; } textarea.input(key); app.files_modified[idx] = true; },
                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => { if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT { app.is_selecting = false; } textarea.input(key); }
                    _ => {}
                }
            }
        }
        ActiveView::Actions => {
            match key.code {
                KeyCode::Esc => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                KeyCode::Tab => { change_view = Some(ActiveView::Editor(EditorMode::Notes)); }
                KeyCode::Down => { if !app.config.commands.is_empty() { let i = match app.list_state.selected() { Some(i) => if i >= app.config.commands.len() - 1 { 0 } else { i + 1 }, None => 0, }; app.list_state.select(Some(i)); } }
                KeyCode::Up => { if !app.config.commands.is_empty() { let i = match app.list_state.selected() { Some(i) => if i == 0 { app.config.commands.len() - 1 } else { i - 1 }, None => 0, }; app.list_state.select(Some(i)); } }
                KeyCode::Enter => {
                    if let Some(i) = app.list_state.selected() {
                        if i < app.config.commands.len() {
                            let cmd_struct = app.config.commands[i].clone();
                            if cmd_struct.args.contains(&"%INPUT%".to_string()) {
                                change_view = Some(ActiveView::InputPopup { command_idx: i, input_buffer: String::new() });
                            } else {
                                change_view = Some(ActiveView::Editor(EditorMode::Logs));
                                let tx_cmd = app.tx_main.clone();
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

    if let Some(new_view) = change_view { app.active_view = new_view; }
}