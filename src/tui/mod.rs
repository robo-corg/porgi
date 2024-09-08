use color_eyre::config::HookBuilder;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use eyre::Result;
use futures::{future::FutureExt, select, StreamExt};
use ratatui::{prelude::*, style::palette::tailwind, widgets::*};

const INFO_TEXT: &str =
    "(Esc) quit | (↑) move up | (↓) move down | (o) open project | (←) unselect";

use serde::Deserialize;
use std::{
    io::{self, stdout},
    sync::Arc,
};

use crate::{
    config::Config,
    project::{Project, ProjectLoader},
    tui::project_table::ProjectTable,
};

mod project_table;

#[derive(Debug, Deserialize)]
pub struct ColorConfig {
    normal_row_color: Color,
    selected_style_fg: Color,
    text_color: Color,
    project_header_bg: Color,
    footer_border_color: Color,
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            normal_row_color: tailwind::SLATE.c950,
            selected_style_fg: tailwind::BLUE.c300,
            text_color: tailwind::SLATE.c200,
            project_header_bg: tailwind::BLUE.c950,
            footer_border_color: tailwind::BLUE.c300,
        }
    }
}

/// This struct holds the current state of the app. In particular, it has the `items` field which is
/// a wrapper around `ListState`. Keeping track of the items state let us render the associated
/// widget with its state and have access to features such as natural scrolling.
///
/// Check the event handling at the bottom to see how to change the state on incoming events.
/// Check the drawing logic for items on how to specify the highlighting style for selected items.
pub(crate) struct App {
    quit: bool,
    config: Arc<Config>,
    items: ProjectTable,
    project_events: ProjectLoader,
}

pub(crate) fn init_error_hooks() -> color_eyre::Result<()> {
    let (panic, error) = HookBuilder::default().into_hooks();
    let panic = panic.into_panic_hook();
    let error = error.into_eyre_hook();
    color_eyre::eyre::set_hook(Box::new(move |e| {
        let _ = restore_terminal();
        error(e)
    }))?;
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        panic(info);
    }));
    Ok(())
}

pub(crate) fn init_terminal() -> color_eyre::Result<Terminal<impl Backend>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

pub(crate) fn restore_terminal() -> color_eyre::Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

impl App {
    pub(crate) fn new(config: Arc<Config>, project_events: ProjectLoader) -> Self {
        Self {
            quit: false,
            config,
            items: ProjectTable::new(),
            project_events,
        }
    }

    async fn open_project(&mut self) -> Result<()> {
        if let Some(project) = self.items.current() {
            self.config.opener.open(project).await?;
        }

        Ok(())
    }
}

impl App {
    pub(crate) async fn run(&mut self, mut terminal: Terminal<impl Backend>) -> Result<()> {
        let mut reader = EventStream::new();

        while !self.quit {
            self.draw(&mut terminal)?;

            let mut event = reader.next().fuse();
            let mut project_event_fut = self.project_events.next().fuse();

            select! {
                project_event = project_event_fut => {
                    if let Some(project_event) = project_event.transpose()? {
                        self.items.update(project_event)?;
                    }
                },
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            self.handle_input(&mut terminal, event).await?;
                        }
                        Some(Err(e)) => {
                            eprintln!("Error: {}", e);
                        }
                        None => break,
                    }
                }
            };
        }

        Ok(())
    }

    pub(crate) async fn handle_input(
        &mut self,
        terminal: &mut Terminal<impl Backend>,
        event: Event,
    ) -> Result<()> {
        use KeyCode::*;

        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match key.code {
                    Esc => {
                        self.quit = true;
                    }
                    KeyCode::Char('o') => {
                        // So far it seem sufficient to clear and force a redraw
                        // but we may want to restore the terminal first before
                        // launching an editor that runs in the terminal.
                        self.open_project().await?;
                        terminal.clear()?;
                        self.draw(terminal)?;
                        return Ok(());
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        self.items.handle_input(event).await?;
        Ok(())
    }

    fn draw(&mut self, terminal: &mut Terminal<impl Backend>) -> io::Result<()> {
        terminal.draw(|f| f.render_widget(self, f.size()))?;
        Ok(())
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rects = Layout::vertical([Constraint::Min(5), Constraint::Length(3)]).split(area);

        self.render_body(rects[0], buf);
        self.render_footer(rects[1], buf);
    }
}

impl App {
    fn render_body(&mut self, area: Rect, buf: &mut Buffer) {
        // Create a layout with 2 columns
        let horizontal =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]);

        let [left, right] = horizontal.areas(area);

        // Create two chunks with equal vertical screen space. One for the list and the other for
        // the info block.
        //let vertical = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]);

        //let [upper_item_list_area, input_area] = vertical.areas(left);

        self.items.render(&self.config, left, buf);

        if let Some(project) = self.items.current() {
            self.render_info(project, right, buf);
        }

        // TODO: Add this back when search is done properly
        // Paragraph::new(self.search.as_str())
        //     .style(Style::default().fg(Color::Yellow))
        //     .render(input_area, buf);
    }

    fn render_footer(&mut self, area: Rect, buf: &mut Buffer) {
        let info_footer = Paragraph::new(Line::from(INFO_TEXT))
            .style(
                Style::new()
                    .fg(self.config.colors.text_color)
                    .bg(self.config.colors.normal_row_color),
            )
            .centered()
            .block(
                Block::bordered()
                    .border_type(BorderType::Double)
                    .border_style(Style::new().fg(self.config.colors.footer_border_color)),
            );
        info_footer.render(area, buf);
        //f.render_widget(info_footer, area);
    }

    fn render_info(&self, project: &Project, area: Rect, buf: &mut Buffer) {
        // We get the info depending on the item's state.
        let info = format!(
            "{}\n{}",
            project.name,
            project.readme.as_deref().unwrap_or(""),
        );

        // We show the list item's info under the list in this paragraph
        let outer_info_block = Block::new()
            .borders(Borders::NONE)
            .title_alignment(Alignment::Center)
            .title(project.name.as_str())
            .fg(self.config.colors.text_color)
            .bg(self.config.colors.project_header_bg);

        let inner_info_block = Block::new()
            .borders(Borders::NONE)
            .padding(Padding::horizontal(1))
            .bg(self.config.colors.normal_row_color);

        // This is a similar process to what we did for list. outer_info_area will be used for
        // header inner_info_area will be used for the list info.
        let outer_info_area = area;
        let inner_info_area = outer_info_block.inner(outer_info_area);

        // We can render the header. Inner info will be rendered later
        outer_info_block.render(outer_info_area, buf);

        let info_paragraph = Paragraph::new(info)
            .block(inner_info_block)
            .fg(self.config.colors.text_color)
            .wrap(Wrap { trim: false });

        // We can now render the item info
        info_paragraph.render(inner_info_area, buf);
    }
}
