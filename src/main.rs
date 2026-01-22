use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Gauge},
};
use std::{fs, io, net::TcpStream, sync::mpsc, thread, time::{Duration, Instant}};
use tui_textarea::TextArea;
use sysinfo::{System, Networks}; // Спростили імпорти

struct Target {
    name: &'static str,
    address: &'static str,
}

fn main() -> Result<()> {
    let path = "notes.txt";
    let content = fs::read_to_string(path).unwrap_or_default();

    let mut textarea = TextArea::new(content.lines().map(|s| s.to_string()).collect());
    textarea.set_block(
        Block::default().borders(Borders::ALL).title(" 📝 Нотатки "),
    );

    let targets = vec![
        Target { name: "Cloudflare", address: "1.1.1.1:80" },
        Target { name: "Google DNS", address: "8.8.8.8:53" },
        Target { name: "Local Router", address: "192.168.0.1:80" },
    ];

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        loop {
            let mut report = String::new();
            report.push_str("   TARGET       | STATUS | PING \n");
            report.push_str("----------------+--------+------\n");

            for target in &targets {
                let start = Instant::now();
                match TcpStream::connect_timeout(&target.address.parse().unwrap(), Duration::from_millis(1000)) {
                    Ok(_) => {
                        let duration = start.elapsed().as_millis();
                        report.push_str(&format!("{:<15} |  🟢 ON | {}ms\n", target.name, duration));
                    },
                    Err(_) => {
                        report.push_str(&format!("{:<15} |  🔴 OFF| ---\n", target.name));
                    }
                }
            }
            tx.send(report).unwrap();
            thread::sleep(Duration::from_secs(2));
        }
    });

    // ВИПРАВЛЕННЯ 1: Простіша ініціалізація sysinfo
    let mut sys = System::new_all();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut monitor_output = "Scanning network...".to_string();

    loop {
        // Оновлюємо дані
        sys.refresh_all();

        let global_cpu_usage = sys.global_cpu_info().cpu_usage();
        let used_mem = sys.used_memory();
        let total_mem = sys.total_memory();
        // Захист від ділення на нуль (про всяк випадок)
        let mem_percentage = if total_mem > 0 {
            (used_mem as f64 / total_mem as f64) * 100.0
        } else {
            0.0
        };

        if let Ok(new_report) = rx.try_recv() {
            monitor_output = new_report;
        }

        terminal.draw(|f| {
            let main_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(f.size());

            let left_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(main_chunks[0]);

            // Network
            let net_block = Block::default().title(" 📡 Network ").borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan));
            let net_text = Paragraph::new(monitor_output.clone()).block(net_block);
            f.render_widget(net_text, left_chunks[0]);

            // System
            let sys_block = Block::default().title(" 💻 System Resources ").borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow));
            f.render_widget(sys_block, left_chunks[1]);

            // ВИПРАВЛЕННЯ 2: Прибрали '&' перед Margin
            let sys_area = left_chunks[1].inner(Margin { vertical: 1, horizontal: 1 });

            let gauge_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Length(2)])
                .split(sys_area);

            let cpu_label = format!("CPU: {:.1}%", global_cpu_usage);
            let cpu_gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Green))
                .ratio((global_cpu_usage as f64 / 100.0).clamp(0.0, 1.0)) // clamp щоб не вилізло за 100%
                .label(cpu_label);
            f.render_widget(cpu_gauge, gauge_chunks[0]);

            let mem_label = format!("RAM: {:.1}% ({}/{} GB)", mem_percentage, used_mem/1024/1024/1024, total_mem/1024/1024/1024);
            let mem_gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Magenta))
                .ratio((mem_percentage / 100.0).clamp(0.0, 1.0))
                .label(mem_label);
            f.render_widget(mem_gauge, gauge_chunks[2]);

            f.render_widget(textarea.widget(), main_chunks[1]);
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Esc => break,
                        _ => { textarea.input(key); }
                    }
                }
            }
        }
    }

    let text_to_save = textarea.lines().join("\n");
    fs::write(path, text_to_save)?;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}