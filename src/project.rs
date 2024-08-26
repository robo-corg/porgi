use std::collections::HashMap;
use std::io;
use std::ops::Index;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use eyre::{anyhow, Context};
use eyre::{OptionExt, Result};
use futures::{future, stream, FutureExt, Stream, StreamExt, TryStreamExt};
use ignore::WalkBuilder;
use serde::Deserialize;
use tokio::process;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::wrappers::{ReadDirStream, ReceiverStream};
use which::which;

use crate::config::Config;

pub(crate) type ProjectKey = PathBuf;

pub(crate) enum ProjectEvent {
    Add(Project),
    Update(ProjectKey, std::time::SystemTime, usize),
}

#[derive(Debug, Default)]
pub(crate) struct ProjectStore {
    project_by_key: HashMap<ProjectKey, usize>,
    display_order: Vec<usize>,
    projects: Vec<Project>,
}

impl ProjectStore {
    pub(crate) fn sort(&mut self) {
        self.display_order
            .sort_by(|a, b| self.projects[*a].name.cmp(&self.projects[*b].name));
        self.display_order
            .sort_by(|a, b| self.projects[*b].modified.cmp(&self.projects[*a].modified));
    }

    pub(crate) fn add(&mut self, project: Project) {
        let key = project.key().clone();
        let idx = self.projects.len();
        self.projects.push(project);
        self.display_order.push(idx);
        if self.project_by_key.insert(key, idx).is_some() {
            panic!("Duplicate project key");
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.projects.len()
    }

    pub(crate) fn get_mut(&mut self, key: &ProjectKey) -> Option<&mut Project> {
        self.project_by_key
            .get(key)
            .map(|idx| &mut self.projects[*idx])
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Project> {
        self.display_order
            .iter()
            .map(move |idx| &self.projects[*idx])
    }
}

impl Index<usize> for ProjectStore {
    type Output = Project;

    fn index(&self, index: usize) -> &Self::Output {
        &self.projects[self.display_order[index]]
    }
}

#[derive(Debug)]
pub(crate) struct Project {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) readme: Option<String>,
    pub(crate) modified: std::time::SystemTime,
    pub(crate) file_count: usize,
}

impl Project {
    pub fn from_path(_config: &Config, path: PathBuf) -> Result<Self> {
        let name = path
            .file_name()
            .ok_or_eyre("Project path does not have a name")?
            .to_string_lossy()
            .to_string();

        let readme_path = path.join("README.md");
        let readme = if readme_path.exists() {
            Some(std::fs::read_to_string(readme_path)?)
        } else {
            None
        };

        let (modified, file_count) = (std::fs::metadata(path.as_path())?.modified()?, 0);

        Ok(Project {
            name,
            path,
            readme,
            modified,
            file_count,
        })
    }

    pub(crate) fn key(&self) -> &ProjectKey {
        &self.path
    }
}

fn get_file_summary(_config: &Config, path: &Path) -> Result<(std::time::SystemTime, usize)> {
    let mut modified = {
        let metadata = std::fs::metadata(path)?;
        metadata.modified()?
    };

    let mut file_count = 0;

    WalkBuilder::new(path)
        .standard_filters(true)
        .build()
        .filter_map(Result::ok)
        .filter_map(|path| path.metadata().ok())
        .filter_map(|metadata| metadata.modified().ok())
        .for_each(|modified_time| {
            file_count += 1;
            if modified_time > modified {
                modified = modified_time;
            }
        });

    Ok((modified, file_count))
}

pub(crate) struct ProjectLoader {
    rx: tokio::sync::mpsc::Receiver<ProjectEvent>,
    _fetcher: tokio::task::JoinHandle<Result<()>>,
    _walker: tokio::task::JoinHandle<Result<()>>,
}

impl ProjectLoader {
    pub(crate) fn new(config: Arc<Config>) -> Result<Self> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let (walker_tx, walker_rx): (Sender<PathBuf>, Receiver<PathBuf>) =
            tokio::sync::mpsc::channel(100);

        let fetcher = tokio::spawn(Self::fetcher(config.clone(), tx.clone(), walker_tx).boxed());

        let walker_rx_stream = ReceiverStream::new(walker_rx);

        let walker = tokio::spawn(async move {
            walker_rx_stream
                .map::<Result<PathBuf>, _>(Ok)
                .try_for_each_concurrent(8, move |path| {
                    let config = config.clone();
                    let tx = tx.clone();
                    async move {
                        let summary_path = path.clone();
                        let (modified, file_count) = tokio::task::spawn_blocking(move || {
                            get_file_summary(config.as_ref(), &summary_path)
                        })
                        .await??;

                        tx.send(ProjectEvent::Update(path.to_owned(), modified, file_count))
                            .await?;
                        Ok(())
                    }
                })
                .await
        });

        Ok(ProjectLoader {
            rx,
            _fetcher: fetcher,
            _walker: walker,
        })
    }

    pub(crate) async fn fetcher(
        config: Arc<Config>,
        tx: tokio::sync::mpsc::Sender<ProjectEvent>,
        tx_walker: tokio::sync::mpsc::Sender<PathBuf>,
    ) -> Result<()> {
        let project_dirs: Vec<PathBuf> = config
            .project_dirs
            .iter()
            .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()))
            .collect();

        let entries_stream = stream::iter(project_dirs.into_iter())
            .then(|d| async {
                let res: io::Result<_> = Ok(ReadDirStream::new(tokio::fs::read_dir(d).await?));
                res
            })
            .try_flatten()
            .map_err(eyre::Report::new);

        entries_stream
            .try_filter_map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    future::ok(Some(path))
                } else {
                    future::ok(None)
                }
            })
            .try_for_each_concurrent(8, |path| async {
                let tx = tx.clone();
                let project = Project::from_path(config.as_ref(), path.clone())
                    .context("Failed to read project")?;
                tx.send(ProjectEvent::Add(project)).await?;
                tx_walker.send(path).await?;
                Ok(())
            })
            .await?;

        Ok(())
    }
}

impl Stream for ProjectLoader {
    type Item = Result<ProjectEvent>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut futures::task::Context,
    ) -> Poll<Option<Self::Item>> {
        let self_mut = self.get_mut();

        match self_mut.rx.poll_recv(cx) {
            // TODO: Need to poll _fetcher and _walker here also to propagate errors
            Poll::Ready(None) => Poll::Pending,
            Poll::Ready(Some(event)) => Poll::Ready(Some(Ok(event))),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AddPathToArgs {
    #[default]
    Auto,
    Last,
    Never,
}

#[derive(Debug, Deserialize)]
pub struct Command {
    args: Vec<String>,
    #[serde(default = "ProjectOpener::chdir_default")]
    chdir: bool,
    #[serde(default)]
    add_path_to_args: AddPathToArgs,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOpener {
    #[default]
    Auto,
    Code,
    Editor,
    Command(Command),
}

impl ProjectOpener {
    fn chdir_default() -> bool {
        true
    }

    pub(crate) async fn open(&self, project: &Project) -> Result<()> {
        match self {
            ProjectOpener::Auto => {
                if which("code").is_ok() {
                    Self::open_code(project).await
                } else if std::env::var("EDITOR").is_ok() {
                    Self::open_editor(project).await
                } else {
                    Err(anyhow!("vscode not found nor was an editor set"))
                }
            }
            ProjectOpener::Code => Self::open_code(project).await,
            ProjectOpener::Editor => Self::open_editor(project).await,
            ProjectOpener::Command(cmd) => Self::open_command(project, cmd).await,
        }
    }

    pub(crate) async fn open_code(project: &Project) -> Result<()> {
        let mut child = process::Command::new("code").arg(&project.path).spawn()?;

        child.wait().await?;

        Ok(())
    }

    pub(crate) async fn open_editor(project: &Project) -> Result<()> {
        let editor =
            std::env::var("EDITOR").wrap_err("Could not read EDITOR environment variable")?;

        let mut child = process::Command::new(&editor)
            .current_dir(&project.path)
            .arg(&project.path)
            .spawn()?;

        child.wait().await?;

        Ok(())
    }

    pub(crate) async fn open_command(project: &Project, cmd: &Command) -> Result<()> {
        let mut proc: process::Command = process::Command::new(&cmd.args[0]);

        proc.args(&cmd.args[1..]);

        match cmd.add_path_to_args {
            AddPathToArgs::Auto => {
                proc.arg(&project.path);
            }
            AddPathToArgs::Last => {
                proc.arg(&project.path);
            }
            AddPathToArgs::Never => {}
        }

        if cmd.chdir {
            proc.current_dir(&project.path);
        }

        let mut child = proc.spawn()?;

        child.wait().await?;

        Ok(())
    }
}
