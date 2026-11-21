//! A community's package list. Thunderstore publishes it as a gzipped JSON list of chunk URLs,
//! each chunk a gzipped JSON array of packages with every version. UMM keeps a compact copy
//! (only the fields it uses) per community, refreshed when over an hour old.

use crate::{cache, Http, Result, TsError, BASE_URL};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

pub const MAX_AGE: Duration = Duration::from_secs(3600);

/// One package, all versions, newest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub owner: String,
    pub name: String,
    /// `Owner-Name`.
    pub full_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub website: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub rating: i64,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub updated: String,
    pub versions: Vec<PackageVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersion {
    pub version: String,
    /// `Owner-Name-1.2.3`. Empty in lists read from Thunderstore (see `From<RawPackage>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub size: u64,
}

impl Package {
    pub fn latest(&self) -> Option<&PackageVersion> {
        self.versions.first()
    }
    pub fn version(&self, v: &str) -> Option<&PackageVersion> {
        self.versions.iter().find(|x| x.version == v)
    }
    pub fn download_url(&self, version: &str) -> String {
        format!(
            "{BASE_URL}/package/download/{}/{}/{version}/",
            self.owner, self.name
        )
    }
}

/// The listing as Thunderstore serves it; only what UMM keeps.
#[derive(Deserialize)]
struct RawPackage {
    name: String,
    full_name: String,
    owner: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    rating_score: i64,
    #[serde(default)]
    is_deprecated: bool,
    #[serde(default)]
    has_nsfw_content: bool,
    #[serde(default)]
    is_pinned: bool,
    #[serde(default)]
    date_updated: String,
    #[serde(default)]
    versions: Vec<RawVersion>,
}

#[derive(Deserialize)]
struct RawVersion {
    version_number: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    website_url: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    file_size: u64,
}

impl From<RawPackage> for Package {
    fn from(r: RawPackage) -> Self {
        let mut versions: Vec<&RawVersion> = r.versions.iter().collect();
        versions.sort_by(|a, b| compare_versions(&b.version_number, &a.version_number));
        let latest = versions.first();
        Package {
            description: latest.map(|v| v.description.clone()).unwrap_or_default(),
            icon: latest.map(|v| v.icon.clone()).unwrap_or_default(),
            website: latest.map(|v| v.website_url.clone()).unwrap_or_default(),
            downloads: r.versions.iter().map(|v| v.downloads).sum(),
            // Dependencies are not kept: modpacks list hundreds in each of hundreds of
            // versions, which made a big community's cache over 100 MB. Installing reads them
            // from each package's own manifest instead.
            versions: versions
                .iter()
                .map(|v| PackageVersion {
                    version: v.version_number.clone(),
                    dependencies: Vec::new(),
                    size: v.file_size,
                })
                .collect(),
            owner: r.owner,
            name: r.name,
            full_name: r.full_name,
            categories: r.categories,
            rating: r.rating_score,
            deprecated: r.is_deprecated,
            nsfw: r.has_nsfw_content,
            pinned: r.is_pinned,
            updated: r.date_updated,
        }
    }
}

/// Compares `1.10.0` above `1.9.2`. Non-numeric parts compare as text.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let mut pa = a.split('.');
    let mut pb = b.split('.');
    loop {
        match (pa.next(), pb.next()) {
            (None, None) => return Ordering::Equal,
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (Some(x), Some(y)) => {
                let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    _ => x.cmp(y),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// Splits `Owner-Name-1.2.3` into `Owner-Name` and `1.2.3`.
pub fn split_dependency(dep: &str) -> Option<(&str, &str)> {
    let (full, version) = dep.rsplit_once('-')?;
    (full.contains('-') && version.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .then_some((full, version))
}

fn gunzip(bytes: &[u8], what: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|e| TsError::Data {
            what: what.into(),
            message: e.to_string(),
        })?;
    Ok(out)
}

/// Bytes that may or may not be gzipped (the CDN serves them gzipped).
fn maybe_gunzip(bytes: Vec<u8>, what: &str) -> Result<Vec<u8>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        gunzip(&bytes, what)
    } else {
        Ok(bytes)
    }
}

fn json<T: serde::de::DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| TsError::Data {
        what: what.into(),
        message: e.to_string(),
    })
}

/// Downloads a community's whole package list from its index URL.
pub fn download(http: &dyn Http, index_url: &str) -> Result<Vec<Package>> {
    let index = maybe_gunzip(http.get(index_url)?, "package index")?;
    let chunks: Vec<String> = json(&index, "package index")?;
    let results: Vec<Result<Vec<Package>>> = std::thread::scope(|s| {
        // A few at a time: Lethal Company has 26 chunks of about 700 KB.
        let mut out = Vec::new();
        for group in chunks.chunks(6) {
            let handles: Vec<_> = group
                .iter()
                .map(|url| {
                    s.spawn(move || -> Result<Vec<Package>> {
                        let body = maybe_gunzip(http.get(url)?, "package list")?;
                        let raw: Vec<RawPackage> = json(&body, "package list")?;
                        Ok(raw.into_iter().map(Package::from).collect())
                    })
                })
                .collect();
            for h in handles {
                out.push(h.join().unwrap_or_else(|_| {
                    Err(TsError::Invalid("a download thread panicked".into()))
                }));
            }
        }
        out
    });
    let mut all = Vec::new();
    for r in results {
        all.extend(r?);
    }
    Ok(all)
}

/// A package looked up by name, whatever community lists it, with how each community that
/// lists it reviewed it (`approved`, `unreviewed`, `rejected`).
#[derive(Debug, Clone)]
pub struct FetchedPackage {
    pub package: Package,
    /// `(community, review status)`.
    pub listings: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct ApiPackage {
    name: String,
    full_name: String,
    owner: String,
    #[serde(default)]
    rating_score: i64,
    #[serde(default)]
    is_deprecated: bool,
    #[serde(default)]
    is_pinned: bool,
    #[serde(default)]
    date_updated: String,
    /// Thunderstore answers -1 when it has no count.
    #[serde(default)]
    total_downloads: i64,
    latest: ApiVersion,
    #[serde(default)]
    community_listings: Vec<ApiListing>,
}

#[derive(Deserialize)]
struct ApiVersion {
    version_number: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    website_url: String,
}

#[derive(Deserialize)]
struct ApiListing {
    community: String,
    #[serde(default)]
    review_status: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    has_nsfw_content: bool,
}

/// Looks up `Owner-Name` on Thunderstore's package API, which knows every package, not only
/// those a community lists. Only the latest version is known; any other can still be
/// downloaded by number.
pub fn fetch_package(http: &dyn Http, full_name: &str) -> Result<FetchedPackage> {
    let (owner, name) = full_name.split_once('-').ok_or_else(|| {
        TsError::Invalid(format!("`{full_name}` is not a package name (Owner-Name)"))
    })?;
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok(owner) || !ok(name) {
        return Err(TsError::Invalid(format!(
            "`{full_name}` is not a package name (Owner-Name)"
        )));
    }
    let body = http.get(&format!("{BASE_URL}/api/experimental/package/{owner}/{name}/"))?;
    let p: ApiPackage = json(&body, "package")?;
    let first = p.community_listings.first();
    Ok(FetchedPackage {
        listings: p
            .community_listings
            .iter()
            .map(|l| (l.community.clone(), l.review_status.clone()))
            .collect(),
        package: Package {
            categories: first.map(|l| l.categories.clone()).unwrap_or_default(),
            nsfw: p.community_listings.iter().any(|l| l.has_nsfw_content),
            description: p.latest.description,
            icon: p.latest.icon,
            website: p.latest.website_url,
            versions: vec![PackageVersion {
                version: p.latest.version_number,
                dependencies: Vec::new(),
                size: 0,
            }],
            owner: p.owner,
            name: p.name,
            full_name: p.full_name,
            rating: p.rating_score,
            downloads: p.total_downloads.max(0) as u64,
            deprecated: p.is_deprecated,
            pinned: p.is_pinned,
            updated: p.date_updated,
        },
    })
}

/// The community's packages from `cache_file` (gzipped compact JSON), downloaded when the copy
/// is missing, over an hour old, or `refresh` is set; the cached copy is used when offline.
pub fn load(
    http: &dyn Http,
    index_url: &str,
    cache_file: &Path,
    refresh: bool,
) -> Result<(Vec<Package>, cache::Freshness)> {
    let (bytes, fresh) = cache::cached(cache_file, MAX_AGE, refresh, || {
        let packages = download(http, index_url)?;
        let mut enc = GzEncoder::new(Vec::new(), flate2::Compression::default());
        let body = serde_json::to_vec(&packages).map_err(|e| TsError::Invalid(e.to_string()))?;
        enc.write_all(&body)
            .and_then(|_| enc.finish())
            .map_err(|e| TsError::Invalid(e.to_string()))
    })?;
    let body = gunzip(&bytes, "cached package list")?;
    Ok((json(&body, "cached package list")?, fresh))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::http::FakeHttp;

    pub fn gz(s: &str) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(s.as_bytes()).unwrap();
        e.finish().unwrap()
    }

    pub const CHUNK: &str = r#"[
      {"name": "BepInExPack", "full_name": "BepInEx-BepInExPack", "owner": "BepInEx",
       "categories": ["Libraries"], "rating_score": 900, "is_deprecated": false, "has_nsfw_content": false,
       "is_pinned": true, "date_updated": "2026-01-01",
       "versions": [
         {"version_number": "5.4.2100", "description": "old", "dependencies": [], "downloads": 5, "file_size": 10},
         {"version_number": "5.4.2305", "description": "BepInEx pack", "icon": "i.png", "dependencies": [], "downloads": 7, "file_size": 11}
       ]},
      {"name": "MoreThings", "full_name": "Ann-MoreThings", "owner": "Ann", "rating_score": 3,
       "versions": [{"version_number": "1.10.0", "description": "More", "dependencies": ["BepInEx-BepInExPack-5.4.2100"]},
                    {"version_number": "1.9.2", "dependencies": []}]}
    ]"#;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("1.10.0", "1.9.2"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("2.0.0", "2.0.0"), Ordering::Equal);
        assert_eq!(
            split_dependency("BepInEx-BepInExPack-5.4.2100"),
            Some(("BepInEx-BepInExPack", "5.4.2100"))
        );
        assert_eq!(
            split_dependency("Some-Long-Name_X-1.0.0"),
            Some(("Some-Long-Name_X", "1.0.0"))
        );
        assert_eq!(split_dependency("nonsense"), None);
    }

    #[test]
    fn downloads_chunks_and_caches_a_compact_copy() {
        let http = FakeHttp::default()
            .with("https://ts/index", gz(r#"["https://cdn/c1"]"#))
            .with("https://cdn/c1", gz(CHUNK));
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("packages.json.gz");
        let (pkgs, fresh) = load(&http, "https://ts/index", &file, false).unwrap();
        assert_eq!(fresh, cache::Freshness::Fetched);
        assert_eq!(pkgs.len(), 2);
        let pack = &pkgs[0];
        assert_eq!(pack.latest().unwrap().version, "5.4.2305", "newest first");
        assert_eq!(pack.description, "BepInEx pack");
        assert_eq!(pack.downloads, 12);
        assert_eq!(
            pack.download_url("5.4.2305"),
            "https://thunderstore.io/package/download/BepInEx/BepInExPack/5.4.2305/"
        );
        assert_eq!(pkgs[1].latest().unwrap().version, "1.10.0");

        let (again, fresh) = load(&FakeHttp::default(), "https://ts/index", &file, false).unwrap();
        assert_eq!(fresh, cache::Freshness::Cached);
        assert_eq!(again.len(), 2);
    }

    #[test]
    fn looks_up_a_package_outside_any_list() {
        let http = FakeHttp::default().with(
            "https://thunderstore.io/api/experimental/package/BepInEx/BepInExPack_IL2CPP/",
            r#"{"namespace":"BepInEx","name":"BepInExPack_IL2CPP","full_name":"BepInEx-BepInExPack_IL2CPP",
                "owner":"BepInEx","rating_score":5,"is_deprecated":false,"total_downloads":-1,
                "latest":{"version_number":"6.0.788","description":"BepInEx 6 IL2CPP","icon":"i.png","website_url":""},
                "community_listings":[{"community":"among-us","review_status":"rejected","categories":[]},
                                      {"community":"megabonk","review_status":"unreviewed","categories":["Libraries"]}]}"#,
        );
        let f = fetch_package(&http, "BepInEx-BepInExPack_IL2CPP").unwrap();
        assert_eq!(f.package.latest().unwrap().version, "6.0.788");
        assert_eq!(
            f.listings,
            [
                ("among-us".to_string(), "rejected".to_string()),
                ("megabonk".to_string(), "unreviewed".to_string())
            ]
        );
        assert!(fetch_package(&http, "../evil").is_err());
        assert!(fetch_package(&http, "NoDash").is_err());
    }
}
