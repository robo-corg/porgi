//! Tool to organize code projects
//!
//! Collects status for projects and their git status as well other metadata

use color_eyre::config::HookBuilder;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{prelude::*, style::palette::tailwind, widgets::*};

const TODO_HEADER_BG: Color = tailwind::BLUE.c950;
const NORMAL_ROW_COLOR: Color = tailwind::SLATE.c950;
const ALT_ROW_COLOR: Color = tailwind::SLATE.c900;
const SELECTED_STYLE_FG: Color = tailwind::BLUE.c300;
const TEXT_COLOR: Color = tailwind::SLATE.c200;
const COMPLETED_TEXT_COLOR: Color = tailwind::GREEN.c500;

use eyre::Result;
use itertools::Itertools;
use serde::Deserialize;
use std::{
    io::{self, stdout},
    path::PathBuf,
};

#[derive(Debug, Deserialize)]
struct CargoMeta {
    name: String,
    version: String,
    authors: Vec<String>,
    description: Option<String>,
    license: Option<String>,
    repository: Option<String>,
    dependencies: Vec<String>,
    dev_dependencies: Vec<String>,
}

#[derive(Debug)]
enum PackageMeta {
    Cargo(CargoMeta),
}

#[derive(Debug)]
struct Project {
    name: String,
    path: PathBuf,
    git: bool,
    package: Vec<PackageMeta>,
}

impl Project {
    fn from_path(path: PathBuf) -> Result<Self> {
        let git = path.join(".git").exists();
        let package = Vec::new();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        Ok(Project { name, path, git, package })
    }
}

struct StatefulList {
    state: ListState,
    items: Vec<Project>,
    last_selected: Option<usize>,
}

/// This struct holds the current state of the app. In particular, it has the `items` field which is
/// a wrapper around `ListState`. Keeping track of the items state let us render the associated
/// widget with its state and have access to features such as natural scrolling.
///
/// Check the event handling at the bottom to see how to change the state on incoming events.
/// Check the drawing logic for items on how to specify the highlighting style for selected items.
struct App {
    search: String,
    items: StatefulList,
}

fn main() -> Result<()> {
    // setup terminal
    init_error_hooks()?;
    let terminal = init_terminal()?;

    let projects_dir = PathBuf::from(shellexpand::tilde("~/projects").into_owned());

    let projects_iter = projects_dir
        .read_dir()
        .unwrap()
        .filter_map_ok(|entry| {
            let path = entry.path();
            if path.is_dir() {
                Some(Project::from_path(path))
            } else {
                None
            }
        })
        .flatten();

    let projects = projects_iter.collect::<Result<Vec<_>>>()?;

    // create app and run it
    App::new(projects).run(terminal)?;

    restore_terminal()?;

    Ok(())
}

fn init_error_hooks() -> color_eyre::Result<()> {
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

fn init_terminal() -> color_eyre::Result<Terminal<impl Backend>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal() -> color_eyre::Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

impl App {
    fn new(projects: Vec<Project>) -> Self {
        Self {
            search: String::new(),
            items: StatefulList::with_projects(projects),
        }
    }

    fn go_top(&mut self) {
        self.items.state.select(Some(0));
    }

    fn go_bottom(&mut self) {
        self.items.state.select(Some(self.items.items.len() - 1));
    }
}

impl App {
    fn run(&mut self, mut terminal: Terminal<impl Backend>) -> io::Result<()> {
        loop {
            self.draw(&mut terminal)?;

            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    use KeyCode::*;
                    match key.code {
                        Esc => return Ok(()),
                        Left => self.items.unselect(),
                        Down => self.items.next(),
                        Up => self.items.previous(),
                        KeyCode::Char(ch) => {
                            self.search.push(ch);
                        }
                        Backspace => {
                            self.search.pop();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn draw(&mut self, terminal: &mut Terminal<impl Backend>) -> io::Result<()> {
        terminal.draw(|f| f.render_widget(self, f.size()))?;
        Ok(())
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let horizontal = Layout::horizontal([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ]);

        let [left, right] = horizontal.areas(area);

        // Create two chunks with equal vertical screen space. One for the list and the other for
        // the info block.
        let vertical = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);
        let [upper_item_list_area, input_area] = vertical.areas(left);

        self.render_projects(upper_item_list_area, buf);

        if let Some(i) = self.items.state.selected() {
            let project = &self.items.items[i];
            self.render_info(project, right, buf);
        }

        Paragraph::new(self.search.as_str())
            .style(Style::default().fg(Color::Yellow))
            .render(input_area, buf);
    }
}

fn fuzzy_match(search: &str, item: &str) -> bool {
    let search_chars = search.chars();
    let mut item_chars = item.chars();

    'outer: for s_ch in search_chars {
        while let Some(i_ch) = item_chars.next() {
            if s_ch == i_ch {
                continue 'outer;
            }
        }

        return false;
    }

    true
}

impl App {
    fn render_projects(&mut self, area: Rect, buf: &mut Buffer) {
        // We create two blocks, one is for the header (outer) and the other is for list (inner).
        let outer_block = Block::new()
            .borders(Borders::NONE)
            .title_alignment(Alignment::Center)
            .title("Projects")
            .fg(TEXT_COLOR)
            .bg(TODO_HEADER_BG);
        let inner_block = Block::new()
            .borders(Borders::NONE)
            .fg(TEXT_COLOR)
            .bg(NORMAL_ROW_COLOR);

        // We get the inner area from outer_block. We'll use this area later to render the table.
        let outer_area = area;
        let inner_area = outer_block.inner(outer_area);

        // We can render the header in outer_area.
        outer_block.render(outer_area, buf);

        // Iterate through all elements in the `items` and stylize them.
        let items: Vec<ListItem> = self
            .items
            .items
            .iter()
            .enumerate()
            .filter(|(_, project)| {
                fuzzy_match(&self.search, &project.path.display().to_string())
            })
            .map(|(i, project)|
                ListItem::new(project.name.as_str())
            )
            .collect();

        // Create a List from all list items and highlight the currently selected one
        let items = List::new(items)
            .block(inner_block)
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
                    .fg(SELECTED_STYLE_FG),
            )
            .highlight_symbol(">")
            .highlight_spacing(HighlightSpacing::Always);

        // We can now render the item list
        // (look careful we are using StatefulWidget's render.)
        // ratatui::widgets::StatefulWidget::render as stateful_render
        StatefulWidget::render(items, inner_area, buf, &mut self.items.state);
    }

    fn render_info(&self, project: &Project, area: Rect, buf: &mut Buffer) {
        // We get the info depending on the item's state.
        let info = format!("{:?}", project);

        // We show the list item's info under the list in this paragraph
        let outer_info_block = Block::new()
            .borders(Borders::NONE)
            .title_alignment(Alignment::Center)
            .title(project.name.as_str())
            .fg(TEXT_COLOR)
            .bg(TODO_HEADER_BG);

        let inner_info_block = Block::new()
            .borders(Borders::NONE)
            .padding(Padding::horizontal(1))
            .bg(NORMAL_ROW_COLOR);

        // This is a similar process to what we did for list. outer_info_area will be used for
        // header inner_info_area will be used for the list info.
        let outer_info_area = area;
        let inner_info_area = outer_info_block.inner(outer_info_area);

        // We can render the header. Inner info will be rendered later
        outer_info_block.render(outer_info_area, buf);

        let info_paragraph = Paragraph::new(info)
            .block(inner_info_block)
            .fg(TEXT_COLOR)
            .wrap(Wrap { trim: false });

        // We can now render the item info
        info_paragraph.render(inner_info_area, buf);
    }
}

impl StatefulList {
    fn with_projects(projects: Vec<Project>) -> Self {
        StatefulList {
            state: ListState::default(),
            items: projects,
            last_selected: None,
        }
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
}
