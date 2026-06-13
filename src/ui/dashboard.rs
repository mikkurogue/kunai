use std::{
    collections::VecDeque,
    fs,
    io::Read,
    path::{
        Path,
        PathBuf,
    },
    time::{
        Duration,
        SystemTime,
    },
};

use anyhow::Result;
use ratatui::{
    DefaultTerminal,
    Frame,
    crossterm::event::{
        self,
        Event,
        KeyCode,
        KeyEventKind,
    },
    layout::{
        Constraint,
        Layout,
    },
    style::{
        Color,
        Modifier,
        Style,
        Stylize,
    },
    text::Text,
    widgets::{
        Block,
        Paragraph,
    },
};

const MAX_LINES: usize = 10_000;

struct DashboardState {
    lines:         VecDeque<String>,
    log_path:      PathBuf,
    pid_path:      PathBuf,
    last_mtime:    Option<SystemTime>,
    log_size:      Option<u64>,
    scroll_offset: usize,
    follow:        bool,
    pid:           Option<i32>,
    running:       bool,
}

pub fn run() -> Result<()> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("kunai");

    let log_path = config_dir.join("daemon.log");
    let pid_path = config_dir.join("daemon.pid");

    let mut terminal =
        ratatui::try_init().map_err(|e| anyhow::anyhow!("Dashboard requires a terminal: {}", e))?;
    let result = run_dashboard(&mut terminal, &log_path, &pid_path);
    let _ = ratatui::try_restore();
    result
}

fn run_dashboard(terminal: &mut DefaultTerminal, log_path: &Path, pid_path: &Path) -> Result<()> {
    let mut state = DashboardState {
        lines:         VecDeque::new(),
        log_path:      log_path.to_owned(),
        pid_path:      pid_path.to_owned(),
        last_mtime:    None,
        log_size:      None,
        scroll_offset: 0,
        follow:        true,
        pid:           None,
        running:       false,
    };

    load_log(&mut state)?;
    read_pid(&mut state);

    loop {
        terminal.draw(|frame| draw_dashboard(frame, &state))?;

        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => break,
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        state.follow = !state.follow;
                        if state.follow {
                            state.scroll_offset = 0;
                        }
                    }
                    KeyCode::Up => {
                        state.follow = false;
                        state.scroll_offset = state.scroll_offset.saturating_add(1);
                    }
                    KeyCode::Down => {
                        state.scroll_offset = state.scroll_offset.saturating_sub(1);
                    }
                    KeyCode::PageUp => {
                        state.follow = false;
                        state.scroll_offset = state
                            .scroll_offset
                            .saturating_add(area_height(terminal) as usize);
                    }
                    KeyCode::PageDown => {
                        state.scroll_offset = state
                            .scroll_offset
                            .saturating_sub(area_height(terminal) as usize);
                    }
                    _ => {}
                },
                _ => {}
            }
        } else {
            load_log(&mut state)?;
            read_pid(&mut state);
        }
    }

    Ok(())
}

fn area_height(terminal: &DefaultTerminal) -> u16 {
    terminal.size().map(|s| s.height).unwrap_or(24)
}

fn load_log(state: &mut DashboardState) -> Result<()> {
    let meta = match fs::metadata(&state.log_path) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };

    let mtime = meta.modified().ok();
    let size = meta.len();

    if mtime == state.last_mtime && size == state.log_size.unwrap_or(0) {
        return Ok(());
    }

    state.last_mtime = mtime;
    state.log_size = Some(size);

    let mut file = fs::File::open(&state.log_path)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;

    state.lines.clear();
    for line in content.lines() {
        if state.lines.len() >= MAX_LINES {
            state.lines.pop_front();
        }
        state.lines.push_back(line.to_string());
    }

    Ok(())
}

fn read_pid(state: &mut DashboardState) {
    let content = match fs::read_to_string(&state.pid_path) {
        Ok(c) => c,
        Err(_) => {
            state.pid = None;
            state.running = false;
            return;
        }
    };

    let pid: i32 = match content.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            state.pid = None;
            state.running = false;
            return;
        }
    };

    state.pid = Some(pid);
    state.running = is_process_alive(pid);
}

fn is_process_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

fn draw_dashboard(frame: &mut Frame, state: &DashboardState) {
    let area = frame.area();

    let [status_area, log_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

    let status = format!(
        " PID: {}  |  {}  |  Lines: {}",
        state.pid.map_or("—".to_string(), |p| p.to_string()),
        if state.running {
            "● Running"
        } else {
            "○ Stopped"
        },
        state.lines.len(),
    );
    let status_style = if state.running {
        Style::new().fg(Color::Black).bg(Color::Green)
    } else {
        Style::new().fg(Color::White).bg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(status)
            .style(status_style)
            .add_modifier(Modifier::BOLD),
        status_area,
    );

    let block = Block::bordered()
        .title(" Daemon Log ")
        .title_bottom(format!(
            " {} | ↑↓ scroll | f follow toggle | q quit ",
            if state.follow { "FOLLOW" } else { "MANUAL" }
        ));

    let inner = block.inner(log_area);
    frame.render_widget(block, log_area);

    let visible_height = inner.height as usize;
    let total_lines = state.lines.len();

    let (start, _end) = if state.follow || state.scroll_offset == 0 {
        (total_lines.saturating_sub(visible_height), total_lines)
    } else {
        let end = total_lines.saturating_sub(state.scroll_offset);
        let start = end.saturating_sub(visible_height);
        (start, end)
    };

    let lines: Vec<&str> = state
        .lines
        .iter()
        .skip(start)
        .take(visible_height)
        .map(|s| s.as_str())
        .collect();

    frame.render_widget(
        Paragraph::new(Text::styled(lines.join("\n"), Style::default())),
        inner,
    );
}
