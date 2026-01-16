use crate::context::Context;
use crate::utils::{CommandOutput, create_command, exec_timeout};
use gix::{
    Repository, ThreadSafeRepository,
    repository::Kind,
    sec::{self as git_sec, trust::DefaultForLevel},
    state as git_state,
};
use std::ffi::OsStr;
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Repo {
    pub repo: ThreadSafeRepository,

    /// If `current_dir` is a git repository or is contained within one,
    /// this is the short name of the current branch name of that repo,
    /// i.e. `main`.
    pub branch: Option<String>,

    /// If `current_dir` is a git repository or is contained within one,
    /// this is the path to the root of that repo.
    pub workdir: Option<PathBuf>,

    /// The path of the repository's `.git` directory.
    pub path: PathBuf,

    /// State
    pub state: Option<git_state::InProgress>,

    /// Remote repository
    pub remote: Option<Remote>,

    /// Contains `true` if the value of `core.fsmonitor` is set to `true`.
    /// If not `true`, `fsmonitor` is explicitly disabled in git commands.
    pub(crate) fs_monitor_value_is_true: bool,

    // Kind of repository, work tree or bare
    pub kind: Kind,
}

/// Remote repository
pub struct Remote {
    pub branch: Option<String>,
    pub name: Option<String>,
}

pub fn init_repo(current_dir: &Path) -> Result<Repo, Box<gix::discover::Error>> {
    // custom open options
    let mut git_open_opts_map = git_sec::trust::Mapping::<gix::open::Options>::default();

    // Load all the configuration as it affects aspects of the
    // `git_status` and `git_metrics` modules.
    let config = gix::open::permissions::Config {
        git_binary: true,
        system: true,
        git: true,
        user: true,
        env: true,
        includes: true,
    };
    // change options for config permissions without touching anything else
    git_open_opts_map.reduced = git_open_opts_map
        .reduced
        .permissions(gix::open::Permissions {
            config,
            ..gix::open::Permissions::default_for_level(git_sec::Trust::Reduced)
        });
    git_open_opts_map.full = git_open_opts_map.full.permissions(gix::open::Permissions {
        config,
        ..gix::open::Permissions::default_for_level(git_sec::Trust::Full)
    });

    let shared_repo = ThreadSafeRepository::discover_with_environment_overrides_opts(
        current_dir,
        gix::discover::upwards::Options {
            match_ceiling_dir_or_error: false,
            ..Default::default()
        },
        git_open_opts_map,
    )
        .inspect_err(|e| log::debug!("Failed to find git repo: {e}"))?;

    let repository = shared_repo.to_thread_local();
    log::trace!(
        "Found git repo: {repository:?}, (trust: {:?})",
        repository.git_dir_trust()
    );

    let branch = get_current_branch(&repository);
    let remote = get_remote_repository_info(&repository, branch.as_ref().map(AsRef::as_ref));
    let path = repository.path().to_path_buf();

    let fs_monitor_value_is_true = repository
        .config_snapshot()
        .boolean("core.fsmonitor")
        .unwrap_or(false);

    Ok(Repo {
        repo: shared_repo,
        branch: branch.map(|b| b.shorten().to_string()),
        workdir: repository.workdir().map(PathBuf::from),
        path,
        state: repository.state(),
        remote,
        fs_monitor_value_is_true,
        kind: repository.kind(),
    })
}

fn get_current_branch(repository: &Repository) -> Option<gix::refs::FullName> {
    repository.head_name().ok()?
}

fn get_remote_repository_info(
    repository: &Repository,
    branch_name: Option<&gix::refs::FullNameRef>,
) -> Option<Remote> {
    let branch_name = branch_name?;
    let branch = repository
        .branch_remote_ref_name(branch_name, gix::remote::Direction::Fetch)
        .and_then(std::result::Result::ok)
        .map(|r| r.shorten().to_string());
    let name = repository
        .branch_remote_name(branch_name.shorten(), gix::remote::Direction::Fetch)
        .map(|n| n.as_bstr().to_string());

    Some(Remote { branch, name })
}

impl Repo {
    /// Opens the associated git repository.
    pub fn open(&self) -> Repository {
        self.repo.to_thread_local()
    }

    /// Wrapper to execute external git commands.
    /// Handles adding the appropriate `--git-dir` and `--work-tree` flags to the command.
    /// Also handles additional features required for security, such as disabling `fsmonitor`.
    /// At this time, mocking is not supported.
    pub fn exec_git<T: AsRef<OsStr> + Debug>(
        &self,
        context: &Context,
        git_args: impl IntoIterator<Item = T>,
    ) -> Option<CommandOutput> {
        let mut command = create_command("git").ok()?;

        // A value of `true` should not execute external commands.
        let fsm_config_value = if self.fs_monitor_value_is_true {
            "core.fsmonitor=true"
        } else {
            "core.fsmonitor="
        };

        command.env("GIT_OPTIONAL_LOCKS", "0").args([
            OsStr::new("-C"),
            context.current_dir.as_os_str(),
            OsStr::new("--git-dir"),
            self.path.as_os_str(),
            OsStr::new("-c"),
            OsStr::new(fsm_config_value),
        ]);

        // Bare repositories might not have a workdir, so we need to check for that.
        if let Some(wt) = self.workdir.as_ref() {
            command.args([OsStr::new("--work-tree"), wt.as_os_str()]);
        }

        command.args(git_args);
        log::trace!("Executing git command: {command:?}");

        exec_timeout(
            &mut command,
            Duration::from_millis(context.root_config.command_timeout),
        )
    }
}