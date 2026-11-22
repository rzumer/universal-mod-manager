//! Mods with a FOMOD installer: nothing deploys until the options are chosen; the chosen
//! files go under the suggested game folder; an update replays the choices while they fit.

use std::fs;
use std::path::Path;
use umm_core::{FomodChoice, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "rocks";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

fn utf16(s: &str) -> Vec<u8> {
    [0xFF, 0xFE]
        .into_iter()
        .chain(s.encode_utf16().flat_map(|u| u.to_le_bytes()))
        .collect()
}

fn config(group: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<config>
  <moduleName>Better Rocks</moduleName>
  <requiredInstallFiles><file source="core\Rocks.esp" destination="Rocks.esp" /></requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Resolution">
      <optionalFileGroups>
        <group name="{group}" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="2K">
              <description>Sharper.</description>
              <image path="fomod\images\2k.png" />
              <files><folder source="2K" destination="" /></files>
              <typeDescriptor><type name="Recommended" /></typeDescriptor>
            </plugin>
            <plugin name="1K">
              <description>Lighter.</description>
              <files><folder source="1K" destination="" /></files>
              <typeDescriptor><type name="Optional" /></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
</config>"#
    )
}

fn mod_src(dir: &Path, group: &str, rock: &str) {
    let m = dir.join("Better Rocks");
    write(&m.join("fomod/ModuleConfig.xml"), &utf16(&config(group)));
    write(&m.join("fomod/images/2k.png"), b"\x89PNG\r\n");
    write(&m.join("core/Rocks.esp"), b"esp");
    write(
        &m.join("2K/textures/rock.dds"),
        format!("2k {rock}").as_bytes(),
    );
    write(
        &m.join("1K/textures/rock.dds"),
        format!("1k {rock}").as_bytes(),
    );
}

fn import(s: &Session, src: &Path, replace: bool) -> umm_core::ModMeta {
    s.import(
        GAME,
        src,
        ImportOptions {
            id: Some("better-rocks".into()),
            unwrap: true,
            replace,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap()
}

fn targets(s: &Session) -> Vec<String> {
    s.plan(GAME, DEFAULT_PROFILE)
        .unwrap()
        .files
        .iter()
        .map(|f| f.target.to_string())
        .collect()
}

#[test]
fn installer_choices_decide_the_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Rocks.exe"), b"exe");
    write(&root.join("Data/textures/stone.dds"), b"stone");
    write(&root.join("Data/Skyrim.esm"), b"esm");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        b"schema = 1\nname = \"Rocks\"\n[detect]\nmarker = \"Rocks.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path(GAME, &root).unwrap();

    // Imported: the installer is found inside the wrapper; nothing deploys yet.
    let v1 = tmp.path().join("v1");
    mod_src(&v1, "Textures", "v1");
    let m = import(&s, &v1, false);
    let f = m.fomod.unwrap();
    assert_eq!((f.root.as_str(), f.done), ("Better Rocks", false));
    assert!(targets(&s).is_empty());
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    assert!(plan.skipped[0]
        .reason
        .contains("options have not been chosen"));

    // The dialog's view: defaults (2K recommended), files, and the suggested game folder.
    let v = s.fomod_view(GAME, "better-rocks", None, None).unwrap();
    assert_eq!(v.name, "Better Rocks");
    assert!(v.complete);
    let g = &v.steps[0].groups[0];
    assert_eq!(
        (g.options[0].name.as_str(), g.options[0].selected),
        ("2K", true)
    );
    assert_eq!(
        g.options[0].image.as_deref(),
        Some("Better Rocks/fomod/images/2k.png")
    );
    assert_eq!(v.suggested_target, "Data");
    assert!(v.files.contains(&(
        "Better Rocks/2K/textures/rock.dds".into(),
        "textures/rock.dds".into()
    )));
    assert!(s
        .mod_image(GAME, "better-rocks", "better rocks/fomod/images/2K.png")
        .unwrap()
        .starts_with("data:image/png;base64,"));

    // The user picks 1K.
    let one_k = vec![FomodChoice {
        step: "Resolution".into(),
        group: "Textures".into(),
        plugins: vec!["1K".into()],
    }];
    let preview = s
        .fomod_view(GAME, "better-rocks", Some(one_k.clone()), None)
        .unwrap();
    assert!(preview.files.contains(&(
        "Better Rocks/1K/textures/rock.dds".into(),
        "textures/rock.dds".into()
    )));
    s.fomod_apply(GAME, "better-rocks", one_k, v.suggested_target.clone())
        .unwrap();
    assert_eq!(targets(&s), ["Data/Rocks.esp", "Data/textures/rock.dds"]);
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    let rock = plan
        .files
        .iter()
        .find(|f| f.target.as_str() == "Data/textures/rock.dds")
        .unwrap();
    assert_eq!(fs::read_to_string(&rock.source).unwrap(), "1k v1");
    // Placement editing is for mods without an installer.
    assert!(s.placement_view(GAME, "better-rocks", None).is_err());

    // An update with the same installer: the choices are replayed.
    let v2 = tmp.path().join("v2");
    mod_src(&v2, "Textures", "v2");
    let m = import(&s, &v2, true);
    assert!(m.fomod.as_ref().unwrap().done);
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    let rock = plan
        .files
        .iter()
        .find(|f| f.target.as_str() == "Data/textures/rock.dds")
        .unwrap();
    assert_eq!(fs::read_to_string(&rock.source).unwrap(), "1k v2");

    // An update whose installer changed: asked again.
    let v3 = tmp.path().join("v3");
    mod_src(&v3, "Texture size", "v3");
    let m = import(&s, &v3, true);
    assert!(!m.fomod.as_ref().unwrap().done);
    assert!(targets(&s).is_empty());
}
