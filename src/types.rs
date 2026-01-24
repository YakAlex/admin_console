use std::collections::VecDeque;

#[derive(Clone)]
pub struct ServerStatus {
    pub name: String,
    pub is_online: bool,
    pub latency: u128,
    pub history: VecDeque<u128>,
}

pub enum AppEvent {
    ServerUpdate(Vec<ServerStatus>),
    LogOutput(String),
}

#[derive(PartialEq, Copy, Clone)]
pub enum EditorMode {
    Notes = 0,
    Todo = 1,
    Logs = 2,
}

#[derive(PartialEq)]
pub enum ActiveView {
    Editor(EditorMode),
    Actions,
    InputPopup {
        command_idx: usize,
        input_buffer: String
    },
    Search {
        mode_return_to: EditorMode,
        query: String,
    }
}