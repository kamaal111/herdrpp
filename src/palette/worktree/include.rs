//! Copy opt-in ignored files without following links or replacing checkout files.
use anyhow::{Context, Result, ensure};
use ignore::gitignore::GitignoreBuilder;
use std::{
    collections::HashSet,
    ffi::{CString, OsStr, OsString},
    fs::File,
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStringExt, fs::PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};

pub(super) struct Include {
    source: PathBuf,
    files: Vec<PathBuf>,
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    crate::command::run(
        Command::new("git").arg("-C").arg(root).args(args),
        Duration::from_secs(10),
    )
}

fn paths(bytes: Vec<u8>) -> Vec<PathBuf> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| PathBuf::from(OsString::from_vec(name.to_vec())))
        .collect()
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(name) if name != ".git"))
}

impl Include {
    pub(super) fn prepare(source: &Path) -> Result<Option<Self>> {
        let source_dir = open_dir(source)?;
        let mut file = match open_at(
            &source_dir,
            OsStr::new(".worktreeinclude"),
            libc::O_RDONLY | libc::O_NONBLOCK,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read .worktreeinclude"),
        };
        ensure!(
            file.metadata()?.is_file(),
            ".worktreeinclude is not a regular file"
        );
        let mut rules = String::new();
        file.read_to_string(&mut rules)
            .context("read .worktreeinclude")?;
        let mut builder = GitignoreBuilder::new(source);
        for line in rules.lines() {
            builder.add_line(Some(source.join(".worktreeinclude")), line)?;
        }
        let matcher = builder.build()?;
        if matcher.is_empty() {
            return Ok(None);
        }
        let files: Vec<_> = paths(git(
            source,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
            ],
        )?)
        .into_iter()
        .filter(|path| safe_relative(path))
        .filter(|path| matcher.matched_path_or_any_parents(path, false).is_ignore())
        .collect();
        if files.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            source: source.into(),
            files,
        }))
    }

    pub(super) fn copy(&self, destination: &Path) -> Vec<String> {
        match self.copy_checked(destination) {
            Ok(warnings) => warnings,
            Err(error) => vec![format!("{error:#}")],
        }
    }

    fn copy_checked(&self, destination: &Path) -> Result<Vec<String>> {
        let destination =
            std::fs::canonicalize(destination).context("resolve destination checkout")?;
        ensure!(
            destination != self.source,
            "destination is the invoking checkout"
        );
        ensure!(
            super::root(&destination, &Default::default())? == destination,
            "destination is not a checkout root"
        );
        ensure!(
            common_dir(&self.source)? == common_dir(&destination)?,
            "destination belongs to another repository"
        );
        let tracked: HashSet<_> = paths(git(&destination, &["ls-files", "--cached", "-z"])?)
            .into_iter()
            .collect();
        let source_dir = open_dir(&self.source)?;
        let destination_dir = open_dir(&destination)?;
        let mut warnings = vec![];
        for path in &self.files {
            if tracked.contains(path) {
                continue;
            }
            if let Err(error) = copy_file(&source_dir, &destination_dir, path) {
                warnings.push(format!("{}: {error:#}", path.display()));
            }
        }
        Ok(warnings)
    }
}

fn common_dir(root: &Path) -> Result<PathBuf> {
    let mut bytes = git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    std::fs::canonicalize(PathBuf::from(OsString::from_vec(bytes)))
        .context("resolve repository identity")
}

fn open_dir(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    File::options()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

fn name(name: &OsStr) -> io::Result<CString> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))
}

fn open_at(parent: &File, child: &OsStr, flags: i32) -> io::Result<File> {
    let child = name(child)?;
    // SAFETY: parent owns a live descriptor; child is a NUL-terminated name.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            child.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a new descriptor owned exclusively by this File.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn parent(root: &File, path: &Path, create: bool) -> Result<File> {
    let mut directory = root.try_clone()?;
    for component in path.parent().into_iter().flat_map(Path::components) {
        let Component::Normal(component) = component else {
            anyhow::bail!("invalid relative path");
        };
        if create {
            let component = name(component)?;
            // SAFETY: directory owns a live descriptor; component is a C string.
            let status = unsafe { libc::mkdirat(directory.as_raw_fd(), component.as_ptr(), 0o755) };
            if status < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error()).context("create destination directory");
            }
        }
        directory = open_at(&directory, component, libc::O_RDONLY | libc::O_DIRECTORY)
            .context("open directory without following symlinks")?;
        match open_at(
            &directory,
            OsStr::new(".git"),
            libc::O_RDONLY | libc::O_NONBLOCK,
        ) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            _ => anyhow::bail!("nested repository is excluded"),
        }
    }
    Ok(directory)
}

fn copy_file(source: &File, destination: &File, path: &Path) -> Result<()> {
    ensure!(safe_relative(path), "invalid relative path");
    let filename = path.file_name().context("missing filename")?;
    let source_parent = parent(source, path, false)?;
    let mut input = open_at(&source_parent, filename, libc::O_RDONLY | libc::O_NONBLOCK)
        .context("open regular source file without following symlinks")?;
    let metadata = input.metadata()?;
    ensure!(metadata.is_file(), "source is not a regular file");
    let destination_parent = parent(destination, path, true)?;
    write_file(
        &destination_parent,
        filename,
        &mut input,
        metadata.permissions(),
    )
}

fn write_file(
    destination_parent: &File,
    filename: &OsStr,
    input: &mut impl Read,
    permissions: std::fs::Permissions,
) -> Result<()> {
    let mut output = match open_at(
        destination_parent,
        filename,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
    ) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error).context("create destination file"),
    };
    let copied = io::copy(input, &mut output).and_then(|_| {
        output.set_permissions(std::fs::Permissions::from_mode(permissions.mode() & 0o777))
    });
    if let Err(error) = copied {
        let filename = name(filename)?;
        // SAFETY: destination_parent is live; unlinkat removes only this entry.
        let removed =
            unsafe { libc::unlinkat(destination_parent.as_raw_fd(), filename.as_ptr(), 0) };
        ensure!(
            removed == 0,
            "copy failed: {error}; incomplete destination cleanup failed: {}",
            io::Error::last_os_error()
        );
        return Err(error).context("copy file");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{ffi::OsStrExt, fs::symlink};

    struct Project {
        directory: tempfile::TempDir,
        source: PathBuf,
        destination: PathBuf,
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(root: &Path, path: impl AsRef<Path>, content: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn fifo(path: &Path) {
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: path is a valid C string naming a temporary test path.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    }

    struct ReadFailure;

    impl Read for ReadFailure {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("injected read failure"))
        }
    }

    impl Project {
        fn new(rules: &str, ignored: &str) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("source");
            std::fs::create_dir(&source).unwrap();
            let source = source.canonicalize().unwrap();
            run_git(&source, &["init", "-q", "-b", "main"]);
            write(&source, ".gitignore", ignored);
            write(&source, ".worktreeinclude", rules);
            write(&source, "tracked.txt", "tracked");
            run_git(&source, &["add", "."]);
            run_git(&source, &["add", "-f", "tracked.txt"]);
            run_git(
                &source,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "commit",
                    "-qm",
                    "initial",
                ],
            );
            let destination = directory.path().join("destination");
            run_git(
                &source,
                &[
                    "worktree",
                    "add",
                    "-qb",
                    "feature",
                    destination.to_str().unwrap(),
                ],
            );
            Self {
                directory,
                source,
                destination,
            }
        }

        fn copy(&self) -> Vec<String> {
            Include::prepare(&self.source)
                .unwrap()
                .unwrap()
                .copy(&self.destination)
        }
    }

    #[test]
    fn copies_only_ignored_matches_with_negation_and_preserves_permissions() {
        let project = Project::new(
            "# local files\n.env*\n!.env.production\nnotes.txt\ntracked.txt\n",
            ".env*\ntracked.txt\n",
        );
        write(&project.source, ".env.local", "MODE=local");
        write(&project.source, ".env.production", "MODE=production");
        write(&project.source, "notes.txt", "untracked");
        write(&project.source, "tracked.txt", "changed");
        std::fs::set_permissions(
            project.source.join(".env.local"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        assert!(project.copy().is_empty());
        assert_eq!(
            std::fs::read_to_string(project.destination.join(".env.local")).unwrap(),
            "MODE=local"
        );
        assert_eq!(
            std::fs::metadata(project.destination.join(".env.local"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        assert!(!project.destination.join(".env.production").exists());
        assert!(!project.destination.join("notes.txt").exists());
        assert_eq!(
            std::fs::read_to_string(project.destination.join("tracked.txt")).unwrap(),
            "tracked"
        );
    }

    #[test]
    fn matches_directories_anchors_escaped_comments_and_recursive_patterns() {
        let project = Project::new(
            "/root.env\nsecrets/\n!secrets/private/\n**/config.json\n\\#local\n",
            "*.env\nsecrets/\nvendor/\n# comment\n\\#local\n",
        );
        write(&project.source, "root.env", "root");
        write(&project.source, "nested/root.env", "nested");
        write(&project.source, "secrets/local/key", "secret");
        write(&project.source, "secrets/private/key", "excluded");
        write(&project.source, "vendor/deep/config.json", "config");
        write(&project.source, "#local", "escaped");
        assert!(project.copy().is_empty());
        assert_eq!(
            std::fs::read_to_string(project.destination.join("root.env")).unwrap(),
            "root"
        );
        assert!(!project.destination.join("nested/root.env").exists());
        assert_eq!(
            std::fs::read_to_string(project.destination.join("secrets/local/key")).unwrap(),
            "secret"
        );
        assert!(!project.destination.join("secrets/private/key").exists());
        assert_eq!(
            std::fs::read_to_string(project.destination.join("vendor/deep/config.json")).unwrap(),
            "config"
        );
        assert_eq!(
            std::fs::read_to_string(project.destination.join("#local")).unwrap(),
            "escaped"
        );
    }

    #[test]
    fn preserves_existing_files_and_dangling_symlinks() {
        let project = Project::new(".env*\n", ".env*\n");
        write(&project.source, ".env", "source");
        write(&project.source, ".env.local", "local");
        write(&project.destination, ".env", "target");
        symlink("missing", project.destination.join(".env.local")).unwrap();
        assert!(project.copy().is_empty());
        assert_eq!(
            std::fs::read_to_string(project.destination.join(".env")).unwrap(),
            "target"
        );
        assert_eq!(
            std::fs::read_link(project.destination.join(".env.local")).unwrap(),
            Path::new("missing")
        );
    }

    #[test]
    fn excludes_destination_tracked_files_even_when_deleted() {
        let project = Project::new(".env\n", ".env\n");
        write(&project.destination, ".env", "tracked destination");
        run_git(&project.destination, &["add", "-f", ".env"]);
        std::fs::remove_file(project.destination.join(".env")).unwrap();
        write(&project.source, ".env", "source");
        assert!(project.copy().is_empty());
        assert!(!project.destination.join(".env").exists());
    }

    #[test]
    fn copies_spaces_unicode_and_newlines_without_splitting_filenames() {
        let project = Project::new("local/\n", "local/\n");
        let path = Path::new("local/space newline\n雪");
        write(&project.source, path, "content");
        assert!(project.copy().is_empty());
        assert_eq!(
            std::fs::read_to_string(project.destination.join(path)).unwrap(),
            "content"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn copies_non_utf8_filenames_without_loss() {
        let project = Project::new("local/\n", "local/\n");
        let path = PathBuf::from(OsString::from_vec(b"local/space newline\n\xff".to_vec()));
        write(&project.source, &path, "content");
        assert!(project.copy().is_empty());
        assert_eq!(
            std::fs::read_to_string(project.destination.join(path)).unwrap(),
            "content"
        );
    }

    #[test]
    fn missing_empty_and_comment_only_rules_are_noops() {
        let project = Project::new("", ".env\n");
        assert!(Include::prepare(&project.source).unwrap().is_none());
        write(&project.source, ".worktreeinclude", "# comment\n\n");
        assert!(Include::prepare(&project.source).unwrap().is_none());
        std::fs::remove_file(project.source.join(".worktreeinclude")).unwrap();
        assert!(Include::prepare(&project.source).unwrap().is_none());
    }

    #[test]
    fn rejects_invalid_rules_and_symlinked_rule_file() {
        let project = Project::new("[z-a]\n", ".env\n");
        assert!(Include::prepare(&project.source).is_err());
        std::fs::remove_file(project.source.join(".worktreeinclude")).unwrap();
        symlink("tracked.txt", project.source.join(".worktreeinclude")).unwrap();
        assert!(Include::prepare(&project.source).is_err());
    }

    #[test]
    fn skips_source_symlinks_and_continues_copying_regular_files() {
        let project = Project::new(".env*\n", ".env*\n");
        write(&project.source, ".env.local", "local");
        symlink("tracked.txt", project.source.join(".env")).unwrap();
        assert_eq!(project.copy().len(), 1);
        assert!(!project.destination.join(".env").exists());
        assert_eq!(
            std::fs::read_to_string(project.destination.join(".env.local")).unwrap(),
            "local"
        );
    }

    #[test]
    fn rejects_source_parent_symlink_swapped_after_selection() {
        let project = Project::new("local/\n", "local/\n");
        write(&project.source, "local/key", "source");
        let include = Include::prepare(&project.source).unwrap().unwrap();
        std::fs::rename(project.source.join("local"), project.source.join("moved")).unwrap();
        symlink("moved", project.source.join("local")).unwrap();
        assert_eq!(include.copy(&project.destination).len(), 1);
        assert!(!project.destination.join("local/key").exists());
    }

    #[test]
    fn rejects_destination_parent_symlinks_without_writing_outside_checkout() {
        let project = Project::new("local/\n", "local/\n");
        write(&project.source, "local/key", "source");
        let outside = project.directory.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, project.destination.join("local")).unwrap();
        assert_eq!(project.copy().len(), 1);
        assert!(!outside.join("key").exists());
    }

    #[test]
    fn reports_disappeared_sources_and_keeps_other_copies() {
        let project = Project::new(".env*\n", ".env*\n");
        write(&project.source, ".env", "source");
        write(&project.source, ".env.local", "local");
        let include = Include::prepare(&project.source).unwrap().unwrap();
        std::fs::remove_file(project.source.join(".env")).unwrap();
        assert_eq!(include.copy(&project.destination).len(), 1);
        assert_eq!(
            std::fs::read_to_string(project.destination.join(".env.local")).unwrap(),
            "local"
        );
    }

    #[test]
    fn excludes_nested_repository_contents_and_git_metadata() {
        let project = Project::new("**\n", "nested/\n");
        write(&project.source, "nested/.git/config", "metadata");
        write(&project.source, "nested/key", "secret");
        assert!(!project.copy().is_empty());
        assert!(!project.destination.join("nested/key").exists());
        assert!(!safe_relative(Path::new(".git/config")));
        assert!(!safe_relative(Path::new("local/.git/config")));
        assert!(!safe_relative(Path::new("../outside")));
        assert!(!safe_relative(Path::new("/outside")));
    }

    #[test]
    fn refuses_same_checkout_and_other_repositories() {
        let project = Project::new(".env\n", ".env\n");
        write(&project.source, ".env", "source");
        let other = Project::new("", "");
        let include = Include::prepare(&project.source).unwrap().unwrap();
        assert_eq!(include.copy(&project.source).len(), 1);
        assert_eq!(include.copy(&other.destination).len(), 1);
        assert!(!other.destination.join(".env").exists());
    }

    #[test]
    fn skips_special_files_without_blocking() {
        let project = Project::new("local/\n", "local/\n");
        write(&project.source, "local/pipe", "regular");
        let include = Include::prepare(&project.source).unwrap().unwrap();
        std::fs::remove_file(project.source.join("local/pipe")).unwrap();
        fifo(&project.source.join("local/pipe"));
        assert_eq!(include.copy(&project.destination).len(), 1);
        assert!(!project.destination.join("local/pipe").exists());
    }

    #[test]
    fn removes_incomplete_destination_after_a_partial_read_failure() {
        let directory = tempfile::tempdir().unwrap();
        let destination = open_dir(directory.path()).unwrap();
        let mut input = io::Cursor::new(b"partial contents").chain(ReadFailure);
        let result = write_file(
            &destination,
            OsStr::new(".env"),
            &mut input,
            std::fs::Permissions::from_mode(0o600),
        );
        assert!(result.is_err());
        assert!(!directory.path().join(".env").exists());
    }
}
