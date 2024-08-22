use std::fs::DirEntry;
use std::path::{Path, PathBuf};

use eyre::Result;
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::Deserialize;

use crate::config::Config;

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
pub(crate) enum PackageMeta {
    Cargo(CargoMeta),
}

#[derive(Debug)]
pub(crate) struct Project {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) git: bool,
    pub(crate) package: Vec<PackageMeta>,
    pub(crate) readme: Option<String>,
    pub(crate) modified: std::time::SystemTime,
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

        let modified = get_modified_time(config, &path)?;

        Ok(Project {
            name,
            path,
            git,
            package,
            readme,
            modified,
        })
    }
}

fn get_modified_time(config: &Config, path: &Path) -> Result<std::time::SystemTime> {
    let mut modified = {
        let metadata = std::fs::metadata(path)?;
        metadata.modified()?
    };

    WalkBuilder::new(path)
        .standard_filters(true)
        .build()
        .filter_map(Result::ok)
        .filter_map(|path| path.metadata().ok())
        .map(|metadata| metadata.modified().ok())
        .filter_map(|modified_time| modified_time)
        .for_each(|modified_time| {
            if modified_time > modified {
                modified = modified_time;
            }
        });

    Ok(modified)
}

pub(crate) fn read_projects(config: &Config) -> Result<Vec<Project>> {
    let projects_dirs = config
        .project_dirs
        .iter()
        .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()));

    let project_dir_ents: Vec<DirEntry> = projects_dirs
        .flat_map(|d| d.read_dir().unwrap())
        .collect::<Result<Vec<_>, _>>()?;

    let projects_iter = project_dir_ents
        .par_iter()
        .map(|entry| entry.path())
        .filter(|p| p.is_dir())
        .map(|path| Project::from_path(&config, path));

    let projects = projects_iter.collect::<Result<Vec<_>>>()?;

    Ok(projects)
}
