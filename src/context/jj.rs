use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Repo {
    pub workdir: PathBuf,
}

pub fn init_repo(cwd: &Path) -> Option<Repo> {
    fn ok<T, E: std::fmt::Display>(r: Result<T, E>) -> Option<T> {
        r.inspect_err(|e| log::warn!("while loading jj repo: {e}"))
            .ok()
    }

    let workspace_dir = cwd.ancestors().find(|path| path.join(".jj").is_dir())?;

    Some(Repo {
        workdir: workspace_dir.into(),
    })
}

pub trait OrLog {
    type Output;
    fn or_log(self, module: &str) -> Self::Output;
}

impl<T, E: std::fmt::Display> OrLog for Result<T, E> {
    type Output = Option<T>;

    fn or_log(self, module: &str) -> Self::Output {
        self.inspect_err(|e| log::warn!("in {module}: {e}")).ok()
    }
}
