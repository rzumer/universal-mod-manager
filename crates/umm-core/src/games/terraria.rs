//! Terraria: it loads no mods by itself; tModLoader does, a game of its own on Steam (see
//! [`crate::games::tmodloader`]). UMM sets Terraria up as tModLoader when that is installed;
//! otherwise its draft says so.

use std::path::Path;
use umm_gamedef::steam::{Inspection, Note};
use umm_gamedef::{GameDef, GameKind, LoaderSetup, Recognised, Shape};

const EXE: &str = "Terraria.exe";

pub(crate) struct Terraria;

impl GameKind for Terraria {
    fn key(&self) -> &'static str {
        "terraria"
    }

    fn recognise(&self, _root: &Path, exes: &[String]) -> Option<Recognised> {
        umm_gamedef::kind::has_top_exe(exes, EXE).then_some(Recognised {
            key: "terraria",
            label: None,
            exe: None,
            own_mods: None,
        })
    }

    fn shape(&self, _root: &Path, _found: &Inspection, exe: &str, _setup: LoaderSetup) -> Shape {
        let mut shape = Shape::default();
        if exe.eq_ignore_ascii_case(EXE) {
            shape.notes.push(Note::new(
                "terraria",
                "Terraria loads no mods by itself: they need tModLoader, a free game of its own on \
                 Steam. Install it there, then remove Terraria from UMM and add it again: UMM sets \
                 it up with tModLoader.",
            ));
        }
        shape
    }

    /// No definition is Terraria's: without tModLoader it is a game like any other.
    fn applies(&self, _def: &GameDef) -> bool {
        false
    }

    fn own_mods(&self) -> Option<&'static str> {
        None
    }
}

impl crate::games::GameIntegration for Terraria {}
