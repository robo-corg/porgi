use chrono::{DateTime, Local, TimeDelta};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use eyre::Result;
use fancy_duration::FancyDuration;
use ratatui::{prelude::*, widgets::*};

use crate::{
    config::Config,
    project::{Project, ProjectEvent, ProjectStore},
};

#[derive(Default)]
pub(crate) struct ProjectTable {
    state: TableState,
    items: ProjectStore,
    last_selected: Option<usize>,
}

impl ProjectTable {
    pub(crate) fn new() -> Self {
        Self {
            state: TableState::default(),
            items: ProjectStore::default(),
            last_selected: None,
        }
    }

    fn go_top(&mut self) {
        self.state.select(Some(0));
    }

    fn go_bottom(&mut self) {
        self.state.select(Some(self.items.len() - 1));
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

    pub(crate) fn current(&self) -> Option<&Project> {
        self.state.selected().map(|i| &self.items[i])
    }

    pub(crate) fn update(&mut self, event: ProjectEvent) -> Result<()> {
        self.items.update(event)
    }

    pub(crate) async fn handle_input(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('h') | KeyCode::Left => self.unselect(),
                KeyCode::Char('j') | KeyCode::Down => self.next(),
                KeyCode::Char('k') | KeyCode::Up => self.previous(),
                KeyCode::Char('g') | KeyCode::Home => self.go_top(),
                KeyCode::Char('G') | KeyCode::End => self.go_bottom(),
                _ => {}
            },
            _ => {}
        }

        Ok(())
    }

    pub(crate) fn render(&mut self, config: &Config, area: Rect, buf: &mut Buffer) {
        // We create two blocks, one is for the header (outer) and the other is for list (inner).
        let outer_block = Block::new()
            .borders(Borders::NONE)
            .title_alignment(Alignment::Center)
            .title("Projects")
            .fg(config.colors.text_color)
            .bg(config.colors.project_header_bg);
        let inner_block = Block::new()
            .borders(Borders::RIGHT)
            .fg(config.colors.text_color)
            .bg(config.colors.normal_row_color);

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

        let rows: Vec<Row> = self
            .items
            .iter()
            .map(|project| {
                Row::new(vec![project.name.clone(), {
                    let now: DateTime<Local> = Local::now();
                    let date: DateTime<Local> = project.modified.into();
                    let d = now.signed_duration_since(date);

                    if d.abs() < TimeDelta::new(60, 0).unwrap() {
                        "just now".to_string()
                    } else if d.abs() > TimeDelta::new(48 * 60 * 60, 0).unwrap() {
                        let date: DateTime<Local> = project.modified.into();
                        date.format("%Y-%m-%d").to_string()
                    } else if d >= TimeDelta::zero() {
                        format!(
                            "{} ago",
                            FancyDuration::new(d).filter(&[
                                fancy_duration::DurationPart::Days,
                                fancy_duration::DurationPart::Hours,
                                fancy_duration::DurationPart::Minutes,
                            ])
                        )
                    } else {
                        format!(
                            "{} from now",
                            FancyDuration::new(d.abs()).filter(&[
                                fancy_duration::DurationPart::Days,
                                fancy_duration::DurationPart::Hours,
                                fancy_duration::DurationPart::Minutes,
                            ])
                        )
                    }
                }])
            })
            .collect();

        //let rows = [Row::new(vec!["Cell1", "Cell2"])];
        // Columns widths are constrained in the same way as Layout...
        let widths = [Constraint::Fill(1), Constraint::Length(16)];

        let table = Table::new(rows, widths)
            // ...and they can be separated by a fixed spacing.
            .column_spacing(1)
            // You can set the style of the entire Table.
            .style(Style::new().blue())
            // It has an optional header, which is simply a Row always visible at the top.
            // It has an optional footer, which is simply a Row always visible at the bottom.
            //.footer(Row::new(vec!["Updated on Dec 28"]))
            // .footer(
            // )
            .block(inner_block)
            // The selected row and its content can also be styled.
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
                    .fg(config.colors.selected_style_fg),
            )
            // ...and potentially show a symbol in front of the selection.
            .highlight_symbol(">");

        // We can now render the item list
        // (look careful we are using StatefulWidget's render.)
        // ratatui::widgets::StatefulWidget::render as stateful_render
        StatefulWidget::render(table, inner_area, buf, &mut self.state);
    }
}
