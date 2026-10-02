//! A git tree as staged mode reads it: the index or `HEAD`, never the working
//! tree, so a partially staged file is checked as it will be committed.

use crate::git;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub struct Tree<'a> {
    root: &'a Path,
    /// Each regular file's path, with its blob.
    blobs: BTreeMap<String, git::Blob>,
}

impl<'a> Tree<'a> {
    pub fn index(root: &'a Path) -> Result<Self> {
        Ok(Self {
            root,
            blobs: git::blobs(root, false)?,
        })
    }

    pub fn head(root: &'a Path) -> Result<Self> {
        Ok(Self {
            root,
            blobs: git::blobs(root, true)?,
        })
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.blobs.keys().map(String::as_str)
    }

    /// The paths this tree adds, or holds with other contents or mode than
    /// `base`.
    pub fn changed_from(&self, base: &Tree) -> BTreeSet<String> {
        self.blobs
            .iter()
            .filter(|(path, blob)| base.blobs.get(*path) != Some(*blob))
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// One file's text, if the tree holds it.
    pub fn text(&self, path: &str) -> Result<Option<String>> {
        Ok(self.texts([path])?.pop().map(|(_, text)| text))
    }

    /// The text of each of `paths` the tree holds, in order.
    pub fn texts<'p>(
        &self,
        paths: impl IntoIterator<Item = &'p str>,
    ) -> Result<Vec<(String, String)>> {
        let held: Vec<(&str, &str)> = paths
            .into_iter()
            .filter_map(|path| Some((path, self.blobs.get(path)?.id.as_str())))
            .collect();
        let ids: Vec<&str> = held.iter().map(|(_, id)| *id).collect();
        let contents = git::read_blobs(self.root, &ids)?;
        Ok(held
            .into_iter()
            .zip(contents)
            .map(|((path, _), bytes)| {
                (
                    path.to_owned(),
                    String::from_utf8_lossy(&bytes).into_owned(),
                )
            })
            .collect())
    }
}

/// The files the index renames from `HEAD`, old path to new, and those it
/// deletes.
pub fn moves(
    root: &Path,
    head: &Tree,
    index: &Tree,
) -> Result<(BTreeMap<String, String>, BTreeSet<String>)> {
    let renames = git::staged_renames(root)?;
    let deleted = head
        .paths()
        .filter(|path| !index.blobs.contains_key(*path) && !renames.contains_key(*path))
        .map(str::to_owned)
        .collect();
    Ok((renames, deleted))
}
