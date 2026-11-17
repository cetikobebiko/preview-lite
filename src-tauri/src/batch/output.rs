//! Batch output names and the conflict policy.
//!
//! Names come from `output_names::output_name_stem` of the input file name.
//! PDF outputs always carry a suffix, so even in the input's own folder the
//! plain name never equals the input. A target that is any batch input, or a
//! file this batch already published, is never replaced: such a target is
//! numbered even under the overwrite policy.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Numbered names go up to `name (999)`.
const MAX_NUMBER: u32 = 999;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Pick the first free `name (n)`.
    #[default]
    Number,
    /// Leave the file out when a target exists.
    Skip,
    /// Replace existing regular files (never an input or this batch's own output).
    Overwrite,
}

/// The names one input publishes, before a number is added.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NamePlan {
    /// One file: `{base}.{extension}`, numbered as `{base} (n).{extension}`.
    Single { base: String, extension: String },
    /// One image per page: `{stem}_sayfa_{p}.{extension}`, numbered as a set
    /// (`{stem} (n)_sayfa_{p}`) so the pages of one input stay together.
    Pages {
        stem: String,
        pages: Vec<u32>,
        extension: String,
    },
}

impl NamePlan {
    pub(crate) fn names(&self, number: Option<u32>) -> Vec<String> {
        match self {
            Self::Single { base, extension } => vec![match number {
                Some(number) => format!("{base} ({number}).{extension}"),
                None => format!("{base}.{extension}"),
            }],
            Self::Pages {
                stem,
                pages,
                extension,
            } => {
                let stem = match number {
                    Some(number) => format!("{stem} ({number})"),
                    None => stem.clone(),
                };
                pages
                    .iter()
                    .map(|&page| {
                        crate::output_names::page_image_output_name(&stem, page, extension)
                    })
                    .collect()
            }
        }
    }

    /// The stem the names are built from (page sets only need it to plan worker names).
    pub(crate) fn stem_for(&self, number: Option<u32>) -> String {
        let stem = match self {
            Self::Single { base, .. } => base,
            Self::Pages { stem, .. } => stem,
        };
        match number {
            Some(number) => format!("{stem} ({number})"),
            None => stem.clone(),
        }
    }
}

/// Targets an input may not replace: every batch input (by file identity) and
/// every path this batch has published.
#[derive(Debug, Default)]
pub(crate) struct ProtectedTargets {
    input_identities: HashSet<String>,
    published: HashSet<PathBuf>,
}

impl ProtectedTargets {
    pub(crate) fn new(input_identities: impl IntoIterator<Item = String>) -> Self {
        Self {
            input_identities: input_identities.into_iter().collect(),
            published: HashSet::new(),
        }
    }

    pub(crate) fn record_published(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.published.extend(paths);
    }

    fn protects(&self, target: &Path) -> bool {
        if self.published.contains(target) {
            return true;
        }
        crate::document_session::source_identity(target)
            .is_ok_and(|identity| self.input_identities.contains(&identity))
    }
}

/// Where one input's outputs go.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Resolution {
    Publish {
        number: Option<u32>,
        names: Vec<String>,
        overwrite: bool,
    },
    /// Skip policy and a target already exists.
    Skip,
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn first_free(plan: &NamePlan, output_dir: &Path, start: Option<u32>) -> Option<Resolution> {
    let numbers = std::iter::once(None)
        .chain((2..=MAX_NUMBER).map(Some))
        .skip_while(|number| start.is_some() && *number < start);
    for number in numbers {
        let names = plan.names(number);
        if names.iter().all(|name| !exists(&output_dir.join(name))) {
            return Some(Resolution::Publish {
                number,
                names,
                overwrite: false,
            });
        }
    }
    None
}

/// Picks the names for `plan` under `policy`. `after` restarts numbering past
/// a number that lost a race to another writer.
pub(crate) fn resolve(
    plan: &NamePlan,
    output_dir: &Path,
    policy: ConflictPolicy,
    protected: &ProtectedTargets,
    after: Option<u32>,
) -> Option<Resolution> {
    let start = after.map(|number| number.max(1) + 1);
    match policy {
        ConflictPolicy::Number => first_free(plan, output_dir, start),
        ConflictPolicy::Skip => {
            let names = plan.names(None);
            if names.iter().any(|name| exists(&output_dir.join(name))) {
                Some(Resolution::Skip)
            } else {
                Some(Resolution::Publish {
                    number: None,
                    names,
                    overwrite: false,
                })
            }
        }
        ConflictPolicy::Overwrite => {
            let names = plan.names(None);
            if after.is_none()
                && !names
                    .iter()
                    .any(|name| protected.protects(&output_dir.join(name)))
            {
                Some(Resolution::Publish {
                    number: None,
                    names,
                    overwrite: true,
                })
            } else {
                first_free(plan, output_dir, start)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(base: &str) -> NamePlan {
        NamePlan::Single {
            base: base.to_string(),
            extension: "pdf".to_string(),
        }
    }

    fn touch(path: &Path) {
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn single_and_page_names_number_where_the_user_expects() {
        assert_eq!(
            single("rapor_küçültülmüş").names(None),
            ["rapor_küçültülmüş.pdf"]
        );
        assert_eq!(
            single("rapor_küçültülmüş").names(Some(2)),
            ["rapor_küçültülmüş (2).pdf"]
        );
        let pages = NamePlan::Pages {
            stem: "rapor".into(),
            pages: vec![0, 2],
            extension: "png".into(),
        };
        assert_eq!(
            pages.names(None),
            ["rapor_sayfa_1.png", "rapor_sayfa_3.png"]
        );
        assert_eq!(
            pages.names(Some(3)),
            ["rapor (3)_sayfa_1.png", "rapor (3)_sayfa_3.png"]
        );
    }

    #[test]
    fn number_policy_takes_the_first_free_name() {
        let directory = tempfile::tempdir().unwrap();
        touch(&directory.path().join("a.pdf"));
        touch(&directory.path().join("a (2).pdf"));
        let resolution = resolve(
            &single("a"),
            directory.path(),
            ConflictPolicy::Number,
            &ProtectedTargets::default(),
            None,
        );
        assert_eq!(
            resolution,
            Some(Resolution::Publish {
                number: Some(3),
                names: vec!["a (3).pdf".into()],
                overwrite: false
            })
        );
        // A lost race restarts after the number that was taken.
        let retry = resolve(
            &single("a"),
            directory.path(),
            ConflictPolicy::Number,
            &ProtectedTargets::default(),
            Some(3),
        );
        assert!(matches!(
            retry,
            Some(Resolution::Publish {
                number: Some(4),
                ..
            })
        ));
    }

    #[test]
    fn page_sets_are_numbered_together() {
        let directory = tempfile::tempdir().unwrap();
        touch(&directory.path().join("r_sayfa_2.png"));
        let plan = NamePlan::Pages {
            stem: "r".into(),
            pages: vec![0, 1],
            extension: "png".into(),
        };
        let resolution = resolve(
            &plan,
            directory.path(),
            ConflictPolicy::Number,
            &ProtectedTargets::default(),
            None,
        );
        assert_eq!(
            resolution,
            Some(Resolution::Publish {
                number: Some(2),
                names: vec!["r (2)_sayfa_1.png".into(), "r (2)_sayfa_2.png".into()],
                overwrite: false
            })
        );
    }

    #[test]
    fn skip_policy_skips_when_any_target_exists() {
        let directory = tempfile::tempdir().unwrap();
        touch(&directory.path().join("a.pdf"));
        assert_eq!(
            resolve(
                &single("a"),
                directory.path(),
                ConflictPolicy::Skip,
                &ProtectedTargets::default(),
                None
            ),
            Some(Resolution::Skip)
        );
    }

    #[test]
    fn overwrite_replaces_an_unrelated_file() {
        let directory = tempfile::tempdir().unwrap();
        touch(&directory.path().join("a.pdf"));
        assert_eq!(
            resolve(
                &single("a"),
                directory.path(),
                ConflictPolicy::Overwrite,
                &ProtectedTargets::default(),
                None
            ),
            Some(Resolution::Publish {
                number: None,
                names: vec!["a.pdf".into()],
                overwrite: true
            })
        );
    }

    #[test]
    fn overwrite_never_replaces_an_input_or_this_batchs_output() {
        let directory = tempfile::tempdir().unwrap();
        // `rapor.pdf`'s output name is another input of the same batch.
        let other_input = directory.path().join("rapor_küçültülmüş.pdf");
        touch(&other_input);
        let identity = crate::document_session::source_identity(&other_input).unwrap();
        let protected = ProtectedTargets::new([identity]);
        let resolution = resolve(
            &single("rapor_küçültülmüş"),
            directory.path(),
            ConflictPolicy::Overwrite,
            &protected,
            None,
        );
        assert!(matches!(
            resolution,
            Some(Resolution::Publish {
                number: Some(2),
                overwrite: false,
                ..
            })
        ));

        let mut protected = ProtectedTargets::default();
        let published = directory.path().join("b.pdf");
        touch(&published);
        protected.record_published([published]);
        let resolution = resolve(
            &single("b"),
            directory.path(),
            ConflictPolicy::Overwrite,
            &protected,
            None,
        );
        assert!(matches!(
            resolution,
            Some(Resolution::Publish {
                number: Some(2),
                overwrite: false,
                ..
            })
        ));
    }

    #[test]
    fn numbering_gives_up_after_the_last_number() {
        let directory = tempfile::tempdir().unwrap();
        touch(&directory.path().join("a.pdf"));
        for number in 2..=MAX_NUMBER {
            touch(&directory.path().join(format!("a ({number}).pdf")));
        }
        assert_eq!(
            resolve(
                &single("a"),
                directory.path(),
                ConflictPolicy::Number,
                &ProtectedTargets::default(),
                None
            ),
            None
        );
    }
}
