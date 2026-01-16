use crate::context::Context;
use std::time::Duration;
use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::fmt::Debug;
use crate::utils::{CommandOutput, create_command, exec_timeout};
use std::collections::HashMap;
use serde::Deserialize;

pub struct Repo {
    pub workdir: PathBuf,
    pub jj_closest_bookmarks: Option<JjClosestBookmarksInfo>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct JjClosestBookmarksInfo {
    pub bookmarks: Vec<BookmarkInfo>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BookmarkInfo {
    pub name: String,
    pub remote_ahead: usize,
    pub remote_behind: usize,
    pub is_tracked: bool,
}

#[derive(Debug, Deserialize)]
struct TrackedBookmarkOutput {
    name: String,
    ahead: usize,
    behind: usize,
}

const CLOSEST_BOOKMARKS_TEMPLATE: &str =  r#"bookmarks.map(|b| b.normal_target().change_id() ++ "\x1f")"#;

fn jujutsu_closest_template() -> String {
    format!(
        r#"local_bookmarks.map(|b| b.name()).join("\x1e") ++ "\n"
        ++ remote_bookmarks.filter(|b| b.tracked()).map(|b| b.name() ++ "\x1f" ++ b.tracking_ahead_count().lower() ++ "\x1f" ++ b.tracking_behind_count().lower()).join("\x1e") ++ "\n"
        ++ (local_bookmarks.any(|b| b.conflict()) || remote_bookmarks.any(|b| b.conflict()))"#,
    )
}

fn parse_tracked_bookmarks(output: &str) -> HashMap<String, (usize, usize)> {
    output
        .lines()
        .filter(|entry| !entry.trim().is_empty())
        .filter_map(|entry| serde_json::from_str::<TrackedBookmarkOutput>(entry).ok())
        .map(|entry| (entry.name, (entry.ahead, entry.behind)))
        .fold(HashMap::new(), |mut map, (name, counts)| {
            map.entry(name)
                .and_modify(|existing| {
                    existing.0 = existing.0.max(counts.0);
                    existing.1 = existing.1.max(counts.1);
                })
                .or_insert(counts);
            map
        })
}
pub(crate) fn get_closest_jujutsu_bookmarks_info(ctx: &Context, ignore_working_copy: &bool) -> Option<JjClosestBookmarksInfo> {

    let closest_bookmarks_output = ctx
        .exec_cmd(
            "jj",
            &[
                "log",
                "--no-graph",
                "-r",
                "heads(::@ & bookmarks())",
                if *ignore_working_copy {
                    "--ignore-working-copy"
                } else {
                    ""
                },
                "-T",
                CLOSEST_BOOKMARKS_TEMPLATE,
            ],
        )?
        .stdout;

    let change_id_closest = closest_bookmarks_output.split("\x1f").next().unwrap_or("");
    let mut closest_bookmarks = Vec::new();

    if !change_id_closest.is_empty() {
        let output = ctx
            .exec_cmd(
                "jj",
                &[
                    "log",
                    "--no-graph",
                    "-r",
                    change_id_closest,
                    if *ignore_working_copy {
                        "--ignore-working-copy"
                    } else {
                        ""
                    },
                    "-T",
                    &jujutsu_closest_template(),
                ],
            )?
            .stdout;

        let mut lines = output.lines();

        let bookmarks_str = lines.next().unwrap_or("");
        let tracked_bookmarks_str = lines.next().unwrap_or("");

        let tracked_bookmarks = parse_tracked_bookmarks(tracked_bookmarks_str);

        closest_bookmarks = if bookmarks_str.is_empty() {
            Vec::new()
        } else {
            bookmarks_str
                .split('\x1e')
                .filter(|entry| !entry.is_empty())
                .map(|name| {
                    let (ahead, behind, is_tracked) = tracked_bookmarks
                        .get(name)
                        .map(|(ahead, behind)| (*ahead, *behind, true))
                        .unwrap_or((0, 0, false));
                    BookmarkInfo {
                        name: name.to_string(),
                        remote_ahead: ahead,
                        remote_behind: behind,
                        is_tracked,
                    }
                })
                .collect()
        };
    }

    Some(JjClosestBookmarksInfo {
        bookmarks: closest_bookmarks,
    })
}

pub fn init_repo(context: &Context, cwd: &Path) -> Option<Repo> {
    fn ok<T, E: std::fmt::Display>(r: Result<T, E>) -> Option<T> {
        r.inspect_err(|e| log::warn!("while loading jj repo: {e}"))
            .ok()
    }

    let workspace_dir = cwd.ancestors().find(|path| path.join(".jj").is_dir())?;

    let jjbmk = get_closest_jujutsu_bookmarks_info(context, &true);

    Some(Repo {
        workdir: workspace_dir.into(),
        jj_closest_bookmarks: jjbmk,
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

impl Repo {


    /// Wrapper to execute external jj commands
    /// At this time, mocking is not supported.
    pub fn exec_jj<T: AsRef<OsStr> + Debug>(
        &self,
        context: &Context,
        jj_args: impl IntoIterator<Item = T>,
    ) -> Option<CommandOutput> {
        let mut command = create_command("jj").ok()?;


        //command.env("GIT_OPTIONAL_LOCKS", "0").args([
        //    OsStr::new("-C"),
        //    context.current_dir.as_os_str(),
        //    OsStr::new("--git-dir"),
        //    self.path.as_os_str(),
        //    OsStr::new("-c"),
        //    OsStr::new(fsm_config_value),
        //]);

        command.args(jj_args);
        log::trace!("Executing git command: {command:?}");

        exec_timeout(
            &mut command,
            Duration::from_millis(context.root_config.command_timeout),
        )
    }
}
