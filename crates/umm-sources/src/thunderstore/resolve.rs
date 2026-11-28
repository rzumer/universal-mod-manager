//! Which package versions to install for a request, dependencies first.
//!
//! Each dependency names a version (`Owner-Name-1.2.3`), the one its mod was built against.
//! As r2modman does, a dependency gets its newest version instead, unless an installed version
//! at least as new as the named one is kept (installing a mod does not update the others).
//! A modpack's dependencies, and theirs, get exactly the versions named, so everyone who
//! installs it gets the same set. When several packages need the same one, the highest
//! version wins, except for a package whose version is fixed (a profile pinned to it, or a
//! version asked for by name): it keeps that version, whatever the others name.

use crate::thunderstore::index::{compare_versions, split_dependency, Package};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub full_name: String,
    pub version: String,
    /// The version installed now, which this replaces.
    pub replaces: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Resolution {
    /// What to download and install, dependencies before the packages that need them.
    pub steps: Vec<Step>,
    /// Every package the request needs, installed already or not, in the same order.
    pub all: Vec<String>,
    /// `Owner-Name-x.y.z` dependencies Thunderstore no longer lists.
    pub missing: Vec<String>,
}

/// A package version's dependencies (`Owner-Name-x.y.z`). The caller reads them from the
/// installed package's record, or from the manifest in the downloaded zip.
pub type DepsOf<'d> = dyn FnMut(&Package, &str) -> Result<Vec<String>, String> + 'd;

struct Walk<'a, 'd> {
    find: &'a dyn Fn(&str) -> Option<&'a Package>,
    /// Installed versions, by lower-case `Owner-Name`.
    installed: HashMap<String, String>,
    /// Versions that never change, by lower-case `Owner-Name`.
    fixed: HashMap<String, String>,
    deps_of: &'a mut DepsOf<'d>,
    need: BTreeMap<String, String>,
    order: Vec<String>,
    missing: Vec<String>,
    stack: Vec<String>,
}

impl<'a> Walk<'a, '_> {
    /// Depth first, so dependencies come before the packages that need them. `exact`: under a
    /// modpack, dependencies get the versions named.
    fn visit(&mut self, pkg: &'a Package, version: String, exact: bool) -> Result<(), String> {
        let key = pkg.full_name.clone();
        let version = self
            .fixed
            .get(&key.to_lowercase())
            .cloned()
            .unwrap_or(version);
        let raised = match self.need.get(&key) {
            Some(v) if compare_versions(&version, v) != Ordering::Greater => false,
            _ => {
                self.need.insert(key.clone(), version.clone());
                true
            }
        };
        if !raised || self.stack.contains(&key) {
            return Ok(());
        }
        self.stack.push(key.clone());
        let exact = exact || is_modpack(pkg);
        for dep in (self.deps_of)(pkg, &version)? {
            match split_dependency(&dep)
                .and_then(|(n, v)| (self.find)(n).map(|p| (p, v.to_string())))
            {
                Some((p, named)) => {
                    let latest = p.latest().map(|l| l.version.clone());
                    let have = self
                        .installed
                        .get(&p.full_name.to_lowercase())
                        .filter(|h| compare_versions(h, &named) != Ordering::Less)
                        .cloned();
                    let v = if exact && p.version(&named).is_some() {
                        named
                    } else if exact {
                        // A version Thunderstore no longer has: take the latest instead.
                        latest.unwrap_or(named)
                    } else {
                        have.or(latest).unwrap_or(named)
                    };
                    self.visit(p, v, exact)?
                }
                None => {
                    if !self.missing.contains(&dep) {
                        self.missing.push(dep)
                    }
                }
            }
        }
        self.stack.pop();
        if !self.order.contains(&key) {
            self.order.push(key);
        }
        Ok(())
    }
}

/// Plans installing each `(full_name, version)` in `wanted` (version `None` = latest) with its
/// dependencies. `installed` maps `Owner-Name` to the installed version.
pub fn resolve(
    packages: &[Package],
    installed: &HashMap<String, String>,
    wanted: &[(String, Option<String>)],
    deps_of: &mut DepsOf<'_>,
) -> Result<Resolution, String> {
    resolve_fixed(packages, installed, &HashMap::new(), wanted, deps_of)
}

/// [`resolve`], with versions that never change: `fixed` maps `Owner-Name` to the version a
/// package keeps (one a profile is pinned to). A dependency naming a newer one does not raise
/// it; the caller checks for that afterwards. A version asked for in `wanted` is fixed too.
pub fn resolve_fixed(
    packages: &[Package],
    installed: &HashMap<String, String>,
    fixed: &HashMap<String, String>,
    wanted: &[(String, Option<String>)],
    deps_of: &mut DepsOf<'_>,
) -> Result<Resolution, String> {
    let by_name: HashMap<String, &Package> = packages
        .iter()
        .map(|p| (p.full_name.to_lowercase(), p))
        .collect();
    let find = |name: &str| by_name.get(&name.to_lowercase()).copied();
    let mut fixed: HashMap<String, String> = fixed
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.clone()))
        .collect();
    for (name, version) in wanted {
        if let Some(v) = version {
            fixed.insert(name.to_lowercase(), v.clone());
        }
    }
    let mut walk = Walk {
        find: &find,
        installed: installed
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.clone()))
            .collect(),
        fixed,
        deps_of,
        need: BTreeMap::new(),
        order: Vec::new(),
        missing: Vec::new(),
        stack: Vec::new(),
    };
    let mut explicit: Vec<String> = Vec::new();

    for (name, version) in wanted {
        let pkg =
            find(name).ok_or_else(|| format!("{name} is not on Thunderstore for this game"))?;
        let version = match version {
            Some(v) => {
                pkg.version(v)
                    .ok_or_else(|| format!("{name} has no version {v}"))?;
                v.clone()
            }
            None => pkg
                .latest()
                .ok_or_else(|| format!("{name} has no versions"))?
                .version
                .clone(),
        };
        explicit.push(pkg.full_name.clone());
        walk.stack.clear();
        walk.visit(pkg, version, false)?;
    }

    let Walk {
        need,
        order,
        missing,
        ..
    } = walk;
    let installed: HashMap<String, &String> = installed
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect();
    let all = order.clone();
    let steps = order
        .into_iter()
        .filter_map(|name| {
            let version = need[&name].clone();
            let have = installed.get(&name.to_lowercase()).map(|v| v.to_string());
            let keep = match &have {
                // An explicit request installs exactly that version, even an older one.
                Some(h) if explicit.contains(&name) => h == &version,
                Some(h) => compare_versions(h, &version) != Ordering::Less,
                None => false,
            };
            (!keep).then_some(Step {
                full_name: name,
                version,
                replaces: have,
            })
        })
        .collect();
    Ok(Resolution {
        steps,
        all,
        missing,
    })
}

/// Whether a package is a modpack, whose dependencies are installed at the versions it names.
fn is_modpack(p: &Package) -> bool {
    p.categories
        .iter()
        .any(|c| c.eq_ignore_ascii_case("Modpacks"))
}

/// Packages whose installed version is older than the latest on Thunderstore:
/// `(Owner-Name, installed, latest)`.
pub fn updates(
    packages: &[Package],
    installed: &HashMap<String, String>,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = packages
        .iter()
        .filter_map(|p| {
            let have = installed
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&p.full_name))?
                .1;
            let latest = &p.latest()?.version;
            (compare_versions(latest, have) == Ordering::Greater)
                .then(|| (p.full_name.clone(), have.clone(), latest.clone()))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thunderstore::index::PackageVersion;

    fn pkg(full: &str, versions: &[(&str, &[&str])]) -> Package {
        let (owner, name) = full.split_once('-').unwrap();
        Package {
            owner: owner.into(),
            name: name.into(),
            full_name: full.into(),
            description: String::new(),
            icon: String::new(),
            website: String::new(),
            categories: vec![],
            rating: 0,
            downloads: 0,
            deprecated: false,
            nsfw: false,
            pinned: false,
            updated: String::new(),
            versions: versions
                .iter()
                .map(|(v, deps)| PackageVersion {
                    version: v.to_string(),
                    dependencies: deps.iter().map(|d| d.to_string()).collect(),
                    size: 0,
                })
                .collect(),
        }
    }

    fn index() -> Vec<Package> {
        vec![
            pkg(
                "BepInEx-BepInExPack",
                &[("5.4.2305", &[]), ("5.4.2100", &[])],
            ),
            pkg(
                "Lib-Core",
                &[("2.0.0", &["BepInEx-BepInExPack-5.4.2100"]), ("1.0.0", &[])],
            ),
            pkg(
                "Ann-Mod",
                &[(
                    "1.1.0",
                    &[
                        "Lib-Core-2.0.0",
                        "BepInEx-BepInExPack-5.4.2305",
                        "Gone-Pkg-1.0.0",
                    ],
                )],
            ),
            pkg("Bob-Other", &[("3.0.0", &["Lib-Core-1.0.0"])]),
        ]
    }

    /// Dependencies as the test index spells them (real lists come from manifests).
    fn from_list(p: &Package, v: &str) -> Result<Vec<String>, String> {
        Ok(p.version(v)
            .map(|x| x.dependencies.clone())
            .unwrap_or_default())
    }

    fn names(r: &Resolution) -> Vec<String> {
        r.steps
            .iter()
            .map(|s| format!("{}@{}", s.full_name, s.version))
            .collect()
    }

    #[test]
    fn a_fixed_version_is_not_raised_by_its_dependents() {
        // Lib-Core is pinned at 1.0.0: Ann-Mod naming 2.0.0 does not raise it.
        let installed: HashMap<String, String> = [("Lib-Core".to_string(), "1.0.0".to_string())]
            .into_iter()
            .collect();
        let r = resolve_fixed(
            &index(),
            &installed,
            &installed,
            &[("Ann-Mod".into(), None)],
            &mut from_list,
        )
        .unwrap();
        assert!(
            !names(&r).iter().any(|n| n.starts_with("Lib-Core")),
            "{:?}",
            names(&r)
        );
        assert!(r.all.contains(&"Lib-Core".to_string()));
        // A version asked for by name is fixed too, even under a pin.
        let r = resolve_fixed(
            &index(),
            &installed,
            &installed,
            &[("Lib-Core".into(), Some("2.0.0".into()))],
            &mut from_list,
        )
        .unwrap();
        assert!(names(&r).contains(&"Lib-Core@2.0.0".to_string()));
    }

    #[test]
    fn dependencies_first_highest_version_wins() {
        let r = resolve(
            &index(),
            &HashMap::new(),
            &[("Ann-Mod".into(), None)],
            &mut from_list,
        )
        .unwrap();
        assert_eq!(
            names(&r),
            [
                "BepInEx-BepInExPack@5.4.2305",
                "Lib-Core@2.0.0",
                "Ann-Mod@1.1.0"
            ]
        );
        assert_eq!(r.missing, ["Gone-Pkg-1.0.0"]);
    }

    #[test]
    fn dependencies_get_their_newest_version() {
        // Bob-Other names Lib-Core 1.0.0, built against it; 2.0.0 is installed instead.
        let r = resolve(
            &index(),
            &HashMap::new(),
            &[("Bob-Other".into(), None)],
            &mut from_list,
        )
        .unwrap();
        assert_eq!(
            names(&r),
            [
                "BepInEx-BepInExPack@5.4.2305",
                "Lib-Core@2.0.0",
                "Bob-Other@3.0.0"
            ]
        );
    }

    #[test]
    fn a_modpack_gets_exactly_the_versions_it_names() {
        let mut packages = index();
        let mut pack = pkg(
            "Team-Pack",
            &[("1.0.0", &["Lib-Core-2.0.0", "Bob-Other-3.0.0"])],
        );
        pack.categories = vec!["Modpacks".into()];
        packages.push(pack);
        let r = resolve(
            &packages,
            &HashMap::new(),
            &[("Team-Pack".into(), None)],
            &mut from_list,
        )
        .unwrap();
        // Lib-Core's own dependency is BepInEx 5.4.2100, named; Bob-Other's Lib-Core 1.0.0
        // loses to the 2.0.0 the pack names.
        assert_eq!(
            names(&r),
            [
                "BepInEx-BepInExPack@5.4.2100",
                "Lib-Core@2.0.0",
                "Bob-Other@3.0.0",
                "Team-Pack@1.0.0"
            ]
        );
    }

    #[test]
    fn newer_installed_versions_are_kept() {
        let installed = HashMap::from([
            ("BepInEx-BepInExPack".to_string(), "5.4.2305".to_string()),
            ("Lib-Core".to_string(), "2.0.0".to_string()),
        ]);
        let r = resolve(
            &index(),
            &installed,
            &[("Bob-Other".into(), None)],
            &mut from_list,
        )
        .unwrap();
        assert_eq!(
            names(&r),
            ["Bob-Other@3.0.0"],
            "Lib-Core 2.0.0 satisfies 1.0.0"
        );
        assert_eq!(r.all, ["BepInEx-BepInExPack", "Lib-Core", "Bob-Other"]);

        let old = HashMap::from([("Lib-Core".to_string(), "1.0.0".to_string())]);
        let r = resolve(&index(), &old, &[("Ann-Mod".into(), None)], &mut from_list).unwrap();
        let core = r.steps.iter().find(|s| s.full_name == "Lib-Core").unwrap();
        assert_eq!(core.replaces.as_deref(), Some("1.0.0"));
        assert!(resolve(
            &index(),
            &old,
            &[("Nope-Nope".into(), None)],
            &mut from_list
        )
        .is_err());
    }

    #[test]
    fn reports_updates() {
        let installed = HashMap::from([
            ("BepInEx-BepInExPack".to_string(), "5.4.2100".to_string()),
            ("Ann-Mod".to_string(), "1.1.0".to_string()),
        ]);
        assert_eq!(
            updates(&index(), &installed),
            [(
                "BepInEx-BepInExPack".to_string(),
                "5.4.2100".to_string(),
                "5.4.2305".to_string()
            )]
        );
    }
}
