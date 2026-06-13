use anyhow::Result;
use ratatui::{
    DefaultTerminal,
    Frame,
    TerminalOptions,
    Viewport,
    crossterm::event::{
        self,
        Event,
        KeyCode,
        KeyEventKind,
    },
    layout::{
        Constraint,
        Layout,
        Rect,
    },
    style::{
        Color,
        Modifier,
        Style,
    },
    text::Span,
    widgets::{
        Block,
        Cell,
        Clear,
        List,
        Paragraph,
        Row,
        Table,
    },
};

use crate::{
    config::{
        Config,
        KeyboardConfig,
    },
    input,
    niri,
};

const HEADER_BG: Color = Color::DarkGray;
const SELECTED_BG: Color = Color::Blue;
const ACCENT: Color = Color::Cyan;
const UNSET: Color = Color::DarkGray;

struct SetupState {
    assignments:   Vec<Option<usize>>,
    row:           usize,
    mode:          Mode,
    layout_cursor: usize,
}

enum Mode {
    Browsing,
    ChoosingLayout { keyboard_idx: usize },
}

pub fn run(dry_run: bool) -> Result<()> {
    let keyboards = input::list_keyboards()?;
    let layouts = niri::get_layouts()?;

    if keyboards.is_empty() {
        anyhow::bail!("No keyboards detected. Check permissions.");
    }

    let viewport_height = (keyboards.len() + 6) as u16;

    let mut terminal = ratatui::try_init_with_options(TerminalOptions {
        viewport: Viewport::Inline(viewport_height),
    })
    .map_err(|e| anyhow::anyhow!("Failed to initialize terminal: {}", e))?;

    let mut state = SetupState {
        assignments:   vec![None; keyboards.len()],
        row:           0,
        mode:          Mode::Browsing,
        layout_cursor: 0,
    };

    let saved = run_loop(&mut terminal, &mut state, &keyboards, &layouts, dry_run)?;

    let _ = ratatui::try_restore();
    crate::ui::clear_inline(viewport_height);

    if saved {
        let config = build_config(&state, &keyboards, &layouts);
        if dry_run {
            println!("\nDry-run");
            println!("Would save to ~/.config/kunai/config.toml:\n");
            println!("{}", toml::to_string(&config).unwrap_or_default());
        } else {
            config.save()?;
            println!("\n✓ Configuration saved!");
            println!("\nAdd to your niri config (~/.config/niri/config.kdl):");
            println!("  spawn-at-startup \"kunai\" \"daemon\"");
            println!("\nOr run manually:");
            println!("  kunai daemon");
        }
    }

    Ok(())
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    state: &mut SetupState,
    keyboards: &[input::Keyboard],
    layouts: &[String],
    dry_run: bool,
) -> Result<bool> {
    loop {
        terminal.draw(|frame| draw_wizard(frame, state, keyboards, layouts, dry_run))?;

        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match state.mode {
                Mode::Browsing => match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.row = state.row.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if state.row + 1 < keyboards.len() {
                            state.row += 1;
                        }
                    }
                    KeyCode::Enter => {
                        state.layout_cursor = state.assignments[state.row].unwrap_or(0);
                        state.mode = Mode::ChoosingLayout {
                            keyboard_idx: state.row,
                        };
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => return Ok(true),
                    KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(false),
                    _ => {}
                },
                Mode::ChoosingLayout { keyboard_idx } => match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.layout_cursor = state.layout_cursor.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if state.layout_cursor + 1 < layouts.len() {
                            state.layout_cursor += 1;
                        }
                    }
                    KeyCode::Enter => {
                        state.assignments[keyboard_idx] = Some(state.layout_cursor);
                        state.mode = Mode::Browsing;
                    }
                    KeyCode::Esc => {
                        state.mode = Mode::Browsing;
                    }
                    _ => {}
                },
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn draw_wizard(
    frame: &mut Frame,
    state: &SetupState,
    keyboards: &[input::Keyboard],
    layouts: &[String],
    dry_run: bool,
) {
    let area = frame.area();

    let [table_area, help_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    let header_row = Row::new(["", "Keyboard", "ID", "Layout"]).style(
        Style::new()
            .bg(HEADER_BG)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = keyboards
        .iter()
        .enumerate()
        .map(|(i, kb)| {
            let indicator = if i == state.row { "▸" } else { " " };
            let id = format!("{:04x}:{:04x}", kb.vendor_id, kb.product_id);
            let layout_name = match state.assignments[i] {
                Some(idx) => layouts[idx].as_str(),
                None => "— unset —",
            };
            let layout_style = if state.assignments[i].is_some() {
                Style::default()
            } else {
                Style::new().fg(UNSET).italic()
            };

            let cells = vec![
                Cell::from(indicator),
                Cell::from(kb.name.as_str()),
                Cell::from(id),
                Cell::from(Span::styled(layout_name, layout_style)),
            ];

            let style = if i == state.row {
                Style::new().bg(SELECTED_BG)
            } else {
                Style::default()
            };

            Row::new(cells).style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(14),
            Constraint::Fill(1),
        ],
    )
    .header(header_row)
    .block(Block::bordered().title(format!(
        " Keyboard Setup{} ",
        if dry_run { " — DRY RUN" } else { "" }
    )));

    frame.render_widget(table, table_area);

    let help_text = if matches!(state.mode, Mode::Browsing) {
        if dry_run {
            "↑↓ nagivate  |  Enter choose layout  |  s show config  |  q quit"
        } else {
            "↑↓ nagivate  |  Enter choose layout  |  s save  |  q quit"
        }
    } else {
        "↑↓ select  |  Enter confirm  |  Esc cancel"
    };
    frame.render_widget(
        Paragraph::new(help_text).style(Style::new().fg(Color::DarkGray)),
        help_area,
    );

    if let Mode::ChoosingLayout { keyboard_idx } = state.mode {
        draw_layout_popup(frame, area, layouts, keyboard_idx, state.layout_cursor);
    }
}

fn draw_layout_popup(
    frame: &mut Frame,
    area: Rect,
    layouts: &[String],
    _keyboard_idx: usize,
    cursor: usize,
) {
    let popup_width = 46.min(area.width.saturating_sub(4));
    let popup_height = (layouts.len() as u16 + 2).min(area.height.saturating_sub(4));

    let vert = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(popup_height),
        Constraint::Fill(1),
    ]);
    let horz = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(popup_width),
        Constraint::Fill(1),
    ]);
    let popup_area = horz.split(vert.split(area)[1])[1];

    frame.render_widget(Clear, popup_area);

    let items: Vec<ratatui::widgets::ListItem> = layouts
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let prefix = if i == cursor { "▸ " } else { "  " };
            let mut item = ratatui::widgets::ListItem::new(format!("{}{}", prefix, name));
            if i == cursor {
                item = item.style(
                    Style::new()
                        .bg(ACCENT)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                );
            }
            item
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Select Layout ")
                .border_style(Style::new().fg(ACCENT)),
        )
        .highlight_style(Style::new().bg(ACCENT));

    frame.render_widget(list, popup_area);
}

fn build_config(state: &SetupState, keyboards: &[input::Keyboard], _layouts: &[String]) -> Config {
    let kb_configs: Vec<KeyboardConfig> = keyboards
        .iter()
        .enumerate()
        .filter_map(|(i, kb)| {
            let layout_index = state.assignments[i]?;
            let name = kb.name.clone();
            Some(KeyboardConfig {
                name,
                vendor_id: format!("{:04x}", kb.vendor_id),
                product_id: format!("{:04x}", kb.product_id),
                layout_index: layout_index as u32,
            })
        })
        .collect();

    Config {
        keyboards: kb_configs,
    }
}
