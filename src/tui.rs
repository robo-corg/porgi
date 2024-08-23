use color_eyre::config::HookBuilder;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use eyre::{bail, Result};
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
    project::{Project, ProjectEvent, ProjectLoader, ProjectStore},
};

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

#[derive(Default)]
struct StatefulTable {
    state: TableState,
    items: ProjectStore,
    last_selected: Option<usize>,
}

/// This struct holds the current state of the app. In particular, it has the `items` field which is
/// a wrapper around `ListState`. Keeping track of the items state let us render the associated
/// widget with its state and have access to features such as natural scrolling.
///
/// Check the event handling at the bottom to see how to change the state on incoming events.
/// Check the drawing logic for items on how to specify the highlighting style for selected items.
pub(crate) struct App {
    config: Arc<Config>,
    items: StatefulTable,
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
            config,
            items: StatefulTable::new(),
            project_events,
        }
    }

    fn go_top(&mut self) {
        self.items.state.select(Some(0));
    }

    fn go_bottom(&mut self) {
        self.items.state.select(Some(self.items.items.len() - 1));
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

        loop {
            self.draw(&mut terminal)?;

            let mut event = reader.next().fuse();
            let mut project_event_fut = self.project_events.next().fuse();

            select! {
                project_event = project_event_fut => {
                    match project_event {
                        Some(Ok(event)) => {
                            match event {
                                ProjectEvent::Add(project) => {
                                    self.items.items.add(project);
                                }
                                ProjectEvent::Update(project_key, last_modified, file_count) => {
                                    let project = self.items.items.get_mut(&project_key).unwrap();
                                    project.modified = last_modified;
                                    project.file_count = file_count;
                                }
                            }
                            self.items.sort();
                        }
                        Some(Err(e)) => {
                            bail!(e);
                        }
                        None => break,
                    }
                },
                maybe_event = event => {
                    match maybe_event {
                        Some(Ok(Event::Key(key))) => {
                            if key.kind == KeyEventKind::Press {
                                use KeyCode::*;
                                match key.code {
                                    Esc => return Ok(()),
                                    KeyCode::Char('h') | Left => self.items.unselect(),
                                    KeyCode::Char('j') |  Down => self.items.next(),
                                    KeyCode::Char('k') | Up => self.items.previous(),
                                    KeyCode::Char('g') | KeyCode::Home => self.go_top(),
                                    KeyCode::Char('G') | KeyCode::End => self.go_bottom(),
                                    KeyCode::Char('o') => {
                                        // So far it seem sufficient to clear and force a redraw
                                        // but we may want to restore the terminal first before
                                        // launching an editor that runs in the terminal.
                                        self.open_project().await?;
                                        terminal.clear()?;
                                        self.draw(&mut terminal)?;
                                    },
                                    _ => {}
                                }
                            }
                        }
                        Some(Ok(_)) => {}
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

        self.render_projects(left, buf);

        if let Some(i) = self.items.state.selected() {
            let project = &self.items.items[i];
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

    fn render_projects(&mut self, area: Rect, buf: &mut Buffer) {
        // We create two blocks, one is for the header (outer) and the other is for list (inner).
        let outer_block = Block::new()
            .borders(Borders::NONE)
            .title_alignment(Alignment::Center)
            .title("Projects")
            .fg(self.config.colors.text_color)
            .bg(self.config.colors.project_header_bg);
        let inner_block = Block::new()
            .borders(Borders::RIGHT)
            .fg(self.config.colors.text_color)
            .bg(self.config.colors.normal_row_color);

        // We get the inner area from outer_block. We'll use this area later to render the table.
        let outer_area = area;
        let inner_area = outer_block.inner(outer_area);

        // We can render the header in outer_area.
        outer_block.render(outer_area, buf);

        // Iterate through all elements in the `items` and stylize them.
        // let items: Vec<ListItem> = self
        //     .items
        //     .items
        //     .iter()
        //     .map(|project| {
        //         let text = Line::from(vec![
        //             Span::raw(project.name.as_str()),
        //             Span::raw(format!(" files: {}", project.file_count)),
        //         ]);
        //         ListItem::new(text)
        //     })
        //     .collect();

        // Create a List from all list items and highlight the currently selected one
        // let items = List::new(items)
        //     .block(inner_block)
        //     .highlight_style(
        //         Style::default()
        //             .add_modifier(Modifier::BOLD)
        //             .add_modifier(Modifier::REVERSED)
        //             .fg(self.config.colors.selected_style_fg),
        //     )
        //     .highlight_symbol(">")
        //     .highlight_spacing(HighlightSpacing::Always);

        let rows = [Row::new(vec!["Cell1", "Cell2", "Cell3"])];
        // Columns widths are constrained in the same way as Layout...
        let widths = [
            Constraint::Length(5),
            Constraint::Length(10),
        ];

        let table = Table::new(rows, widths)
        // ...and they can be separated by a fixed spacing.
        .column_spacing(1)
        // You can set the style of the entire Table.
        .style(Style::new().blue())
        // It has an optional header, which is simply a Row always visible at the top.
        // It has an optional footer, which is simply a Row always visible at the bottom.
        .footer(Row::new(vec!["Updated on Dec 28"]))
        .block(inner_block)
        // The selected row and its content can also be styled.
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED)
                .fg(self.config.colors.selected_style_fg),
        )
        // ...and potentially show a symbol in front of the selection.
        .highlight_symbol(">>");

        // We can now render the item list
        // (look careful we are using StatefulWidget's render.)
        // ratatui::widgets::StatefulWidget::render as stateful_render
        StatefulWidget::render(table, inner_area, buf, &mut self.items.state);
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

impl StatefulTable {
    fn new() -> Self {
        Self {
            state: TableState::default(),
            items: ProjectStore::default(),
            last_selected: None,
        }
    }

    fn sort(&mut self) {
        self.items.sort();
    }

    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => self.last_selected.unwrap_or(0),
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => self.last_selected.unwrap_or(0),
        };
        self.state.select(Some(i));
    }

    fn unselect(&mut self) {
        let offset = self.state.offset();
        self.last_selected = self.state.selected();
        self.state.select(None);
        *self.state.offset_mut() = offset;
    }

    fn current(&self) -> Option<&Project> {
        self.state.selected().map(|i| &self.items[i])
    }
}
