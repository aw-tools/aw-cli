//! Workspace template materialisation.
//!
//! `aw init` clones the selected template at runtime. A remote template with no
//! explicit ref is seeded from its highest stable release tag, so a release
//! never needs a constant bumped here. Compatibility has no ceiling on purpose:
//! a template whose layout an older binary cannot use fails its contract check
//! loudly rather than being skipped.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::git;

pub const DEFAULT_URL: &str = "git@github.com:aw-tools/workspace.template.git";

/// Which commit of a template to seed from.
#[derive(Debug, Eq, PartialEq)]
pub enum Reference {
    /// The highest `vMAJOR.MINOR.PATCH` tag the template carries. Pre-release
    /// and build-suffixed tags never qualify.
    LatestStable,
    /// A ref the user named with `@`.
    Named(String),
    /// Whatever the checkout holds; local paths only.
    Head,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Source {
    pub url: String,
    pub reference: Reference,
}

impl Source {
    pub fn parse(spec: Option<&str>) -> Result<Self> {
        let spec = spec.unwrap_or(DEFAULT_URL);
        anyhow::ensure!(!spec.trim().is_empty(), "template source is empty");
        reject_http_credentials(spec)?;

        if Path::new(spec).exists() {
            return Ok(Self {
                url: spec.to_owned(),
                reference: Reference::Head,
            });
        }

        if let Some((url, reference)) = split_reference(spec) {
            anyhow::ensure!(!reference.is_empty(), "template ref is empty");
            return Ok(Self {
                url: url.to_owned(),
                reference: Reference::Named(reference.to_owned()),
            });
        }

        Ok(Self {
            url: spec.to_owned(),
            reference: Reference::LatestStable,
        })
    }

    /// A workspace that already records this template seeds from the ref it
    /// recorded, so a repeat `aw init` stays a no-op after a newer release.
    pub fn pinned_by(&mut self, recorded: Option<&crate::manifest::Template>) {
        if let Some(recorded) = recorded {
            if self.reference == Reference::LatestStable && recorded.url == self.url {
                self.reference = Reference::Named(recorded.reference.clone());
            }
        }
    }
}

fn reject_http_credentials(spec: &str) -> Result<()> {
    let Some((scheme, after_scheme)) = spec.split_once("://") else {
        return Ok(());
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Ok(());
    }
    anyhow::ensure!(
        !after_scheme.contains(['?', '#']),
        "HTTP(S) template URLs must not contain credentials, query strings, or fragments; \
         use a Git credential helper instead"
    );
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);

    anyhow::ensure!(
        authority
            .rsplit_once('@')
            .is_none_or(|(userinfo, _)| userinfo.is_empty()),
        "HTTP(S) template URLs must not contain credentials, query strings, or fragments; \
         use a Git credential helper instead"
    );
    Ok(())
}

pub struct Prepared {
    temporary: TemporaryDirectory,
    /// The ref the commit was resolved from: the discovered tag for
    /// `LatestStable`, the user's ref for `Named`, empty for `Head`. This is
    /// what the manifest records, never the word "latest".
    pub reference: String,
    pub sha: String,
}

/// Clone a template into an isolated temporary directory, resolve its requested
/// commit, scrub template-repository-only files, and validate its contract.
pub fn prepare(source: &Source) -> Result<Prepared> {
    let temporary = TemporaryDirectory::new()?;
    git::clone_template(&source.url, temporary.path())?;
    // The discovered tag resolves as `refs/tags/<tag>` so a branch sharing its
    // name can never make it ambiguous; the manifest records the bare tag.
    let (reference, resolve_as) = match &source.reference {
        Reference::LatestStable => {
            let tag = latest_stable_tag(&source.url, temporary.path())?;
            let full = format!("refs/tags/{tag}");
            (Some(tag), Some(full))
        }
        Reference::Named(reference) => (Some(reference.clone()), Some(reference.clone())),
        Reference::Head => (None, None),
    };
    let sha = git::resolve_commit(temporary.path(), resolve_as.as_deref())?;
    git::checkout_detached(temporary.path(), &sha)?;
    scrub_seedignored(temporary.path())?;
    validate_contract(temporary.path())?;
    Ok(Prepared {
        temporary,
        reference: reference.unwrap_or_default(),
        sha,
    })
}

/// The highest stable release tag in a clone. The clone carries every tag, so
/// this needs no second round trip to the remote.
fn latest_stable_tag(url: &str, dir: &Path) -> Result<String> {
    let tags = git::tags(dir)?;
    pick_latest_stable(tags.iter().map(String::as_str))
        .map(str::to_owned)
        .with_context(|| {
            format!(
                "the template has no stable release tag (vMAJOR.MINOR.PATCH); \
                 pin a ref with --template {url}@<ref>"
            )
        })
}

/// Pick the highest `vMAJOR.MINOR.PATCH` tag. Anything else — a pre-release
/// suffix, build metadata, a missing `v`, a leading zero — is not a stable
/// release and is skipped.
fn pick_latest_stable<'a>(tags: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    tags.filter_map(|tag| stable_version(tag).map(|version| (version, tag)))
        .max()
        .map(|(_, tag)| tag)
}

fn stable_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v')?.split('.');
    let major = plain_number(parts.next()?)?;
    let minor = plain_number(parts.next()?)?;
    let patch = plain_number(parts.next()?)?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// A decimal number with no sign, no leading zero and nothing else; too large
/// for `u64` is not a version either.
fn plain_number(part: &str) -> Option<u64> {
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if part.len() > 1 && part.starts_with('0') {
        return None;
    }
    part.parse().ok()
}

/// Merge a prepared template into `root` without replacing anything already
/// present.
pub fn materialise(root: &Path, prepared: &Prepared) -> Result<Vec<String>> {
    let mut created = Vec::new();
    copy_directory(prepared.temporary.path(), root, Path::new(""), &mut created)?;
    Ok(created)
}

fn split_reference(spec: &str) -> Option<(&str, &str)> {
    let delimiter = spec.rfind('@')?;
    let before = &spec[..delimiter];

    if Path::new(before).exists() {
        return Some((before, &spec[delimiter + 1..]));
    }

    if spec.contains("://") {
        return (delimiter > spec.rfind('/').unwrap_or(0))
            .then(|| (before, &spec[delimiter + 1..]));
    }

    if let Some(colon) = spec.rfind(':') {
        return (delimiter > colon).then(|| (before, &spec[delimiter + 1..]));
    }

    Some((before, &spec[delimiter + 1..]))
}

fn scrub_seedignored(root: &Path) -> Result<()> {
    let seedignore = root.join(".seedignore");
    if !seedignore.is_file() {
        return Ok(());
    }

    for relative in git::tracked_excluded(root, ".seedignore")? {
        let relative = Path::new(&relative);
        anyhow::ensure!(
            relative.is_relative()
                && !relative
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir)),
            "seedignore selected unsafe path {}",
            relative.display()
        );
        if relative == Path::new(".seedignore") {
            continue;
        }
        let path = root.join(relative);
        if path.is_symlink() || path.is_file() {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing seedignored {}", path.display()))?;
            remove_empty_parents(path.parent(), root)?;
        }
    }

    std::fs::remove_file(&seedignore).with_context(|| format!("removing {}", seedignore.display()))
}

fn remove_empty_parents(mut current: Option<&Path>, root: &Path) -> Result<()> {
    while let Some(path) = current {
        if path == root {
            break;
        }
        match std::fs::remove_dir(path) {
            Ok(()) => current = path.parent(),
            Err(err) if err.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
            Err(err) => {
                return Err(err).with_context(|| format!("removing empty {}", path.display()));
            }
        }
    }
    Ok(())
}

fn validate_contract(root: &Path) -> Result<()> {
    for required in [".gitignore", crate::manifest::FILENAME] {
        let path = root.join(required);
        let present =
            std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file());
        anyhow::ensure!(
            present,
            "template contract violation: missing regular file {required}"
        );
    }

    let manifest = crate::manifest::Manifest::load(root)?;
    anyhow::ensure!(
        manifest.workspace.name == crate::manifest::WORKSPACE_NAME_PLACEHOLDER,
        "template contract violation: [workspace] name must be {:?}",
        crate::manifest::WORKSPACE_NAME_PLACEHOLDER
    );
    let text = std::fs::read_to_string(root.join(crate::manifest::FILENAME))
        .context("reading template manifest")?;
    crate::manifest::replace_workspace_name(&text, crate::manifest::WORKSPACE_NAME_PLACEHOLDER)?;
    Ok(())
}

fn copy_directory(
    source: &Path,
    destination: &Path,
    relative: &Path,
    created: &mut Vec<String>,
) -> Result<()> {
    let mut entries = std::fs::read_dir(source)
        .with_context(|| format!("reading {}", source.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", source.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        if relative.as_os_str().is_empty() && entry.file_name() == ".git" {
            continue;
        }

        let child_relative = relative.join(entry.file_name());
        let target = destination.join(&child_relative);
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading type of {}", entry.path().display()))?;
        let target_type = match std::fs::symlink_metadata(&target) {
            Ok(metadata) => Some(metadata.file_type()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                return Err(err).with_context(|| format!("reading {}", target.display()));
            }
        };

        if file_type.is_dir() {
            match target_type {
                Some(existing) if existing.is_dir() => {}
                Some(_) => continue,
                None => {
                    std::fs::create_dir(&target)
                        .with_context(|| format!("creating {}", target.display()))?;
                }
            }
            copy_directory(&entry.path(), destination, &child_relative, created)?;
            continue;
        }

        if target_type.is_some() {
            continue;
        }

        if file_type.is_symlink() {
            let link = std::fs::read_link(entry.path())
                .with_context(|| format!("reading link {}", entry.path().display()))?;
            std::os::unix::fs::symlink(link, &target)
                .with_context(|| format!("linking {}", target.display()))?;
        } else {
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {}", target.display()))?;
        }
        created.push(child_relative.to_string_lossy().into_owned());
    }
    Ok(())
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Result<Self> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir();

        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("aw-template-{}-{id}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("creating temporary {}", path.display()));
                }
            }
        }
        anyhow::bail!("could not allocate a temporary directory for the template")
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_source_seeds_from_the_latest_stable_release() {
        assert_eq!(
            Source::parse(None).expect("default parses"),
            Source {
                url: "git@github.com:aw-tools/workspace.template.git".to_owned(),
                reference: Reference::LatestStable,
            }
        );
    }

    #[test]
    fn any_remote_url_without_a_ref_seeds_from_the_latest_stable_release() {
        for url in [
            "https://github.com/aw-tools/workspace.template",
            "https://github.com/aw-tools/workspace.template.git",
            "git@github.com:someone/else.git",
        ] {
            assert_eq!(
                Source::parse(Some(url))
                    .expect("remote URL parses")
                    .reference,
                Reference::LatestStable,
                "{url}"
            );
        }
    }

    #[test]
    fn a_local_path_keeps_its_checkout() {
        let local = TemporaryDirectory::new().expect("temporary directory");
        let spec = local.path().display().to_string();
        assert_eq!(
            Source::parse(Some(&spec))
                .expect("local path parses")
                .reference,
            Reference::Head
        );
    }

    #[test]
    fn a_recorded_template_pins_a_repeat_init() {
        let recorded = crate::manifest::Template {
            url: DEFAULT_URL.to_owned(),
            reference: "v0.1.0".to_owned(),
            sha: "0000000".to_owned(),
        };
        let mut same = Source::parse(None).expect("default parses");
        same.pinned_by(Some(&recorded));
        assert_eq!(same.reference, Reference::Named("v0.1.0".to_owned()));

        let mut other = Source::parse(Some("git@github.com:someone/else.git")).expect("parses");
        other.pinned_by(Some(&recorded));
        assert_eq!(other.reference, Reference::LatestStable);

        let mut explicit = Source::parse(Some(
            "git@github.com:aw-tools/workspace.template.git@v0.2.0",
        ))
        .expect("parses");
        explicit.pinned_by(Some(&recorded));
        assert_eq!(explicit.reference, Reference::Named("v0.2.0".to_owned()));
    }

    #[test]
    fn picks_the_highest_stable_tag_and_skips_everything_else() {
        let tags = [
            "v0.1.0",
            "v0.10.0",
            "v0.9.3",
            "v1.0.0-alpha.1",
            "v1.0.0-rc1",
            "v1.0.0-beta",
            "v0.10.1+build.7",
            "1.0.0",
            "v01.0.0",
            "v2",
            "v2.0",
            "v0.10.0.1",
            "release-3",
        ];
        assert_eq!(pick_latest_stable(tags.into_iter()), Some("v0.10.0"));
        assert_eq!(
            pick_latest_stable(["v1.0.0-rc1", "v2.0.0-beta"].into_iter()),
            None
        );
        assert_eq!(pick_latest_stable(std::iter::empty()), None);
        assert_eq!(
            pick_latest_stable(["v1.2.3", "v1.10.0", "v1.9.9"].into_iter()),
            Some("v1.10.0")
        );
    }

    #[test]
    fn prepare_seeds_from_the_latest_stable_tag_and_records_it() {
        let seed = tagged_template(&["v0.1.0", "v0.2.0-rc1", "v0.2.0", "v0.3.0-alpha"]);
        let source = Source {
            url: seed.path().display().to_string(),
            reference: Reference::LatestStable,
        };
        // A branch sharing the tag's name must not make the tag ambiguous.
        git::run(seed.path(), &["branch", "v0.2.0", "v0.1.0"]).expect("branch fixture");
        let prepared = prepare(&source).expect("a stable tag exists");
        assert_eq!(prepared.reference, "v0.2.0");
        let expected =
            git::run(seed.path(), &["rev-parse", "refs/tags/v0.2.0^{commit}"]).expect("rev-parse");
        assert_eq!(prepared.sha, expected.trim());
    }

    #[test]
    fn prepare_refuses_a_template_without_a_stable_tag() {
        let seed = tagged_template(&["v0.2.0-rc1"]);
        let source = Source {
            url: seed.path().display().to_string(),
            reference: Reference::LatestStable,
        };
        let Err(error) = prepare(&source) else {
            panic!("a template without a stable tag must be refused")
        };
        let message = format!("{error:#}");
        assert!(message.contains("no stable release tag"), "{message}");
        assert!(
            message.contains(&format!("--template {}@<ref>", seed.path().display())),
            "{message}"
        );
    }

    /// A local template repository with a contract-valid commit per tag, so
    /// `prepare` can clone it without network access.
    fn tagged_template(tags: &[&str]) -> TemporaryDirectory {
        let seed = TemporaryDirectory::new().expect("temporary directory");
        let dir = seed.path();
        git::run(dir, &["init", "--quiet", "--initial-branch=main"]).expect("init fixture");
        std::fs::write(dir.join(".gitignore"), "*\n!.gitignore\n!workspace.toml\n")
            .expect("gitignore fixture");
        for tag in tags {
            std::fs::write(
                dir.join("workspace.toml"),
                format!("# {tag}\n[workspace]\nname = \"CHANGEME\"\n"),
            )
            .expect("manifest fixture");
            git::run(dir, &["add", "-A"]).expect("add fixture");
            git::run(
                dir,
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "commit",
                    "--quiet",
                    "-m",
                    tag,
                ],
            )
            .expect("commit fixture");
            git::run(dir, &["tag", tag]).expect("tag fixture");
        }
        seed
    }

    #[test]
    fn separates_refs_without_splitting_ssh_usernames() {
        assert_eq!(
            Source::parse(Some("git@github.com:aw-tools/template.git")).expect("SSH URL parses"),
            Source {
                url: "git@github.com:aw-tools/template.git".to_owned(),
                reference: Reference::LatestStable,
            }
        );
        assert_eq!(
            Source::parse(Some("git@github.com:aw-tools/template.git@release/v1"))
                .expect("SSH URL and ref parse"),
            Source {
                url: "git@github.com:aw-tools/template.git".to_owned(),
                reference: Reference::Named("release/v1".to_owned()),
            }
        );
    }

    #[test]
    fn rejects_http_credentials() {
        for source in [
            "https://user@example.com/org/template.git",
            "https://user:password@example.com/org/template.git",
            "http://ghp_token@example.com/org/template.git",
            "HTTPS://user@example.com/org/template.git",
            "https://example.com/org/template.git?access_token=secret",
            "https://example.com/org/template.git#token=secret",
        ] {
            let error = Source::parse(Some(source)).expect_err("credentials are rejected");
            assert_eq!(
                error.to_string(),
                "HTTP(S) template URLs must not contain credentials, query strings, or fragments; \
                 use a Git credential helper instead"
            );
        }
    }

    #[test]
    fn accepts_credential_free_https_and_separates_refs() {
        assert_eq!(
            Source::parse(Some("https://example.com/org/template.git"))
                .expect("credential-free HTTPS URL parses"),
            Source {
                url: "https://example.com/org/template.git".to_owned(),
                reference: Reference::LatestStable,
            }
        );
        assert_eq!(
            Source::parse(Some("https://example.com/org/template.git@v1"))
                .expect("HTTPS URL and ref parse"),
            Source {
                url: "https://example.com/org/template.git".to_owned(),
                reference: Reference::Named("v1".to_owned()),
            }
        );
    }

    #[test]
    fn contract_rejects_missing_or_linked_required_files() {
        let temporary = TemporaryDirectory::new().expect("temporary directory");
        std::fs::write(temporary.path().join("workspace.toml"), "").expect("manifest fixture");
        let missing = validate_contract(temporary.path()).expect_err("gitignore is required");
        assert!(missing.to_string().contains(".gitignore"));

        std::os::unix::fs::symlink("workspace.toml", temporary.path().join(".gitignore"))
            .expect("link fixture");
        let linked = validate_contract(temporary.path()).expect_err("link is not a regular file");
        assert!(linked.to_string().contains(".gitignore"));
    }

    #[test]
    fn contract_rejects_an_invalid_manifest() {
        let temporary = TemporaryDirectory::new().expect("temporary directory");
        std::fs::write(temporary.path().join(".gitignore"), "*").expect("gitignore fixture");
        std::fs::write(temporary.path().join("workspace.toml"), "[workspace")
            .expect("manifest fixture");

        let error = validate_contract(temporary.path()).expect_err("manifest must parse");

        assert!(error.to_string().contains("parsing manifest"), "{error:#}");
    }

    #[test]
    fn contract_requires_the_workspace_name_placeholder() {
        let temporary = TemporaryDirectory::new().expect("temporary directory");
        std::fs::write(temporary.path().join(".gitignore"), "*").expect("gitignore fixture");
        std::fs::write(
            temporary.path().join("workspace.toml"),
            "[workspace]\nname = \"already-set\"\n",
        )
        .expect("manifest fixture");

        let error = validate_contract(temporary.path()).expect_err("placeholder is required");

        assert!(error.to_string().contains("CHANGEME"), "{error:#}");
    }
}
