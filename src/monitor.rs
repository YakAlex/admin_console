use std::{
    collections::VecDeque,
    net::TcpStream,
    sync::mpsc::{Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
use chrono::Local;
use notify_rust::Notification;
use rayon::prelude::*; // <--- ВАЖЛИВИЙ ІМПОРТ

use crate::config::Target;
use crate::types::{AppEvent, MonitorCommand, ServerStatus, Task};

pub fn start_monitor(
    targets: Vec<Target>,
    tasks: Vec<Task>,
    tx_monitor: Sender<AppEvent>,
    rx_from_main: Receiver<MonitorCommand>,
) {
    thread::spawn(move || {
        let mut statuses: Vec<ServerStatus> = targets.iter().map(|t| ServerStatus {
            name: t.name.clone(),
            is_online: false,
            latency: 0,
            history: VecDeque::from(vec![0; 20]),
        }).collect();

        let mut thread_tasks = tasks;
        let mut last_checked_minute = String::new();
        let mut current_targets = targets.clone();
        let mut previous_online_status: Vec<bool> = vec![true; current_targets.len()];

        loop {
            // 1. Оновлення конфігурації
            while let Ok(cmd) = rx_from_main.try_recv() {
                match cmd {
                    MonitorCommand::UpdateTargets(new_targets) => { current_targets = new_targets; }
                    MonitorCommand::UpdateTasks(new_tasks) => { thread_tasks = new_tasks; }
                }
            }

            // 2. ПАРАЛЕЛЬНИЙ ПІНГ (ОПТИМІЗАЦІЯ) 🚀
            // Rayon запускає це одночасно на всіх ядрах
            let check_results: Vec<(bool, u128)> = current_targets
                .par_iter() // Було .iter(), стало .par_iter()
                .map(|target| {
                    let start = Instant::now();
                    // Ця операція тепер не блокує інших
                    match TcpStream::connect_timeout(
                        &target.address.parse().unwrap_or("0.0.0.0:0".parse().unwrap()),
                        Duration::from_millis(500)
                    ) {
                        Ok(_) => (true, start.elapsed().as_millis()),
                        Err(_) => (false, 0),
                    }
                })
                .collect();

            // 3. Оновлення стану (це дуже швидко, мілісекунди)
            for (i, (online, lat)) in check_results.into_iter().enumerate() {
                let status = &mut statuses[i];
                status.is_online = online;
                status.latency = lat;

                let history_val = if status.is_online { status.latency } else { 999 };
                status.history.pop_front();
                status.history.push_back(history_val);
                // 1. СЕРВЕР ВПАВ (Online -> Offline)
                if previous_online_status[i] && !online {
                    let timestamp = Local::now().format("%H:%M:%S");
                    let log_msg = format!("[{}] 🔴 ALERT: Server '{}' went OFFLINE!", timestamp, current_targets[i].name);
                    let _ = tx_monitor.send(AppEvent::LogOutput(log_msg));

                    Notification::new()
                        .summary("SERVER DOWN ⚠️")
                        .body(&format!("Увага! Сервер '{}' перестав відповідати.", current_targets[i].name))
                        .appname("Admin Console")
                        .show()
                        .ok();
                }
                // 2. СЕРВЕР ПІДНЯВСЯ (Offline -> Online)
                else if !previous_online_status[i] && online {
                    let timestamp = Local::now().format("%H:%M:%S");
                    let log_msg = format!("[{}] 🟢 INFO: Server '{}' is back ONLINE.", timestamp, current_targets[i].name);
                    let _ = tx_monitor.send(AppEvent::LogOutput(log_msg));
                }

                previous_online_status[i] = online;
            }

            let _ = tx_monitor.send(AppEvent::ServerUpdate(statuses.clone()));

            // 4. Перевірка нагадувань (Tasks) - без змін
            let current_time_str = Local::now().format("%H:%M").to_string();
            if current_time_str != last_checked_minute {
                for task in &thread_tasks {
                    if !task.completed && !task.time.is_empty() && task.time == current_time_str {
                        Notification::new()
                            .summary(&format!("🔔 Reminder: {}", task.title))
                            .body(&task.description)
                            .appname("Admin Console")
                            .show()
                            .ok();
                        let _ = tx_monitor.send(AppEvent::TaskCompleted(task.title.clone()));
                    }
                }
                last_checked_minute = current_time_str;
            }

            thread::sleep(Duration::from_secs(1));
        }
    });
}