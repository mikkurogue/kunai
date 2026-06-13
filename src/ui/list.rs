use anyhow::Result;
use ratatui::{
    TerminalOptions,
    Viewport,
    crossterm::event::{
        self,
        Event,
        KeyCode,
        KeyEventKind,
    },
    layout::Constraint,
    style::{
        Color,
        Modifier,
        Style,
    },
    widgets::{
        Block,
        Row,
        Table,
    },
};

use crate::input;

pub fn run() -> Result<()> {
    let keyboards = input::list_keyboards()?;

    if keyboards.is_empty() {
        println!("No keyboards found.");
        println!("\nNote: You may need to be in the 'input' group:");
        println!("  sudo usermod -aG input $USER");
        println!("  (then log out and back in)");
        return Ok(());
    }

    let viewport_height = (keyboards.len() + 5) as u16;

    let mut terminal = ratatui::try_init_with_options(TerminalOptions {
        viewport: Viewport::Inline(viewport_height),
    })
    .map_err(|e| anyhow::anyhow!("Failed to initialize terminal: {}", e))?;

    terminal.draw(|frame| {
        let header_row = Row::new(["#", "Keyboard", "Path", "ID"]).style(
            Style::new()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

        let rows: Vec<Row> = keyboards
            .iter()
            .enumerate()
            .map(|(i, kb)| {
                let path = kb.device_path.display().to_string();
                let id = format!("{:04x}:{:04x}", kb.vendor_id, kb.product_id);
                Row::new([(i + 1).to_string(), kb.name.clone(), path, id])
                    .style(Style::new().fg(Color::White))
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(4),
                Constraint::Fill(1),
                Constraint::Fill(1),
                Constraint::Length(14),
            ],
        )
        .header(header_row)
        .block(Block::bordered().title(" Detected Keyboards "));

        frame.render_widget(table, frame.area());
    })?;

    loop {
        if let Event::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
            && matches!(
                key.code,
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc | KeyCode::Enter
            )
        {
            break;
        }
    }

    let _ = ratatui::try_restore();
    crate::ui::clear_inline(viewport_height);

    Ok(())
}
