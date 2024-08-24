use std::fs::DirEntry;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use eyre::Result;
use eyre::{anyhow, Context, Report};
use futures::{future, stream, FutureExt, Stream, StreamExt, TryStreamExt};
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::Deserialize;
use tokio_stream::wrappers::ReadDirStream;

use crate::config::Config;

type ProjectKey = PathBuf;

pub(crate) enum ProjectEvent {
    Add(Project),
    Update(ProjectKey, std::time::SystemTime),
}

#[derive(Debug, Deserialize)]
pub(crate) struct CargoMeta {
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
pub(crate) enum PackageMeta {
    Cargo(CargoMeta),
}

// pub(crate) struct ProjectDirInfo {
//     pub(crate) parent: Option<usize>,
//     pub(crate) modified: std::time::SystemTime,
//     pub(crate) immediate_child_count: usize,
//     pub(crate) total_child_count: usize,
// }

#[derive(Debug)]
pub(crate) struct Project {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) git: bool,
    pub(crate) package: Vec<PackageMeta>,
    pub(crate) readme: Option<String>,
    pub(crate) modified: std::time::SystemTime,
    pub(crate) file_count: usize,
}

impl Project {
    pub fn from_path(config: &Config, path: PathBuf) -> Result<Self> {
        let git = path.join(".git").exists();
        let package = Vec::new();
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let readme_path = path.join("README.md");
        let readme = if readme_path.exists() {
            Some(std::fs::read_to_string(readme_path)?)
        } else {
            None
        };

        let (modified, file_count) = get_file_summary(config, &path)?;

        Ok(Project {
            name,
            path,
            git,
            package,
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
}

impl ProjectLoader {
    pub(crate) fn new(config: Arc<Config>) -> Result<Self> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        let fetcher = tokio::spawn(Self::fetcher(config.clone(), tx.clone()).boxed());

        Ok(ProjectLoader { rx, _fetcher: fetcher })
    }

    pub(crate) async fn fetcher(
        config: Arc<Config>,
        tx: tokio::sync::mpsc::Sender<ProjectEvent>,
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
                let project =
                    Project::from_path(config.as_ref(), path).context("Failed to read project")?;
                tx.send(ProjectEvent::Add(project)).await?;
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
            // Poll::Ready(None) => if let Some(fetcher) = self.mut_fetcher {
            //     match self_mut.fetcher.poll_unpin(cx) {
            //         Poll::Ready(Err(e)) => Poll::Ready(Some(Err(Report::new(e)))),
            //         Poll::Ready(Ok(Ok(()))) => {
            //             Poll::Pending
            //         },
            //         Poll::Ready(Ok(Err(e))) => Poll::Ready(Some(Err(e))),
            //         Poll::Pending => Poll::Pending,
            //     }
            // } else {
            //     Poll::Pending
            // }
            Poll::Ready(None) => Poll::Pending,
            Poll::Ready(Some(event)) => Poll::Ready(Some(Ok(event))),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOpener {
    #[default]
    Auto,
    Code,
    Editor,
    Command(Vec<String>),
}

impl ProjectOpener {
    pub(crate) fn open(&self, project: &Project) -> Result<()> {
        match self {
            ProjectOpener::Auto => {
                if std::env::var("CODE").is_ok() {
                    Self::open_code(project)
                } else if std::env::var("EDITOR").is_ok() {
                    Self::open_editor(project)
                } else {
                    Err(anyhow!("vscode not found nor was an editor set"))
                }
            }
            ProjectOpener::Code => Self::open_code(project),
            ProjectOpener::Editor => Self::open_editor(project),
            ProjectOpener::Command(cmd) => Self::open_command(project, cmd),
        }
    }

    pub(crate) fn open_code(project: &Project) -> Result<()> {
        let mut child = std::process::Command::new("code")
            .arg(&project.path)
            .spawn()?;

        child.wait()?;

        Ok(())
    }

    pub(crate) fn open_editor(project: &Project) -> Result<()> {
        let editor =
            std::env::var("EDITOR").wrap_err("Could not read EDITOR environment variable")?;

        let mut child = std::process::Command::new(&editor)
            .arg(&project.path)
            .spawn()?;

        child.wait()?;

        Ok(())
    }

    pub(crate) fn open_command(project: &Project, cmd: &[String]) -> Result<()> {
        let mut child = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .arg(&project.path)
            .spawn()?;

        child.wait()?;

        Ok(())
    }
}
