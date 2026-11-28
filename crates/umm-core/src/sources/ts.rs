//! Thunderstore packages for a game set up with its mod loader from Thunderstore (BepInEx or
//! Shimloader): search, install with dependencies, update. Packages are stored as ordinary
//! mods, laid out for the game.

use crate::{CoreError, Result, Session};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use umm_deploy::GameLock;
use umm_gamedef::GameDef;
use umm_sources::thunderstore::ecosystem::{package_index_url, SteamMatch};
use umm_sources::thunderstore::index::{compare_versions, split_dependency, Package};
use umm_sources::thunderstore::{layout, resolve, Ecosystem};
use umm_sources::Site;
use umm_store::{fsutil, ImportOptions, Layout, ProfileEntry, ThunderstoreMeta};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TsSort {
    /// Pinned first, then rating (Thunderstore's own order).
    #[default]
    Rating,
    Downloads,
    Updated,
    Name,
}

#[derive(Debug, Clone, Default)]
pub struct TsQuery {
    pub text: String,
    pub category: Option<String>,
    pub sort: TsSort,
    pub include_deprecated: bool,
    pub include_nsfw: bool,
    pub installed_only: bool,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct TsItem {
    pub package: Package,
    /// The installed version, if any.
    pub installed: Option<String>,
    pub update: bool,
}

#[derive(Debug, Clone)]
pub struct TsPage {
    pub total: usize,
    pub items: Vec<TsItem>,
    /// Every category in the community, by name.
    pub categories: Vec<String>,
    /// The package list is an old copy because Thunderstore could not be reached.
    pub stale: bool,
    /// The game's mod loader, by name ("BepInEx", "Shimloader").
    pub loader_name: String,
    /// The community's package for that loader, `Owner-Name`.
    pub loader_package: Option<String>,
    /// The loader's package is installed for the game.
    pub loader_installed: bool,
    /// The installed pack cannot load in this game (built for the other architecture).
    pub loader_warning: Option<String>,
    /// A known pack for the game's build to offer instead (`Owner-Name`).
    pub loader_suggestion: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TsInstallReport {
    /// `Owner-Name`, version, and the version it replaced.
    pub installed: Vec<(String, String, Option<String>)>,
    /// Dependencies Thunderstore no longer lists, as `Owner-Name-x.y.z`.
    pub missing: Vec<String>,
    /// Problems to show, such as a BepInEx pack built for the other architecture.
    pub warnings: Vec<String>,
    /// Other changes made along the way, such as an old BepInEx pack turned off.
    pub notes: Vec<String>,
}

/// A package looked up by name.
#[derive(Debug, Clone)]
pub struct TsLookup {
    pub package: Package,
    /// The game's community lists it.
    pub listed: bool,
    /// When not listed: how the community's moderators reviewed it (`rejected`,
    /// `unreviewed`), or `None` if it was never listed there.
    pub review_status: Option<String>,
}

/// A version of a Thunderstore package, offered to change to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsVersionChoice {
    pub version: String,
    pub size: u64,
    /// The version the profile uses now.
    pub installed: bool,
    /// The version the profile is pinned to.
    pub pinned: bool,
    /// Stored already: choosing it needs no download.
    pub stored: bool,
    /// Packages of the profile that need a newer version than this one: `(Owner-Name, the
    /// version they name)`. Choosing it means downgrading them too (see
    /// [`PinConflictChoice::Downgrade`]).
    pub needed_by: Vec<(String, String)>,
}

/// A package installed for a game: its mod id and Thunderstore details.
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub mod_id: String,
    pub meta: ThunderstoreMeta,
}

pub(crate) struct TsCache {
    pub(crate) packages: HashMap<String, (Arc<Vec<Package>>, bool)>,
}

impl TsCache {
    pub(crate) fn new() -> Self {
        TsCache {
            packages: HashMap::new(),
        }
    }
}

/// Mod managers published on Thunderstore. They are not mods for the game, so search leaves
/// them out unless the Tools category is picked.
const MOD_MANAGERS: &[&str] = &["ebkr-r2modman", "Kesomannen-GaleModManager"];

/// r2modman or Gale. Only these two, by name: categories and descriptions cannot tell mod
/// managers from mods reliably (real mods and libraries are tagged "Tools" too).
pub fn is_mod_manager(p: &Package) -> bool {
    MOD_MANAGERS
        .iter()
        .any(|m| m.eq_ignore_ascii_case(&p.full_name))
}

/// Mod id for a package: its `Owner-Name`, as a slug.
pub fn package_mod_id(full_name: &str) -> String {
    umm_gamedef::slug(full_name).unwrap_or_else(|| "package".into())
}

impl Session {
    /// The game's community and Thunderstore data; refused unless the game is set up with
    /// its mod loader from Thunderstore.
    fn ts_game(&self, id: &str) -> Result<(GameDef, String, Arc<Ecosystem>)> {
        let def = self.def(id)?.def;
        let community = match (&def.loader, def.community()) {
            (Some(l), Some(c)) if l.from_thunderstore() => c.to_string(),
            _ => {
                return Err(CoreError::Invalid(format!(
                "{} is not set up with a mod loader from Thunderstore; change its mod loader first",
                def.name
            )))
            }
        };
        let eco = self.ecosystem(false)?;
        if eco.community(&community).is_none() {
            return Err(CoreError::Invalid(format!(
                "Thunderstore has no community `{community}`"
            )));
        }
        Ok((def, community, eco))
    }

    fn ts_match<'a>(eco: &'a Ecosystem, community: &str) -> SteamMatch<'a> {
        eco.community(community).expect("checked by ts_game")
    }

    /// The community's packages, cached for an hour (`refresh` downloads them again).
    pub fn ts_packages(&self, id: &str, refresh: bool) -> Result<(Arc<Vec<Package>>, bool)> {
        let (_, community, eco) = self.ts_game(id)?;
        let mut cache = self.ts.lock().unwrap_or_else(|e| e.into_inner());
        if !refresh {
            if let Some(hit) = cache.packages.get(&community) {
                let file = self.ts_index_file(&community);
                let young = umm_sources::thunderstore::cache::age(&file)
                    .is_some_and(|a| a <= umm_sources::thunderstore::index::MAX_AGE);
                if young || hit.1 {
                    return Ok(hit.clone());
                }
            }
        }
        let url = package_index_url(&Self::ts_match(&eco, &community));
        let (list, fresh) = umm_sources::thunderstore::index::load(
            self.http(),
            &url,
            &self.ts_index_file(&community),
            refresh,
        )?;
        let stale = fresh == umm_sources::thunderstore::cache::Freshness::Stale;
        let entry = (Arc::new(list), stale);
        cache.packages.insert(community, entry.clone());
        Ok(entry)
    }

    fn ts_index_file(&self, community: &str) -> std::path::PathBuf {
        self.thunderstore_dir()
            .join("communities")
            .join(format!("{community}.json.gz"))
    }

    /// Thunderstore packages installed for the game, by `Owner-Name`.
    pub fn ts_installed(&self, id: &str) -> Result<Vec<InstalledPackage>> {
        Ok(self
            .mods(id)?
            .into_iter()
            .filter_map(|m| {
                m.thunderstore
                    .map(|meta| InstalledPackage { mod_id: m.id, meta })
            })
            .collect())
    }

    /// The community's package for the game's loader (also what a missing loader is installed
    /// from). For BepInEx, the pack for this build of the game: an IL2CPP game never gets the
    /// plain Mono pack, a Mono game never an IL2CPP one (game-specific packs like
    /// BepInExPack_AmongUs do not say which they are). Among the rest, BepInEx's own pack if
    /// listed, else the most downloaded.
    fn loader_package(
        &self,
        def: &GameDef,
        eco: &Ecosystem,
        packages: &[Package],
    ) -> Result<Option<String>> {
        let kind = def.ts_loader().unwrap_or(umm_gamedef::BEPINEX_KIND);
        let bepinex = kind == umm_gamedef::BEPINEX_KIND;
        let il2cpp = self
            .install_path(def)?
            .is_some_and(|(p, _)| p.join("GameAssembly.dll").is_file());
        Ok(packages
            .iter()
            .filter(|p| {
                eco.modloader_for(&p.full_name, kind)
                    .is_some_and(|m| m.loader == kind)
                    && !umm_gamedef::is_companion_loader(&p.full_name)
            })
            .filter(|p| !p.deprecated)
            .filter(|p| {
                if !bepinex {
                    true
                } else if il2cpp {
                    !p.full_name.eq_ignore_ascii_case("BepInEx-BepInExPack")
                } else {
                    !p.full_name.to_lowercase().contains("il2cpp")
                }
            })
            .max_by_key(|p| {
                let own = p.owner == "BepInEx" && p.name.starts_with("BepInExPack");
                (own, p.downloads)
            })
            .map(|p| p.full_name.clone()))
    }

    pub fn ts_search(&self, id: &str, q: &TsQuery) -> Result<TsPage> {
        let (def, _, eco) = self.ts_game(id)?;
        let kind = def
            .ts_loader()
            .unwrap_or(umm_gamedef::BEPINEX_KIND)
            .to_string();
        let (packages, stale) = self.ts_packages(id, false)?;
        let installed_pkgs = self.ts_installed(id)?;
        let loader_installed = installed_pkgs
            .iter()
            .any(|p| p.meta.loader && !umm_gamedef::is_companion_loader(&p.meta.full_name));
        let installed: HashMap<String, String> = installed_pkgs
            .into_iter()
            .map(|p| (p.meta.full_name, p.meta.version))
            .collect();
        let loader_package = self.loader_package(&def, &eco, &packages)?;
        let have = |p: &Package| {
            installed
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&p.full_name))
                .map(|(_, v)| v.clone())
        };
        let text = q.text.trim().to_lowercase();
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut hits: Vec<&Package> = packages
            .iter()
            .filter(|p| q.include_deprecated || !p.deprecated || have(p).is_some())
            .filter(|p| q.include_nsfw || !p.nsfw)
            .filter(|p| q.category.as_deref() == Some("Tools") || !is_mod_manager(p))
            .filter(|p| !q.installed_only || have(p).is_some())
            .filter(|p| {
                q.category
                    .as_ref()
                    .is_none_or(|c| p.categories.iter().any(|x| x == c))
            })
            .filter(|p| {
                words.is_empty() || {
                    let hay = format!("{} {} {}", p.name, p.owner, p.description).to_lowercase();
                    words.iter().all(|w| hay.contains(w))
                }
            })
            .collect();
        match q.sort {
            TsSort::Rating => {
                hits.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.rating.cmp(&a.rating)))
            }
            TsSort::Downloads => hits.sort_by(|a, b| b.downloads.cmp(&a.downloads)),
            TsSort::Updated => hits.sort_by(|a, b| b.updated.cmp(&a.updated)),
            TsSort::Name => hits.sort_by_key(|p| p.name.to_lowercase()),
        }
        let mut categories: Vec<String> = packages
            .iter()
            .flat_map(|p| p.categories.iter().cloned())
            .collect();
        categories.sort();
        categories.dedup();
        let limit = if q.limit == 0 { 50 } else { q.limit };
        let items = hits
            .iter()
            .skip(q.offset)
            .take(limit)
            .map(|p| {
                let installed = have(p);
                let update = match (&installed, p.latest()) {
                    (Some(i), Some(l)) => compare_versions(&l.version, i) == Ordering::Greater,
                    _ => false,
                };
                TsItem {
                    package: (*p).clone(),
                    installed,
                    update,
                }
            })
            .collect();
        let mismatch = self.loader_mismatch(id);
        Ok(TsPage {
            total: hits.len(),
            items,
            categories,
            stale,
            loader_name: umm_gamedef::loader_name(&kind).to_string(),
            loader_package,
            loader_installed,
            loader_warning: mismatch.as_ref().map(|m| m.message.clone()),
            loader_suggestion: mismatch.and_then(|m| m.suggestion),
        })
    }

    /// Installed packages with a newer version: `(Owner-Name, installed, latest)`. Packages
    /// every profile having them is pinned in are left out: nothing would use the update.
    pub fn ts_updates(&self, id: &str) -> Result<Vec<(String, String, String)>> {
        let (packages, _) = self.ts_packages(id, false)?;
        let game = self.data.game(id);
        let mut pinned: HashSet<String> = HashSet::new();
        let mut follow: HashSet<String> = HashSet::new();
        for name in umm_store::list_profiles(&game) {
            for e in umm_store::load_profile(&game, &name)?.mods {
                match e.version {
                    Some(_) => pinned.insert(e.id),
                    None => follow.insert(e.id),
                };
            }
        }
        let installed: HashMap<String, String> = self
            .ts_installed(id)?
            .into_iter()
            .filter(|p| follow.contains(&p.mod_id) || !pinned.contains(&p.mod_id))
            .map(|p| (p.meta.full_name, p.meta.version))
            .collect();
        Ok(resolve::updates(&packages, &installed))
    }

    /// Installs packages (version `None` = latest) with their dependencies, and makes sure all
    /// of them are enabled in `profile`. Pins stay as they are. Refused while mods are
    /// deployed, and (with [`PinConflictChoice::Ask`]) when a pinned package is older than a
    /// package in the profile needs.
    pub fn ts_install(
        &self,
        id: &str,
        wanted: &[(String, Option<String>)],
        profile: &str,
        choice: PinConflictChoice,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        let wants = wanted
            .iter()
            .map(|(n, v)| Want::new(n, v.clone(), PinIntent::Keep))
            .collect::<Vec<_>>();
        self.install_with(id, &wants, profile, choice, progress, None)
    }

    /// Looks up a package by `Owner-Name`, including one the game's community does not list
    /// (not submitted there, unreviewed, or rejected by its moderators).
    pub fn ts_lookup(&self, id: &str, full_name: &str) -> Result<TsLookup> {
        let (_, community, _) = self.ts_game(id)?;
        let (packages, _) = self.ts_packages(id, false)?;
        if let Some(p) = packages
            .iter()
            .find(|p| p.full_name.eq_ignore_ascii_case(full_name.trim()))
        {
            return Ok(TsLookup {
                package: p.clone(),
                listed: true,
                review_status: None,
            });
        }
        let f = umm_sources::thunderstore::index::fetch_package(self.http(), full_name.trim())?;
        let review_status = f
            .listings
            .iter()
            .find(|(c, _)| c == &community)
            .map(|(_, s)| s.clone());
        Ok(TsLookup {
            package: f.package,
            listed: false,
            review_status,
        })
    }

    /// Installs a package the game's community does not list, by `Owner-Name` (version
    /// `None` = latest), with its dependencies. The caller has shown the user that it is not
    /// listed and why ([`Session::ts_lookup`]).
    pub fn ts_install_unlisted(
        &self,
        id: &str,
        full_name: &str,
        version: Option<String>,
        profile: &str,
        choice: PinConflictChoice,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        let mut found = self.ts_lookup(id, full_name)?;
        if found.listed {
            return self.ts_install(
                id,
                &[(found.package.full_name, version)],
                profile,
                choice,
                progress,
            );
        }
        if let Some(v) = &version {
            if found.package.version(v).is_none() {
                found
                    .package
                    .versions
                    .push(umm_sources::thunderstore::index::PackageVersion {
                        version: v.clone(),
                        dependencies: Vec::new(),
                        size: 0,
                    });
            }
        }
        let wants = [Want::new(
            &found.package.full_name,
            version,
            PinIntent::Keep,
        )];
        self.install_with(id, &wants, profile, choice, progress, Some(found.package))
    }

    /// Every stored version of each installed package.
    fn ts_stored(&self, id: &str) -> Result<Vec<Stored>> {
        let game = self.data.game(id);
        Ok(self
            .ts_installed(id)?
            .into_iter()
            .map(|p| {
                let others = umm_store::versions::other_versions(&game, &p.mod_id)
                    .into_iter()
                    .filter_map(|v| {
                        umm_store::versions::load_mod_version(&game, &p.mod_id, Some(&v))
                            .ok()?
                            .0
                            .thunderstore
                    })
                    .collect();
                Stored {
                    mod_id: p.mod_id,
                    latest: p.meta,
                    others,
                }
            })
            .collect())
    }

    /// Removes the stored package versions no profile uses: other versions no profile is
    /// pinned to, and the latest when every profile with the package is pinned to another
    /// (the newest pinned version becomes the latest). A package in no profile keeps its
    /// latest. Left for later while mods are deployed, or while another operation holds the
    /// game.
    pub(crate) fn prune_versions(&self, id: &str) -> Result<()> {
        let game = self.data.game(id);
        let Ok(_lock) = GameLock::acquire(&game) else {
            return Ok(());
        };
        if game.journal_path().exists() {
            return Ok(());
        }
        let mut pins: HashMap<String, HashSet<String>> = HashMap::new();
        // Packages some profile follows the latest version of.
        let mut follow: HashSet<String> = HashSet::new();
        for name in umm_store::list_profiles(&game) {
            let Ok(p) = umm_store::load_profile(&game, &name) else {
                // An unreadable profile may use anything.
                return Ok(());
            };
            for e in p.mods {
                match e.version {
                    Some(v) => {
                        pins.entry(e.id).or_default().insert(v);
                    }
                    None => {
                        follow.insert(e.id);
                    }
                }
            }
        }
        for m in umm_store::list_mods(&game)? {
            let keep = pins.remove(&m.id).unwrap_or_default();
            let others = umm_store::versions::other_versions(&game, &m.id);
            // The latest, used by no profile: the newest pinned version takes its place.
            let unused_latest = !follow.contains(&m.id)
                && !keep.is_empty()
                && m.version.as_ref().is_none_or(|v| !keep.contains(v));
            if unused_latest {
                if let Some(top) = others
                    .iter()
                    .filter(|v| keep.contains(*v))
                    .max_by(|a, b| compare_versions(a, b))
                {
                    umm_store::versions::promote_version(&game, &m.id, top)?;
                }
            }
            if !others.is_empty() {
                umm_store::versions::prune_versions(&game, &m.id, &keep)?;
            }
        }
        Ok(())
    }

    fn install_with(
        &self,
        id: &str,
        wants: &[Want],
        profile: &str,
        choice: PinConflictChoice,
        progress: &mut dyn FnMut(&str),
        extra: Option<Package>,
    ) -> Result<TsInstallReport> {
        let (def, community, eco) = self.ts_game(id)?;
        let m = Self::ts_match(&eco, &community);
        let (listed, _) = self.ts_packages(id, false)?;
        // A package from outside the community's list joins it for this install only.
        let combined: Vec<Package>;
        let packages: &[Package] = match extra {
            Some(p) => {
                combined = listed.iter().cloned().chain(std::iter::once(p)).collect();
                &combined
            }
            None => &listed,
        };
        let find = |name: &str| {
            packages
                .iter()
                .find(|p| p.full_name.eq_ignore_ascii_case(name))
        };
        let game = self.data.game(id);
        game.ensure()?;
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(id.into()));
        }
        let stored = self.ts_stored(id)?;
        let stored_by_name: HashMap<String, &Stored> = stored
            .iter()
            .map(|s| (s.latest.full_name.to_lowercase(), s))
            .collect();
        // Every stored version's dependencies, by lower-case `Owner-Name` and version.
        let recorded: HashMap<(String, String), Vec<String>> = stored
            .iter()
            .flat_map(|s| std::iter::once(&s.latest).chain(&s.others))
            .map(|t| {
                (
                    (t.full_name.to_lowercase(), t.version.clone()),
                    t.dependencies.clone(),
                )
            })
            .collect();
        let before = umm_store::load_profile(&game, profile)?;
        // The version of each installed package this profile uses, and its pins.
        let mut uses: HashMap<String, String> = HashMap::new();
        let mut pins: HashMap<String, String> = HashMap::new();
        for s in &stored {
            let name = s.latest.full_name.to_lowercase();
            match before.pin(&s.mod_id) {
                Some(v) => {
                    pins.insert(name.clone(), v.to_string());
                    uses.insert(name, v.to_string());
                }
                None => {
                    uses.insert(name, s.latest.version.clone());
                }
            }
        }
        // The pins this install sets or removes.
        let mut asked: HashSet<String> = HashSet::new();
        for w in wants {
            let name = w.name.to_lowercase();
            match (w.pin, &w.version) {
                (PinIntent::Follow, _) => {
                    pins.remove(&name);
                    if let Some(s) = stored_by_name.get(&name) {
                        uses.insert(name, s.latest.version.clone());
                    }
                }
                (PinIntent::Pin, Some(v)) => {
                    pins.insert(name.clone(), v.clone());
                    asked.insert(name);
                }
                _ => {}
            }
        }

        // One BepInEx pack per game. A pack asked for by name, else one already installed, is
        // the game's pack; a dependency on any other pack counts as met by it (mods often name
        // a game's original pack, which may not suit the game any more).
        // A companion package (Resonite's renderer BepInEx) comes with the loader instead of
        // replacing it.
        let is_loader =
            |name: &str| eco.modloader(name).is_some() && !umm_gamedef::is_companion_loader(name);
        let in_profile: Vec<String> = before
            .mods
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.id.clone())
            .collect();
        let loaders = || {
            stored.iter().filter(|s| {
                s.latest.loader && !umm_gamedef::is_companion_loader(&s.latest.full_name)
            })
        };
        let chosen_loader: Option<String> = wants
            .iter()
            .map(|w| w.name.clone())
            .find(|n| is_loader(n))
            .or_else(|| {
                // The pack this profile uses, else any installed one.
                loaders()
                    .find(|s| in_profile.contains(&s.mod_id))
                    .or_else(|| loaders().next())
                    .map(|s| s.latest.full_name.clone())
            });
        // No loader yet, and none of the mods names one (some do not, though they need it):
        // it comes with this install anyway, as the Thunderstore tab says.
        let loader_to_add = match (&chosen_loader, def.ts_loader()) {
            (None, Some(_)) => self.loader_package(&def, &eco, packages)?,
            _ => None,
        };
        let other_loader = |dep: &str| {
            split_dependency(dep).is_some_and(|(n, _)| {
                is_loader(n)
                    && chosen_loader
                        .as_deref()
                        .is_some_and(|c| !c.eq_ignore_ascii_case(n))
            })
        };
        // The enabled Thunderstore packages of the profile, by lower-case `Owner-Name`.
        let profile_packages: Vec<String> = stored
            .iter()
            .filter(|s| in_profile.contains(&s.mod_id))
            .map(|s| s.latest.full_name.to_lowercase())
            .collect();

        // Dependencies come from each package version's manifest: a stored version's record,
        // or the zip, downloaded once and kept for the install below.
        let mut zips: HashMap<(String, String), Vec<u8>> = HashMap::new();
        let net_error: std::cell::RefCell<Option<CoreError>> = Default::default();
        let mut added_loader = None;
        // Packages moved to an older version to suit a pin, and pins removed.
        let mut lowered: HashMap<String, String> = HashMap::new();
        let mut unpinned: HashSet<String> = HashSet::new();
        let settled = {
            let mut deps_of =
                |pkg: &Package, version: &str| -> std::result::Result<Vec<String>, String> {
                    let key = (pkg.full_name.to_lowercase(), version.to_string());
                    if let Some(deps) = recorded.get(&key) {
                        return Ok(deps.iter().filter(|d| !other_loader(d)).cloned().collect());
                    }
                    let key = (pkg.full_name.clone(), version.to_string());
                    if !zips.contains_key(&key) {
                        progress(&format!("Downloading {} {}", pkg.name, version));
                        let bytes = self.http().get(&pkg.download_url(version)).map_err(|e| {
                            let msg = e.to_string();
                            *net_error.borrow_mut() = Some(Site::Thunderstore.net(e).into());
                            msg
                        })?;
                        zips.insert(key.clone(), bytes);
                    }
                    let manifest = layout::manifest(&zips[&key]).map_err(|e| e.to_string())?;
                    let deps = manifest.map(|m| m.dependencies).unwrap_or_default();
                    Ok(deps.into_iter().filter(|d| !other_loader(d)).collect())
                };
            let mut round = 0;
            loop {
                round += 1;
                if round > 10 {
                    break Err(CoreError::Invalid(
                        "could not find versions that suit every pin in this profile".into(),
                    ));
                }
                // The pins this round: the profile's, as changed so far.
                let mut fixed: HashMap<String, String> = pins
                    .iter()
                    .filter(|(n, _)| !unpinned.contains(*n))
                    .map(|(n, v)| (n.clone(), v.clone()))
                    .collect();
                fixed.extend(lowered.iter().map(|(n, v)| (n.clone(), v.clone())));
                let mut have = uses.clone();
                for n in &unpinned {
                    if let Some(s) = stored_by_name.get(n) {
                        have.insert(n.clone(), s.latest.version.clone());
                    }
                }
                let mut request: Vec<(String, Option<String>)> = wants
                    .iter()
                    .map(|w| (w.name.clone(), w.version.clone()))
                    .collect();
                for (n, v) in &lowered {
                    if let Some(p) = find(n) {
                        request.push((p.full_name.clone(), Some(v.clone())));
                    }
                }
                let plan =
                    match resolve::resolve_fixed(packages, &have, &fixed, &request, &mut deps_of) {
                        Ok(p) if !p.all.iter().any(|n| is_loader(n)) && loader_to_add.is_some() => {
                            let pkg = loader_to_add.clone().expect("checked");
                            let mut with_loader = vec![(pkg.clone(), None)];
                            with_loader.extend(request.iter().cloned());
                            added_loader = Some(pkg);
                            resolve::resolve_fixed(
                                packages,
                                &have,
                                &fixed,
                                &with_loader,
                                &mut deps_of,
                            )
                        }
                        r => r,
                    };
                let plan = match plan {
                    Ok(p) => p,
                    Err(msg) => {
                        break Err(net_error
                            .borrow_mut()
                            .take()
                            .unwrap_or(CoreError::Invalid(msg)))
                    }
                };
                // The versions the profile would use.
                let mut used = have.clone();
                for s in &plan.steps {
                    used.insert(s.full_name.to_lowercase(), s.version.clone());
                }
                for (n, v) in &fixed {
                    used.insert(n.clone(), v.clone());
                }
                // Pinned packages older than a package of the profile needs.
                let mut conflicts = Vec::new();
                let mut check: Vec<String> = plan.all.iter().map(|n| n.to_lowercase()).collect();
                check.extend(profile_packages.iter().cloned());
                check.sort();
                check.dedup();
                let mut failed = None;
                for x in &check {
                    let (Some(pkg), Some(xv)) = (find(x), used.get(x)) else {
                        continue;
                    };
                    let deps = match deps_of(pkg, xv) {
                        Ok(d) => d,
                        Err(msg) => {
                            failed = Some(
                                net_error
                                    .borrow_mut()
                                    .take()
                                    .unwrap_or(CoreError::Invalid(msg)),
                            );
                            break;
                        }
                    };
                    for d in deps {
                        let Some((p, needs)) = split_dependency(&d) else {
                            continue;
                        };
                        let p_key = p.to_lowercase();
                        let Some(pin) = fixed.get(&p_key) else {
                            continue;
                        };
                        if compare_versions(pin, needs) == Ordering::Less {
                            conflicts.push(PinConflict {
                                dependent: pkg.full_name.clone(),
                                dependent_version: xv.clone(),
                                pinned: find(p).map_or(p.to_string(), |q| q.full_name.clone()),
                                pin: pin.clone(),
                                needs: needs.to_string(),
                                can_unpin: !asked.contains(&p_key) && !lowered.contains_key(&p_key),
                            });
                        }
                    }
                }
                if let Some(e) = failed {
                    break Err(e);
                }
                if conflicts.is_empty() || choice == PinConflictChoice::Ignore {
                    break Ok((plan, used, fixed));
                }
                match choice {
                    PinConflictChoice::Ask | PinConflictChoice::Ignore => {
                        break Err(CoreError::PinConflicts(conflicts))
                    }
                    PinConflictChoice::Unpin => {
                        let stuck: Vec<PinConflict> =
                            conflicts.iter().filter(|c| !c.can_unpin).cloned().collect();
                        if !stuck.is_empty() {
                            break Err(CoreError::PinConflicts(stuck));
                        }
                        unpinned.extend(conflicts.iter().map(|c| c.pinned.to_lowercase()));
                    }
                    PinConflictChoice::Downgrade => {
                        let mut stuck = None;
                        for c in &conflicts {
                            let x = c.dependent.to_lowercase();
                            let pin = &c.pin;
                            let pinned = c.pinned.to_lowercase();
                            let pkg = find(&x).expect("checked above");
                            let mut found = None;
                            if !asked.contains(&x) {
                                for v in pkg.versions.iter().map(|v| v.version.clone()) {
                                    if compare_versions(&v, &c.dependent_version) != Ordering::Less
                                    {
                                        continue;
                                    }
                                    let deps = match deps_of(pkg, &v) {
                                        Ok(d) => d,
                                        Err(_) => continue,
                                    };
                                    let fits = deps.iter().all(|d| {
                                        split_dependency(d).is_none_or(|(n, needs)| {
                                            !n.eq_ignore_ascii_case(&pinned)
                                                || compare_versions(needs, pin) != Ordering::Greater
                                        })
                                    });
                                    if fits {
                                        found = Some(v);
                                        break;
                                    }
                                }
                            }
                            match found {
                                Some(v) => {
                                    lowered.insert(x, v);
                                }
                                None => {
                                    stuck = Some(CoreError::Invalid(format!(
                                        "no version of {} works with {} {}",
                                        c.dependent, c.pinned, c.pin
                                    )));
                                    break;
                                }
                            }
                        }
                        if let Some(e) = stuck {
                            break Err(e);
                        }
                    }
                }
            }
        };
        let (plan, used, fixed) = settled?;
        let mut report = TsInstallReport {
            missing: plan.missing.clone(),
            ..Default::default()
        };
        if let Some(pkg) = &added_loader {
            report.notes.push(format!(
                "Installed {pkg} too: it is the game's mod loader, which every mod needs"
            ));
        }
        for (n, v) in &lowered {
            let name = find(n).map_or(n.clone(), |p| p.full_name.clone());
            report.notes.push(format!(
                "Pinned {name} at {v}, which works with this profile's pins"
            ));
        }
        for n in &unpinned {
            let name = find(n).map_or(n.clone(), |p| p.full_name.clone());
            report.notes.push(format!(
                "Removed this profile's pin on {name}; it follows the latest version"
            ));
        }
        // Packages stored as an older version than the latest: the profile is pinned to them.
        let mut older: HashSet<String> = HashSet::new();
        for step in &plan.steps {
            let pkg = find(&step.full_name).expect("resolved from this list");
            let key = step.full_name.to_lowercase();
            let latest = stored_by_name.get(&key).map(|s| s.latest.version.clone());
            let as_latest = latest
                .as_ref()
                .is_none_or(|l| compare_versions(&step.version, l) == Ordering::Greater);
            if !as_latest {
                older.insert(key.clone());
                let mod_id = package_mod_id(&pkg.full_name);
                if umm_store::versions::has_version(&game, &mod_id, &step.version) {
                    continue;
                }
            }
            let zip_key = (step.full_name.clone(), step.version.clone());
            let zip = match zips.remove(&zip_key) {
                Some(z) => z,
                None => {
                    progress(&format!("Downloading {} {}", pkg.name, step.version));
                    self.http()
                        .get(&pkg.download_url(&step.version))
                        .map_err(|e| Site::Thunderstore.net(e))?
                }
            };
            progress(&format!("Installing {} {}", pkg.name, step.version));
            self.store_package(id, &community, m, &eco, pkg, &step.version, &zip, as_latest)?;
            report.installed.push((
                step.full_name.clone(),
                step.version.clone(),
                step.replaces.clone(),
            ));
        }

        let mut p = umm_store::load_profile(&game, profile)?;
        // A pack installed by name replaces any other pack in this profile: turned off, kept.
        if let Some(chosen) = wants.iter().map(|w| &w.name).find(|n| is_loader(n)) {
            for other in stored.iter().filter(|o| {
                o.latest.loader
                    && !umm_gamedef::is_companion_loader(&o.latest.full_name)
                    && !o.latest.full_name.eq_ignore_ascii_case(chosen)
            }) {
                if let Some(e) = p
                    .mods
                    .iter_mut()
                    .find(|e| e.id == other.mod_id && e.enabled)
                {
                    e.enabled = false;
                    report.notes.push(format!(
                        "Turned off {} in this profile; {} replaces it",
                        other.latest.full_name, chosen
                    ));
                }
            }
        }
        for name in &plan.all {
            let mod_id = package_mod_id(name);
            match p.mods.iter_mut().find(|e| e.id == mod_id) {
                Some(e) => e.enabled = true,
                None => p.mods.push(ProfileEntry::new(mod_id, true)),
            }
        }
        // Each package's pin, as this install leaves it.
        let mut names: HashMap<String, String> = stored
            .iter()
            .map(|s| (s.mod_id.clone(), s.latest.full_name.to_lowercase()))
            .collect();
        names.extend(
            plan.all
                .iter()
                .map(|n| (package_mod_id(n), n.to_lowercase())),
        );
        for e in &mut p.mods {
            let Some(name) = names.get(&e.id) else {
                continue;
            };
            let follows = wants
                .iter()
                .any(|w| w.pin == PinIntent::Follow && w.name.eq_ignore_ascii_case(name));
            e.version = if unpinned.contains(name) || follows {
                None
            } else if let Some(v) = fixed.get(name) {
                Some(v.clone())
            } else if older.contains(name) {
                used.get(name).cloned()
            } else {
                e.version.take()
            };
        }
        umm_store::save_profile(&game, &p)?;
        drop(_lock);
        self.prune_versions(id)?;
        report.warnings.extend(self.loader_architecture_warning(id));
        Ok(report)
    }

    /// Updates installed packages (all of them when `only` is empty) to their latest versions,
    /// with any new dependencies. Packages named in `only` follow their latest version in
    /// `profile` from then on; updating all of them leaves the profile's pins as they are.
    pub fn ts_update(
        &self,
        id: &str,
        only: &[String],
        profile: &str,
        choice: PinConflictChoice,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        let intent = if only.is_empty() {
            PinIntent::Keep
        } else {
            PinIntent::Follow
        };
        let mut wants: Vec<Want> = self
            .ts_updates(id)?
            .into_iter()
            .filter(|(n, _, _)| only.is_empty() || only.iter().any(|o| o.eq_ignore_ascii_case(n)))
            .map(|(n, _, latest)| Want::new(&n, Some(latest), intent))
            .collect();
        // A package already at its latest version, pinned in this profile, follows it.
        for o in only {
            if !wants.iter().any(|w| w.name.eq_ignore_ascii_case(o)) {
                wants.push(Want::new(o, None, PinIntent::Follow));
            }
        }
        if wants.is_empty() {
            return Ok(TsInstallReport::default());
        }
        self.install_with(id, &wants, profile, choice, progress, None)
    }

    /// Makes `profile` follow a package's latest installed version again (installing a
    /// newer one if Thunderstore has it), removing its pin.
    pub fn ts_follow_latest(
        &self,
        id: &str,
        full_name: &str,
        profile: &str,
        choice: PinConflictChoice,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        self.ts_update(id, &[full_name.to_string()], profile, choice, progress)
    }

    /// The versions of a package the game's list has, newest first, as `profile` sees them:
    /// the one it uses, and the packages of the profile that need a newer one (the lowest
    /// version that suits them all is the highest any of them names).
    pub fn ts_versions(
        &self,
        id: &str,
        full_name: &str,
        profile: &str,
    ) -> Result<Vec<TsVersionChoice>> {
        let (packages, _) = self.ts_packages(id, false)?;
        let pkg = packages
            .iter()
            .find(|p| p.full_name.eq_ignore_ascii_case(full_name))
            .ok_or_else(|| {
                CoreError::Invalid(format!("{full_name} is not on Thunderstore for this game"))
            })?;
        let game = self.data.game(id);
        let prof = umm_store::load_profile(&game, profile)?;
        let stored = self.ts_stored(id)?;
        let this = stored
            .iter()
            .find(|s| s.latest.full_name.eq_ignore_ascii_case(full_name));
        let pin = this.and_then(|s| prof.pin(&s.mod_id)).map(str::to_string);
        let current = pin
            .clone()
            .or_else(|| this.map(|s| s.latest.version.clone()));
        // What the profile's other packages, at the versions it uses, name for it.
        let wanted: Vec<(String, String)> = stored
            .iter()
            .filter(|s| !s.latest.full_name.eq_ignore_ascii_case(full_name))
            .filter(|s| prof.enabled().any(|e| e.id == s.mod_id))
            .filter_map(|s| {
                let meta = match prof.pin(&s.mod_id) {
                    Some(v) => s
                        .others
                        .iter()
                        .find(|o| o.version == v)
                        .unwrap_or(&s.latest),
                    None => &s.latest,
                };
                meta.dependencies.iter().find_map(|d| {
                    let (name, version) = split_dependency(d)?;
                    name.eq_ignore_ascii_case(full_name)
                        .then(|| (s.latest.full_name.clone(), version.to_string()))
                })
            })
            .collect();
        let has = |v: &str| {
            this.is_some_and(|s| s.latest.version == v || s.others.iter().any(|o| o.version == v))
        };
        Ok(pkg
            .versions
            .iter()
            .map(|v| TsVersionChoice {
                version: v.version.clone(),
                size: v.size,
                installed: current.as_deref() == Some(v.version.as_str()),
                pinned: pin.as_deref() == Some(v.version.as_str()),
                stored: has(&v.version),
                needed_by: wanted
                    .iter()
                    .filter(|(_, needs)| compare_versions(&v.version, needs) == Ordering::Less)
                    .cloned()
                    .collect(),
            })
            .collect())
    }

    /// Pins `profile` to a version of a package, older or newer than the one it uses,
    /// installing it with its dependencies. Other profiles keep theirs. A version older than
    /// another package of the profile needs is refused unless `choice` says what to do.
    pub fn ts_install_version(
        &self,
        id: &str,
        full_name: &str,
        version: &str,
        profile: &str,
        choice: PinConflictChoice,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        let (packages, _) = self.ts_packages(id, false)?;
        let pkg = packages
            .iter()
            .find(|p| p.full_name.eq_ignore_ascii_case(full_name))
            .ok_or_else(|| {
                CoreError::Invalid(format!("{full_name} is not on Thunderstore for this game"))
            })?;
        if pkg.version(version).is_none() {
            return Err(CoreError::Invalid(format!(
                "{full_name} has no version {version}"
            )));
        }
        let wants = [Want::new(
            &pkg.full_name,
            Some(version.to_string()),
            PinIntent::Pin,
        )];
        self.install_with(id, &wants, profile, choice, progress, None)
    }

    /// Installs packages each pinned in `profile` at the version given (an imported
    /// profile's), as they are, without checking them against each other.
    pub(crate) fn ts_install_pinned(
        &self,
        id: &str,
        wanted: &[(String, String)],
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        let wants: Vec<Want> = wanted
            .iter()
            .map(|(n, v)| Want::new(n, Some(v.clone()), PinIntent::Pin))
            .collect();
        self.install_with(
            id,
            &wants,
            profile,
            PinConflictChoice::Ignore,
            progress,
            None,
        )
    }

    /// The stated dependencies of the profile's enabled Thunderstore packages that it does not
    /// meet, at the versions it uses. Only a warning: some packages state dependencies they
    /// do not need. A dependency on a mod loader is met by any enabled loader pack (one pack
    /// per game; mods often name another). Empty unless the game is set up with its loader
    /// from Thunderstore.
    pub fn ts_dependency_issues(&self, id: &str, profile: &str) -> Result<Vec<DepIssue>> {
        let def = self.def(id)?.def;
        if !def.loader.as_ref().is_some_and(|l| l.from_thunderstore()) {
            return Ok(Vec::new());
        }
        let game = self.data.game(id);
        let prof = umm_store::load_profile(&game, profile)?;
        let stored = self.ts_stored(id)?;
        let by_name: HashMap<String, &Stored> = stored
            .iter()
            .map(|s| (s.latest.full_name.to_lowercase(), s))
            .collect();
        let entry = |mod_id: &str| prof.mods.iter().find(|e| e.id == mod_id);
        // The package version an entry uses: its pin, else the latest.
        let used = |s: &Stored| -> ThunderstoreMeta {
            match entry(&s.mod_id).and_then(|e| e.version.as_deref()) {
                Some(v) if v != s.latest.version => s
                    .others
                    .iter()
                    .find(|o| o.version == v)
                    .cloned()
                    .unwrap_or_else(|| s.latest.clone()),
                _ => s.latest.clone(),
            }
        };
        let eco = self.ecosystem(false).ok();
        let is_loader = |name: &str| {
            !umm_gamedef::is_companion_loader(name)
                && (eco.as_ref().is_some_and(|e| e.modloader(name).is_some())
                    || by_name
                        .get(&name.to_lowercase())
                        .is_some_and(|s| s.latest.loader))
        };
        let has_loader = stored.iter().any(|s| {
            s.latest.loader
                && !umm_gamedef::is_companion_loader(&s.latest.full_name)
                && entry(&s.mod_id).is_some_and(|e| e.enabled)
        });
        let mut issues = Vec::new();
        for e in prof.mods.iter().filter(|e| e.enabled) {
            let Some(s) = stored.iter().find(|s| s.mod_id == e.id) else {
                continue;
            };
            for d in used(s).dependencies {
                let Some((name, needs)) = split_dependency(&d) else {
                    continue;
                };
                if is_loader(name) && has_loader {
                    continue;
                }
                let problem = match by_name.get(&name.to_lowercase()) {
                    None => DepProblem::NotInstalled,
                    Some(dep) => match entry(&dep.mod_id) {
                        None => DepProblem::NotInProfile,
                        Some(de) if !de.enabled => DepProblem::Disabled,
                        Some(_) => {
                            let have = used(dep).version;
                            if compare_versions(&have, needs) != Ordering::Less {
                                continue;
                            }
                            DepProblem::TooOld(have)
                        }
                    },
                };
                issues.push(DepIssue {
                    mod_id: e.id.clone(),
                    dependency: by_name
                        .get(&name.to_lowercase())
                        .map_or(name.to_string(), |s| s.latest.full_name.clone()),
                    needs: needs.to_string(),
                    problem,
                });
            }
        }
        Ok(issues)
    }

    /// Names of installed packages that depend on the mod `mod_id`.
    pub fn ts_dependents(&self, id: &str, mod_id: &str) -> Result<Vec<String>> {
        let installed = self.ts_installed(id)?;
        let Some(target) = installed.iter().find(|p| p.mod_id == mod_id) else {
            return Ok(Vec::new());
        };
        let prefix = format!("{}-", target.meta.full_name.to_lowercase());
        Ok(installed
            .iter()
            .filter(|p| p.mod_id != mod_id)
            .filter(|p| {
                p.meta
                    .dependencies
                    .iter()
                    .any(|d| d.to_lowercase().starts_with(&prefix))
            })
            .map(|p| p.meta.full_name.clone())
            .collect())
    }

    /// Stores a downloaded package version: as the package's latest (`as_latest`; the
    /// version it replaces is kept for profiles pinned to it), or beside it.
    #[allow(clippy::too_many_arguments)]
    fn store_package(
        &self,
        id: &str,
        community: &str,
        m: SteamMatch,
        eco: &Ecosystem,
        pkg: &Package,
        version: &str,
        zip: &[u8],
        as_latest: bool,
    ) -> Result<()> {
        let game = self.data.game(id);
        let mod_id = package_mod_id(&pkg.full_name);
        if let Ok(existing) = umm_store::load_mod(&game, &mod_id) {
            let same = existing
                .thunderstore
                .as_ref()
                .is_some_and(|t| t.full_name.eq_ignore_ascii_case(&pkg.full_name));
            if !same {
                return Err(CoreError::Invalid(format!(
                    "a mod with the id `{mod_id}` is already stored and is not {}; remove or rename it first",
                    pkg.full_name
                )));
            }
        }
        let manifest = layout::manifest(zip)?.unwrap_or_default();
        let loader = eco.modloader_for(&pkg.full_name, m.loader());
        let files = layout::plan(zip, &pkg.full_name, m.r2, loader)?;
        let staging = game.staging_dir().join(fsutil::unique_suffix());
        let result = (|| -> Result<()> {
            layout::write(&files, &staging)?;
            let opts = ImportOptions {
                id: Some(mod_id.clone()),
                name: Some(pkg.name.replace('_', " ")),
                version: Some(version.to_string()),
                source: Some(format!(
                    "https://thunderstore.io/c/{community}/p/{}/{}/",
                    pkg.owner, pkg.name
                )),
                unwrap: false,
                keep_top_dirs: Vec::new(),
                layout: Layout::Game,
                thunderstore: Some(ThunderstoreMeta {
                    community: community.to_string(),
                    full_name: pkg.full_name.clone(),
                    version: version.to_string(),
                    dependencies: manifest.dependencies.clone(),
                    description: Some(pkg.description.clone()).filter(|d| !d.is_empty()),
                    website: Some(pkg.website.clone()).filter(|w| !w.is_empty()),
                    loader: loader.is_some(),
                }),
                nexus: None,
                gamebanana: None,
                replace: true,
                keep_replaced: true,
                package_files: layout::package_files(zip)?,
            };
            if as_latest {
                umm_store::import_mod(&game, &staging, &opts)?;
            } else {
                umm_store::versions::store_version(&game, &staging, &opts)?;
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result
    }
}

/// What installing a package does to a profile's pin on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinIntent {
    /// Leaves it as it is: a pinned package stays at its version.
    Keep,
    /// Follows the latest installed version (an update in this profile).
    Follow,
    /// Pins it to the version installed (one picked from its versions, or an imported
    /// profile's).
    Pin,
}

/// What to do when a pinned package is older than another package of the profile needs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PinConflictChoice {
    /// Refuse, naming the conflicts ([`CoreError::PinConflicts`]), for the user to choose.
    #[default]
    Ask,
    /// Move each package that needs a newer version to its newest version that works with
    /// the pin, pinned there.
    Downgrade,
    /// Remove the pins in the way, so those packages follow their latest version.
    Unpin,
    /// Install as asked, conflicts and all (an imported profile, as r2modman had it).
    Ignore,
}

/// A pinned package older than another package of the profile needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinConflict {
    /// The package needing a newer version (`Owner-Name`), at the version the profile uses.
    pub dependent: String,
    pub dependent_version: String,
    /// The pinned package, its pin, and the version the dependent names.
    pub pinned: String,
    pub pin: String,
    pub needs: String,
    /// The pin is one the profile had, not one being set now, so removing it is a way out.
    pub can_unpin: bool,
}

/// The conflicts, for a message.
pub fn describe_conflicts(c: &[PinConflict]) -> String {
    c.iter()
        .map(|c| {
            format!(
                "{} {} needs {} {} or later, but this profile is pinned to {}",
                c.dependent, c.dependent_version, c.pinned, c.needs, c.pin
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A stated dependency a profile does not meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepIssue {
    /// The package stating it, by mod id.
    pub mod_id: String,
    /// The dependency, `Owner-Name`, and the version it names.
    pub dependency: String,
    pub needs: String,
    pub problem: DepProblem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepProblem {
    /// No version of it is installed.
    NotInstalled,
    /// Installed, but not in the profile.
    NotInProfile,
    /// In the profile, turned off.
    Disabled,
    /// The version the profile uses (given) is older than named.
    TooOld(String),
}

/// A package to install, and what it does to the profile's pin on it.
#[derive(Debug, Clone)]
pub(crate) struct Want {
    name: String,
    version: Option<String>,
    pin: PinIntent,
}

impl Want {
    fn new(name: &str, version: Option<String>, pin: PinIntent) -> Self {
        Want {
            name: name.to_string(),
            version,
            pin,
        }
    }
}

/// An installed package: its latest stored version, and the others kept for pinned profiles.
struct Stored {
    mod_id: String,
    latest: ThunderstoreMeta,
    others: Vec<ThunderstoreMeta>,
}
