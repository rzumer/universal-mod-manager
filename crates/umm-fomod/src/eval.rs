//! Running an installer: which steps show, what each option is, what is chosen, and which
//! files that installs.

use crate::*;
use std::collections::{BTreeMap, HashMap};

/// A file a `fileDependency` names, as the game folder has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Missing,
    /// There but not enabled (a game plugin listed as off).
    Inactive,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginState {
    pub kind: PluginType,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupState {
    pub plugins: Vec<PluginState>,
    /// What is wrong with the choice, if anything ("choose one option").
    pub problem: Option<String>,
    /// No choice was given for it: the installer's defaults apply.
    pub defaulted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepState {
    pub visible: bool,
    pub groups: Vec<GroupState>,
}

/// The installer run with a set of choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluated {
    pub steps: Vec<StepState>,
    /// The choices in effect, for the visible steps: what to keep.
    pub choices: Vec<Choice>,
    /// Files and folders to install, in order (later ones win on equal priority).
    pub files: Vec<FileItem>,
    pub flags: BTreeMap<String, String>,
    /// The mod's own requirements (`moduleDependencies`) do not hold.
    pub needs_unmet: bool,
}

impl Evaluated {
    /// Every visible group has a valid choice.
    pub fn complete(&self) -> bool {
        self.steps
            .iter()
            .filter(|s| s.visible)
            .all(|s| s.groups.iter().all(|g| g.problem.is_none()))
    }

    /// Some visible group had no choice given (a step the user has not seen).
    pub fn defaulted(&self) -> bool {
        self.steps
            .iter()
            .filter(|s| s.visible)
            .any(|s| s.groups.iter().any(|g| g.defaulted))
    }
}

impl Condition {
    fn holds(&self, flags: &BTreeMap<String, String>, files: &dyn Fn(&str) -> FileState) -> bool {
        match self {
            Condition::All(c) => c.iter().all(|c| c.holds(flags, files)),
            Condition::Any(c) => c.is_empty() || c.iter().any(|c| c.holds(flags, files)),
            // An unset flag reads as empty.
            Condition::Flag { name, value } => {
                flags.get(name).map(String::as_str).unwrap_or("") == value.as_str()
            }
            Condition::File { path, state } => files(path) == *state,
            Condition::Version => true,
        }
    }
}

impl Plugin {
    fn kind_now(
        &self,
        flags: &BTreeMap<String, String>,
        files: &dyn Fn(&str) -> FileState,
    ) -> PluginType {
        match &self.kind {
            PluginKind::Fixed(t) => *t,
            PluginKind::Dependent { default, patterns } => patterns
                .iter()
                .find(|(c, _)| c.holds(flags, files))
                .map(|(_, t)| *t)
                .unwrap_or(*default),
        }
    }
}

fn usable(t: PluginType) -> bool {
    t != PluginType::NotUsable
}

/// What a group's options are, and which are chosen.
fn select(
    group: &Group,
    types: &[PluginType],
    choice: Option<&Choice>,
) -> (Vec<bool>, Option<String>) {
    let n = group.plugins.len();
    let mut sel = vec![false; n];
    match choice {
        Some(c) => {
            for (i, p) in group.plugins.iter().enumerate() {
                if usable(types[i]) && c.plugins.iter().any(|x| x.eq_ignore_ascii_case(&p.name)) {
                    sel[i] = true;
                }
            }
        }
        None => {
            for (i, t) in types.iter().enumerate() {
                sel[i] = matches!(t, PluginType::Recommended);
            }
        }
    }
    for (i, t) in types.iter().enumerate() {
        if *t == PluginType::Required {
            sel[i] = true;
        }
    }
    let first_usable = types.iter().position(|t| usable(*t));
    let count = |s: &[bool]| s.iter().filter(|x| **x).count();
    match group.kind {
        GroupKind::All => {
            for (i, t) in types.iter().enumerate() {
                sel[i] = usable(*t);
            }
        }
        GroupKind::ExactlyOne | GroupKind::AtMostOne if count(&sel) > 1 => {
            // Keep the first chosen one (a required option, if any).
            let keep = types
                .iter()
                .enumerate()
                .position(|(i, t)| sel[i] && *t == PluginType::Required)
                .or_else(|| sel.iter().position(|x| *x));
            for (i, s) in sel.iter_mut().enumerate() {
                *s = Some(i) == keep;
            }
        }
        _ => {}
    }
    let must_pick = matches!(group.kind, GroupKind::ExactlyOne | GroupKind::AtLeastOne);
    if choice.is_none() && must_pick && count(&sel) == 0 {
        if let Some(i) = first_usable {
            sel[i] = true;
        }
    }
    let problem = match group.kind {
        GroupKind::ExactlyOne if count(&sel) != 1 && first_usable.is_some() => {
            Some("choose one option".to_string())
        }
        GroupKind::AtLeastOne if count(&sel) == 0 && first_usable.is_some() => {
            Some("choose at least one option".to_string())
        }
        _ => None,
    };
    (sel, problem)
}

impl Installer {
    /// Runs the installer with `choices` (by step, group and option name). Groups without a
    /// choice take the installer's defaults: required and recommended options, and the first
    /// usable one where one must be chosen. `files` says how the game folder has a file.
    pub fn evaluate(&self, choices: &[Choice], files: &dyn Fn(&str) -> FileState) -> Evaluated {
        let mut flags: BTreeMap<String, String> = BTreeMap::new();
        let mut steps = Vec::new();
        let mut kept = Vec::new();
        let mut out: Vec<FileItem> = self.required.clone();
        for step in &self.steps {
            let visible = step.visible.as_ref().is_none_or(|c| c.holds(&flags, files));
            let mut groups = Vec::new();
            let mut set = Vec::new();
            for group in &step.groups {
                // Options react to choices made above them in the same step, as installers
                // show them changing while the user picks.
                let mut now = flags.clone();
                now.extend(set.iter().cloned());
                let types: Vec<PluginType> = group
                    .plugins
                    .iter()
                    .map(|p| p.kind_now(&now, files))
                    .collect();
                let choice = choices.iter().find(|c| {
                    c.step.eq_ignore_ascii_case(&step.name)
                        && c.group.eq_ignore_ascii_case(&group.name)
                });
                let (sel, problem) = select(group, &types, choice);
                if visible {
                    for (i, p) in group.plugins.iter().enumerate() {
                        if sel[i] {
                            set.extend(p.flags.iter().cloned());
                            out.extend(p.files.iter().cloned());
                        } else {
                            out.extend(
                                p.files
                                    .iter()
                                    .filter(|f| f.always || (f.if_usable && usable(types[i])))
                                    .cloned(),
                            );
                        }
                    }
                    kept.push(Choice {
                        step: step.name.clone(),
                        group: group.name.clone(),
                        plugins: group
                            .plugins
                            .iter()
                            .zip(&sel)
                            .filter(|(_, s)| **s)
                            .map(|(p, _)| p.name.clone())
                            .collect(),
                    });
                }
                groups.push(GroupState {
                    plugins: types
                        .iter()
                        .zip(&sel)
                        .map(|(t, s)| PluginState {
                            kind: *t,
                            selected: *s && visible,
                        })
                        .collect(),
                    problem: problem.filter(|_| visible),
                    defaulted: choice.is_none(),
                });
            }
            // A step's flags count from the next step on.
            flags.extend(set);
            steps.push(StepState { visible, groups });
        }
        for c in &self.conditional {
            if c.when.holds(&flags, files) {
                out.extend(c.files.iter().cloned());
            }
        }
        Evaluated {
            needs_unmet: self.needs.as_ref().is_some_and(|n| !n.holds(&flags, files)),
            steps,
            choices: kept,
            files: out,
            flags,
        }
    }
}

/// One archive file and where it goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    /// Path in the archive, relative to the folder holding `fomod/`, as the archive spells it.
    pub source: String,
    /// Path relative to the game's mod folder.
    pub destination: String,
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a}/{b}"),
    }
}

/// The archive files `items` install, given the archive's files (relative to the folder
/// holding `fomod/`). Where two entries install to the same place, the higher priority wins,
/// then the later one. Returns the files in order, and the sources the archive lacks.
pub fn resolve(items: &[FileItem], archive: &[&str]) -> (Vec<Placed>, Vec<String>) {
    let lower: Vec<String> = archive.iter().map(|f| f.to_lowercase()).collect();
    let mut placed: Vec<(Placed, i64)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut missing = Vec::new();
    let mut put = |p: Placed, priority: i64, placed: &mut Vec<(Placed, i64)>| {
        let key = p.destination.to_lowercase();
        match at.get(&key) {
            Some(&i) if placed[i].1 > priority => {}
            Some(&i) => placed[i] = (p, priority),
            None => {
                at.insert(key, placed.len());
                placed.push((p, priority));
            }
        }
    };
    for item in items {
        let src = item.source.to_lowercase();
        if item.folder {
            let prefix = if src.is_empty() {
                String::new()
            } else {
                format!("{src}/")
            };
            let dest = item.destination.clone().unwrap_or_default();
            let mut any = false;
            for (i, f) in lower.iter().enumerate() {
                if let Some(rest) = f.strip_prefix(&prefix) {
                    // Keep the archive's spelling of the part below the folder.
                    let rest = &archive[i][archive[i].len() - rest.len()..];
                    if rest.to_lowercase().starts_with("fomod/") && src.is_empty() {
                        continue;
                    }
                    any = true;
                    put(
                        Placed {
                            source: archive[i].to_string(),
                            destination: join(&dest, rest),
                        },
                        item.priority,
                        &mut placed,
                    );
                }
            }
            if !any {
                missing.push(item.source.clone());
            }
        } else {
            match lower.iter().position(|f| *f == src) {
                Some(i) => put(
                    Placed {
                        source: archive[i].to_string(),
                        destination: item
                            .destination
                            .clone()
                            .unwrap_or_else(|| archive[i].to_string()),
                    },
                    item.priority,
                    &mut placed,
                ),
                None => missing.push(item.source.clone()),
            }
        }
    }
    (
        placed
            .into_iter()
            .map(|(p, _)| p)
            .filter(|p| !p.destination.is_empty())
            .collect(),
        missing,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <moduleName>Better Rocks</moduleName>
  <requiredInstallFiles><folder source="core" destination="" /></requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Resolution">
      <optionalFileGroups order="Explicit">
        <group name="Textures" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="2K">
              <description>Sharper.</description>
              <image path="fomod\images\2k.png" />
              <files><folder source="2K\textures" destination="textures" /></files>
              <conditionFlags><flag name="res">2k</flag></conditionFlags>
              <typeDescriptor><type name="Recommended" /></typeDescriptor>
            </plugin>
            <plugin name="1K">
              <description>Lighter.</description>
              <files><folder source="1K\textures" destination="textures" /></files>
              <conditionFlags><flag name="res">1k</flag></conditionFlags>
              <typeDescriptor><type name="Optional" /></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
    <installStep name="Extras">
      <visible><flagDependency flag="res" value="2k" /></visible>
      <optionalFileGroups>
        <group name="Moss" type="SelectAny">
          <plugins>
            <plugin name="Moss">
              <description>Moss on rocks.</description>
              <files><file source="extras\moss.dds" destination="textures\moss.dds" priority="2" /></files>
              <typeDescriptor>
                <dependencyType>
                  <defaultType name="Optional" />
                  <patterns><pattern>
                    <dependencies><fileDependency file="Lichen.esp" state="Active" /></dependencies>
                    <type name="NotUsable" />
                  </pattern></patterns>
                </dependencyType>
              </typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
  <conditionalFileInstalls><patterns>
    <pattern>
      <dependencies operator="And"><flagDependency flag="res" value="1k" /></dependencies>
      <files><file source="extras\lite.ini" destination="lite.ini" /></files>
    </pattern>
  </patterns></conditionalFileInstalls>
</config>"#;

    const ARCHIVE: &[&str] = &[
        "core/Rocks.esp",
        "2K/textures/rock.dds",
        "2K/textures/moss.dds",
        "1K/textures/rock.dds",
        "extras/moss.dds",
        "extras/lite.ini",
        "fomod/ModuleConfig.xml",
        "fomod/images/2k.png",
    ];

    fn installer() -> Installer {
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(XML.encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        Installer::parse(&utf16).unwrap()
    }

    fn dests(e: &Evaluated) -> Vec<(String, String)> {
        let (p, missing) = resolve(&e.files, ARCHIVE);
        assert!(missing.is_empty(), "{missing:?}");
        p.into_iter().map(|p| (p.source, p.destination)).collect()
    }

    #[test]
    fn reads_the_installer() {
        let i = installer();
        assert_eq!(i.name, "Better Rocks");
        assert_eq!(i.steps.len(), 2);
        let g = &i.steps[0].groups[0];
        assert_eq!(
            (g.name.as_str(), g.kind),
            ("Textures", GroupKind::ExactlyOne)
        );
        assert_eq!(g.plugins[0].image.as_deref(), Some("fomod/images/2k.png"));
        assert_eq!(g.plugins[0].flags, [("res".into(), "2k".into())]);
        assert!(i.steps[1].visible.is_some());
    }

    #[test]
    fn defaults_follow_the_installer() {
        let i = installer();
        let none = |_: &str| FileState::Missing;
        let e = i.evaluate(&[], &none);
        assert!(e.complete() && e.defaulted());
        // 2K is recommended; its flag shows the extras step; moss is optional, so not chosen.
        assert!(e.steps[1].visible);
        assert_eq!(
            dests(&e),
            [
                ("core/Rocks.esp".into(), "Rocks.esp".into()),
                ("2K/textures/rock.dds".into(), "textures/rock.dds".into()),
                ("2K/textures/moss.dds".into(), "textures/moss.dds".into()),
            ]
        );
    }

    #[test]
    fn choices_set_flags_steps_and_conditional_files() {
        let i = installer();
        let none = |_: &str| FileState::Missing;
        let pick = |step: &str, group: &str, p: &[&str]| Choice {
            step: step.into(),
            group: group.into(),
            plugins: p.iter().map(|s| s.to_string()).collect(),
        };
        // 1K hides the extras step and brings the conditional ini.
        let e = i.evaluate(&[pick("Resolution", "Textures", &["1K"])], &none);
        assert!(!e.steps[1].visible && !e.defaulted());
        let d = dests(&e);
        assert!(d.contains(&("1K/textures/rock.dds".into(), "textures/rock.dds".into())));
        assert!(d.contains(&("extras/lite.ini".into(), "lite.ini".into())));
        assert_eq!(e.choices.len(), 1, "hidden steps keep no choice");

        // Moss chosen: its higher priority beats the 2K folder's copy.
        let e = i.evaluate(
            &[
                pick("Resolution", "Textures", &["2K"]),
                pick("Extras", "Moss", &["moss"]),
            ],
            &none,
        );
        assert!(dests(&e).contains(&("extras/moss.dds".into(), "textures/moss.dds".into())));

        // With Lichen.esp active in the game, Moss cannot be used.
        let lichen = |f: &str| {
            if f == "Lichen.esp" {
                FileState::Active
            } else {
                FileState::Missing
            }
        };
        let e = i.evaluate(
            &[
                pick("Resolution", "Textures", &["2K"]),
                pick("Extras", "Moss", &["Moss"]),
            ],
            &lichen,
        );
        assert_eq!(e.steps[1].groups[0].plugins[0].kind, PluginType::NotUsable);
        assert!(!e.steps[1].groups[0].plugins[0].selected);

        // Nothing chosen where one option is needed.
        let e = i.evaluate(&[pick("Resolution", "Textures", &[])], &none);
        assert!(!e.complete());
        assert_eq!(
            e.steps[0].groups[0].problem.as_deref(),
            Some("choose one option")
        );
    }

    #[test]
    fn options_react_to_groups_above_them_in_the_step() {
        // As in Vortex's test installer: picking an option above makes one below required.
        let xml = r#"<config><moduleName>T</moduleName><installSteps><installStep name="S">
          <optionalFileGroups order="Explicit">
            <group name="A" type="SelectExactlyOne"><plugins order="Explicit">
              <plugin name="On"><description/><conditionFlags><flag name="a">On</flag></conditionFlags>
                <typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
              <plugin name="Off"><description/><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
            </plugins></group>
            <group name="Results" type="SelectAny"><plugins>
              <plugin name="A is on"><description/><typeDescriptor><dependencyType>
                <defaultType name="NotUsable"/>
                <patterns><pattern><dependencies><flagDependency flag="a" value="On"/></dependencies>
                  <type name="Required"/></pattern></patterns>
              </dependencyType></typeDescriptor></plugin>
            </plugins></group>
          </optionalFileGroups></installStep></installSteps></config>"#;
        let i = Installer::parse(xml.as_bytes()).unwrap();
        let none = |_: &str| FileState::Missing;
        let pick = |p: &str| {
            vec![Choice {
                step: "S".into(),
                group: "A".into(),
                plugins: vec![p.into()],
            }]
        };
        let on = i.evaluate(&pick("On"), &none);
        assert_eq!(
            on.steps[0].groups[1].plugins[0],
            PluginState {
                kind: PluginType::Required,
                selected: true
            }
        );
        let off = i.evaluate(&pick("Off"), &none);
        assert_eq!(
            off.steps[0].groups[1].plugins[0].kind,
            PluginType::NotUsable
        );
    }

    /// Real installers: every `*.xml` in `UMM_FOMOD_SAMPLES` must parse and run with the
    /// defaults. Run with `cargo test -p umm-fomod -- --ignored`.
    #[test]
    #[ignore]
    fn real_installers_parse_and_run() {
        let Ok(dir) = std::env::var("UMM_FOMOD_SAMPLES") else {
            return;
        };
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let bytes = std::fs::read(&p).unwrap();
            let i = Installer::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            let ev = i.evaluate(&[], &|_| FileState::Missing);
            let options: usize = i
                .steps
                .iter()
                .flat_map(|s| &s.groups)
                .map(|g| g.plugins.len())
                .sum();
            println!(
                "{:<70} {:<32} steps {:>2} (shown {:>2}) options {:>3} files {:>3} complete {}",
                p.file_name().unwrap().to_string_lossy(),
                i.name.chars().take(32).collect::<String>(),
                i.steps.len(),
                ev.steps.iter().filter(|s| s.visible).count(),
                options,
                ev.files.len(),
                ev.complete()
            );
            n += 1;
        }
        assert!(n > 0);
    }

    #[test]
    fn missing_sources_are_reported() {
        let item = FileItem {
            source: "nope".into(),
            destination: None,
            folder: false,
            priority: 0,
            always: false,
            if_usable: false,
        };
        assert_eq!(resolve(&[item], ARCHIVE).1, ["nope"]);
    }
}
