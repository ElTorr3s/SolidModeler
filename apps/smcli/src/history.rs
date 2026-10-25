//! Version commands, until the GUI has a version manager:
//!
//! `smcli commit <in.smp> <message>`         record the part as a new version
//! `smcli log <in.smp>`                       versions, branches and tags
//! `smcli branch <in.smp> <name> [at]`        create a branch (default: here)
//! `smcli tag <in.smp> <name> [at]`           name a version, e.g. a release
//! `smcli checkout <in.smp> <target>`         switch to a branch, tag or version
//! `smcli merge <in.smp> <target> [ours|theirs]`
//!                                            merge into the current branch;
//!                                            the side settles every conflict
//! `smcli set-distance <in.smp> <feature> <value>`
//!                                            edit an extrusion, to try merges

use crate::{printable, Result};
use sm_document::{ExtrudeExtent, FeatureKind};
use sm_format::SmpFile;
use sm_history::{History, MergeOutcome, Resolutions, Side};
use std::path::Path;

pub const USAGE: &str = "smcli commit <in.smp> <message> | log <in.smp> | \
branch <in.smp> <name> [at] | tag <in.smp> <name> [at] | checkout <in.smp> <target> | \
merge <in.smp> <target> [ours|theirs] | set-distance <in.smp> <feature> <value>";

/// Runs a version command, or returns `None` if `args` isn't one.
pub fn run(args: &[&str]) -> Option<Result<()>> {
    Some(match args {
        ["commit", file, message] => commit(file, message),
        ["log", file] => log(file),
        ["branch", file, name] => with_history(file, |h, _| Ok(h.create_branch(name, &head(h))?)),
        ["branch", file, name, at] => with_history(file, |h, _| Ok(h.create_branch(name, at)?)),
        ["tag", file, name] => with_history(file, |h, _| Ok(h.create_tag(name, &head(h))?)),
        ["tag", file, name, at] => with_history(file, |h, _| Ok(h.create_tag(name, at)?)),
        ["checkout", file, target] => checkout(file, target),
        ["merge", file, target] => merge(file, target, None),
        ["merge", file, target, "ours"] => merge(file, target, Some(Side::Ours)),
        ["merge", file, target, "theirs"] => merge(file, target, Some(Side::Theirs)),
        ["set-distance", file, feature, value] => set_distance(file, feature, value),
        _ => return None,
    })
}

fn head(h: &History) -> String {
    h.head_version().id.to_string()
}

fn save(path: &str, file: &SmpFile) -> Result<()> {
    // The cache is only kept if it still matches the document.
    sm_format::save_with_history(
        Path::new(path),
        &file.document,
        file.history.as_ref(),
        file.cached_step.as_deref(),
    )?;
    Ok(())
}

/// Loads a file that must already have a history, applies `f`, saves.
fn with_history(
    path: &str,
    f: impl FnOnce(&mut History, &mut SmpFile) -> Result<()>,
) -> Result<()> {
    let mut file = sm_format::open(Path::new(path))?;
    let mut history = file
        .history
        .take()
        .ok_or("this part has no versions yet; run `smcli commit` first")?;
    f(&mut history, &mut file)?;
    file.history = Some(history);
    save(path, &file)
}

fn commit(path: &str, message: &str) -> Result<()> {
    let mut file = sm_format::open(Path::new(path))?;
    let id = match &mut file.history {
        Some(h) => h.commit(&file.document, message, sm_history::now())?,
        None => {
            let h = History::new(&file.document, message, sm_history::now())?;
            let id = h.head_version().id.clone();
            file.history = Some(h);
            id
        }
    };
    save(path, &file)?;
    println!("recorded version {}", id.short());
    Ok(())
}

fn log(path: &str) -> Result<()> {
    let file = sm_format::open(Path::new(path))?;
    let Some(h) = &file.history else {
        println!("no versions yet");
        return Ok(());
    };
    match h.current_branch() {
        Some(b) => println!("on branch {}", printable(b)),
        None => println!("on version {}", h.head_version().id.short()),
    }
    if !h.is_clean(&file.document)? {
        println!("(the open part has changes not recorded as a version)");
    }
    let labels = |id| {
        let names: Vec<&str> = h
            .branches()
            .iter()
            .chain(h.tags())
            .filter(|(_, v)| *v == id)
            .map(|(n, _)| n.as_str())
            .collect();
        if names.is_empty() {
            String::new()
        } else {
            format!(" ({})", names.join(", "))
        }
    };
    for v in h.versions().iter().rev() {
        let parents: Vec<&str> = v.parents.iter().map(|p| p.short()).collect();
        println!(
            "{}{}  {}  [parents: {}]",
            v.id.short(),
            labels(&v.id),
            printable(&v.message),
            if parents.is_empty() {
                "none".into()
            } else {
                parents.join(", ")
            }
        );
        for change in h.changes(v)? {
            println!("    {}", printable(&change.to_string()));
        }
    }
    Ok(())
}

fn checkout(path: &str, target: &str) -> Result<()> {
    with_history(path, |h, file| {
        file.document = h.checkout(target, &file.document)?;
        file.cached_step = None;
        println!("checked out {}", printable(target));
        Ok(())
    })
}

fn merge(path: &str, target: &str, side: Option<Side>) -> Result<()> {
    with_history(path, |h, file| {
        let now = sm_history::now();
        let mut outcome = h.merge(target, &file.document, &Resolutions::new(), now)?;
        if let (MergeOutcome::Conflicted(result), Some(side)) = (&outcome, side) {
            let picks = result
                .conflicts
                .iter()
                .map(|c| (c.key.clone(), side))
                .collect();
            outcome = h.merge(target, &file.document, &picks, now)?;
        }
        match outcome {
            MergeOutcome::UpToDate => println!("already up to date"),
            MergeOutcome::FastForward(doc) => {
                file.document = doc;
                file.cached_step = None;
                println!("fast-forwarded to {}", printable(target));
            }
            MergeOutcome::Merged { version, document } => {
                file.document = document;
                file.cached_step = None;
                println!(
                    "merged {} as version {}",
                    printable(target),
                    version.short()
                );
            }
            MergeOutcome::Conflicted(result) => {
                // Conflict values are JSON, which escapes control characters.
                for c in &result.conflicts {
                    let show = |v: &Option<serde_json::Value>| {
                        v.as_ref().map_or("deleted".into(), |v| v.to_string())
                    };
                    println!(
                        "conflict on {}: ours {} / theirs {}",
                        c.key,
                        show(&c.ours),
                        show(&c.theirs)
                    );
                }
                if let Some(e) = &result.invalid {
                    println!(
                        "the merged part would be invalid: {}",
                        printable(&e.to_string())
                    );
                    return Err(
                        "merge not recorded; fix the part on one branch and merge again".into(),
                    );
                }
                return Err(
                    "merge not recorded; pass `ours` or `theirs` to settle conflicts".into(),
                );
            }
        }
        Ok(())
    })
}

fn set_distance(path: &str, feature: &str, value: &str) -> Result<()> {
    let distance: f64 = value.parse()?;
    let mut file = sm_format::open(Path::new(path))?;
    let f = file
        .document
        .features
        .iter_mut()
        .find(|f| f.name == feature)
        .ok_or_else(|| format!("no feature called {feature:?}"))?;
    let FeatureKind::Extrude(e) = &mut f.kind else {
        return Err(format!("{feature} is not an extrusion").into());
    };
    e.extent = match e.extent {
        ExtrudeExtent::Distance { .. } => ExtrudeExtent::Distance { distance },
        ExtrudeExtent::Symmetric { .. } => ExtrudeExtent::Symmetric { distance },
    };
    file.cached_step = None;
    save(path, &file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sm_document::{
        BodyOperation, Extrude, LengthUnit, OriginPlane, PartDocument, Profile, Sketch,
    };

    /// A scratch .smp holding a sketch and a 10 mm extrusion.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("smcli-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("part.smp");
            let mut doc = PartDocument::new("Part", LengthUnit::Mm);
            let sketch = doc.add_feature(
                "Sketch1",
                FeatureKind::Sketch(Sketch {
                    plane: OriginPlane::XY,
                    offset: 0.0,
                    profiles: vec![Profile {
                        points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                    }],
                }),
            );
            doc.add_feature(
                "Extrusion1",
                FeatureKind::Extrude(Extrude {
                    sketch,
                    extent: ExtrudeExtent::Distance { distance: 10.0 },
                    operation: BodyOperation::Join,
                }),
            );
            sm_format::save(&path, &doc, None).unwrap();
            Scratch(path)
        }

        fn run(&self, args: &[&str]) -> Result<()> {
            let path = self.0.to_str().unwrap();
            let mut full = vec![args[0], path];
            full.extend(&args[1..]);
            run(&full).expect("a version command")
        }

        fn distance(&self) -> f64 {
            let file = sm_format::open(&self.0).unwrap();
            match &file.document.features[1].kind {
                FeatureKind::Extrude(e) => match e.extent {
                    ExtrudeExtent::Distance { distance } => distance,
                    ExtrudeExtent::Symmetric { distance } => distance,
                },
                _ => unreachable!(),
            }
        }

        fn history(&self) -> History {
            sm_format::open(&self.0).unwrap().history.unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    #[test]
    fn unknown_commands_are_not_handled() {
        assert!(run(&["frobnicate", "x.smp"]).is_none());
        assert!(run(&["merge", "x.smp", "b", "mine"]).is_none());
    }

    #[test]
    fn branch_edit_and_merge() {
        let s = Scratch::new("flow");
        // Version commands other than commit need a history first.
        assert!(s.run(&["branch", "b"]).is_err());
        s.run(&["commit", "Initial"]).unwrap();
        assert!(s.run(&["commit", "again"]).is_err()); // nothing changed
        s.run(&["branch", "b"]).unwrap();
        s.run(&["tag", "Rev A"]).unwrap();

        s.run(&["checkout", "b"]).unwrap();
        s.run(&["set-distance", "Extrusion1", "25"]).unwrap();
        // Switching away would lose the edit.
        assert!(s.run(&["checkout", "main"]).is_err());
        s.run(&["commit", "Deeper"]).unwrap();
        s.run(&["checkout", "main"]).unwrap();
        assert_eq!(s.distance(), 10.0);

        s.run(&["merge", "b"]).unwrap(); // fast-forward
        assert_eq!(s.distance(), 25.0);
        s.run(&["log"]).unwrap();
        s.run(&["checkout", "Rev A"]).unwrap();
        assert_eq!(s.distance(), 10.0);
        assert_eq!(s.history().current_branch(), None);
    }

    #[test]
    fn conflicts_need_a_side() {
        let s = Scratch::new("conflict");
        s.run(&["commit", "Initial"]).unwrap();
        s.run(&["branch", "b"]).unwrap();
        s.run(&["set-distance", "Extrusion1", "20"]).unwrap();
        s.run(&["commit", "Twenty"]).unwrap();
        s.run(&["checkout", "b"]).unwrap();
        s.run(&["set-distance", "Extrusion1", "30"]).unwrap();
        s.run(&["commit", "Thirty"]).unwrap();
        s.run(&["checkout", "main"]).unwrap();

        let versions = s.history().versions().len();
        assert!(s.run(&["merge", "b"]).is_err());
        assert_eq!(s.history().versions().len(), versions);
        assert_eq!(s.distance(), 20.0);

        s.run(&["merge", "b", "theirs"]).unwrap();
        assert_eq!(s.distance(), 30.0);
        assert_eq!(s.history().head_version().parents.len(), 2);
    }

    #[test]
    fn set_distance_rejects_bad_input() {
        let s = Scratch::new("set");
        assert!(s.run(&["set-distance", "Nope", "1"]).is_err());
        assert!(s.run(&["set-distance", "Sketch1", "1"]).is_err());
        assert!(s.run(&["set-distance", "Extrusion1", "deep"]).is_err());
        // Saving re-validates, so a non-finite distance never reaches disk.
        assert!(s.run(&["set-distance", "Extrusion1", "inf"]).is_err());
        assert_eq!(s.distance(), 10.0);
    }
}
