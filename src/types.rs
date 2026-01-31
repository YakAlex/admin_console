use std::collections::VecDeque;
use serde::{Deserialize, Serialize}; // Додали для збереження в JSON
use crate::config::Target;
#[derive(Clone)]
pub struct ServerStatus {
    pub name: String,
    pub is_online: bool,
    pub latency: u128,
    pub history: VecDeque<u128>,
}

// --- НОВА СТРУКТУРА ЗАВДАННЯ ---
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Task {
    pub title: String,
    pub description: String,
    pub time: String, // Формат "HH:MM" або пустий ""
    pub completed: bool,
}

pub enum AppEvent {
    ServerUpdate(Vec<ServerStatus>),
    LogOutput(String),
}

// Команди для фонового потоку
pub enum MonitorCommand {
    UpdateTargets(Vec<Target>),
    UpdateTasks(Vec<Task>), // Оновити список завдань у потоці
}

#[derive(PartialEq, Copy, Clone)]
pub enum EditorMode {
    Notes = 0,
    Todo = 1,
    Logs = 2,
}

// Етапи нашого меню створення (Wizard)
#[derive(PartialEq, Clone)]
pub enum WizardStep {
    Title,
    Description,
    Time,
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
    },
    // --- НОВИЙ РЕЖИМ: СТВОРЕННЯ ЗАВДАННЯ ---
    TodoWizard {
        step: WizardStep,
        buffer: String,      // Те, що ми зараз пишемо
        temp_title: String,  // Вже введена назва
        temp_desc: String,   // Вже введений опис
    }
}