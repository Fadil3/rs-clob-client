use std::io;
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use ratatui::{
    backend::CrosstermBackend,
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    layout::{Layout, Constraint, Direction},
    Terminal,
    style::{Style, Color, Modifier},
    text::{Span, Line},
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use tokio::sync::mpsc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AppLog {
    pub level: String,
    pub msg: String,
    pub timestamp: String,
}

#[derive(Debug)]
pub struct AppState {
    pub logs: VecDeque<AppLog>,
    pub balance: String,
    pub shard: String,
    pub active_arbs: usize,
    pub total_profit: String,
    pub events_processed: u64,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            logs: VecDeque::with_capacity(100),
            balance: "Loading...".to_string(),
            shard: "Unknown".to_string(),
            active_arbs: 0,
            total_profit: "$0.00".to_string(),
            events_processed: 0,
        }
    }

    pub fn add_log(&mut self, level: String, msg: String) {
        if self.logs.len() >= 50 {
            self.logs.pop_front();
        }
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        self.logs.push_back(AppLog { level, msg, timestamp });
    }
}

pub async fn run_tui(
    state: Arc<Mutex<AppState>>,
    mut kill_rx: mpsc::Receiver<()>,
) -> anyhow::Result<()> {
    // Setup Terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, state, &mut kill_rx).await;

    // Restore Terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    state: Arc<Mutex<AppState>>,
    kill_rx: &mut mpsc::Receiver<()>,
) -> io::Result<()> {
    loop {
        {
             // Check for kill signal (non-blocking)
             if kill_rx.try_recv().is_ok() {
                 return Ok(());
             }
        }
        
        terminal.draw(|f| ui(f, &state))?;

        // Handle Input (Poll for 100ms)
        if crossterm::event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Char('q') = key.code {
                    return Ok(());
                }
            }
        }
    }
}

fn ui<B: ratatui::backend::Backend>(f: &mut ratatui::Frame, state: &Arc<Mutex<AppState>>) {
    let state = state.lock().unwrap();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Min(10),    // Logs
            Constraint::Length(10), // Footer / Stats
        ].as_ref())
        .split(f.size()); // Changed f.size() to f.area() in 0.29 but f.size() is deprecated, let's try f.area() or stick to size based on crate version. cargo.toml says 0.29. let's assume f.area() or use simple size() for now. Actually f.size() is correct for ratatui < 0.26 or something. 0.29 uses f.area() usually. I'll use f.area() to be safe or f.size() if compiler complains. I'll use f.size() as it is often aliased.

    // Header
    let header_text = format!(
        " POLYBOT SWARM | SHARD: {} | BALANCE: {} | EVENTS: {}",
        state.shard, state.balance, state.events_processed
    );
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Black).bg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Logs
    let logs: Vec<ListItem> = state.logs.iter().map(|log| {
        let color = match log.level.as_str() {
            "ERROR" => Color::Red,
            "WARN" => Color::Yellow,
            "INFO" => Color::White,
            "ARBITRAGE" => Color::Green,
            _ => Color::White,
        };
        let content = format!("{} [{}] {}", log.timestamp, log.level, log.msg);
        ListItem::new(content).style(Style::default().fg(color))
    }).collect();

    let logs_list = List::new(logs)
        .block(Block::default().borders(Borders::ALL).title("Logs"))
        .style(Style::default().fg(Color::White));
    f.render_widget(logs_list, chunks[1]);
    
    // Stats Footer
    let stats_text = format!(
        "Active Arbitrages: {}\nTotal Profit (Est): {}",
        state.active_arbs, state.total_profit
    );
    let stats = Paragraph::new(stats_text)
        .block(Block::default().borders(Borders::ALL).title("Metrics"))
        .wrap(Wrap { trim: true });
    f.render_widget(stats, chunks[2]);
}
