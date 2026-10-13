// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! フォント。探し方は `zwsdk::fonts`（層のプラグインと同じもの）、登録は [`crate::draw::font_context`] で、
//! BML の丸ゴシック・角ゴシック（[`Bml`]）を足す。

use std::cell::RefCell;
use std::sync::{Arc, RwLock};

use parley::FontContext;
use parley::fontique::{
    Blob, Collection, CollectionOptions, FontInfoOverride, FontStyle, FontWeight,
};

pub use zwsdk::fonts::*;

pub mod choose;

thread_local! {
    static MADE: RefCell<Option<(u64, FontContext)>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct Face {
    pub bytes: Arc<Vec<u8>>,
    pub variable: bool,
    pub name: String,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("Face")
            .field("name", &self.name)
            .field("len", &self.bytes.len())
            .field("variable", &self.variable)
            .finish()
    }
}

/// BML のフォント。太い物が無ければ、そのフォントの太字は疑似太字（一画素ずらして二回レンダリング、`bml::document::html`）。
#[derive(Debug, Clone, Default)]
pub struct Bml {
    pub round: Option<Face>,
    pub round_bold: Option<Face>,
    pub square: Option<Face>,
    pub square_bold: Option<Face>,
}

impl Bml {
    pub fn bold(&self) -> (bool, bool) {
        let has = |regular: &Option<Face>, bold: &Option<Face>| {
            regular
                .as_ref()
                .is_some_and(|face| face.variable || bold.is_some())
        };
        (
            has(&self.round, &self.round_bold),
            has(&self.square, &self.square_bold),
        )
    }
}

static BML: RwLock<(u64, Option<Bml>)> = RwLock::new((0, None));

/// BML のフォントを決める。太字を太いフォントでレンダリングするか疑似太字にするかも、ここで HTML の側へ伝える。
pub fn set_bml(bml: Bml) {
    let (round, square) = bml.bold();
    bml::document::html::set_bold_faces(round, square);
    let mut slot = BML.write().unwrap_or_else(|e| e.into_inner());
    slot.0 += 1;
    slot.1 = Some(bml);
}

/// Blitz に渡す `FontContext`。フォントが同じなら一度だけ作ってコピーを返す。
pub fn context(faces: &Faces) -> FontContext {
    let (generation, bml) = BML.read().unwrap_or_else(|e| e.into_inner()).clone();
    let mark = mark(faces) ^ generation.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    MADE.with(|made| {
        let mut made = made.borrow_mut();
        if let Some((had, context)) = made.as_ref()
            && *had == mark
        {
            return context.clone();
        }
        let mut context = crate::draw::font_context(faces);
        if let Some(bml) = &bml {
            register_bml(&mut context.collection, bml);
        }
        *made = Some((mark, context.clone()));
        context
    })
}

fn register_bml(collection: &mut Collection, bml: &Bml) {
    use bml::document::html::{ROUND, SQUARE};
    for (name, regular, bold) in [
        (ROUND, &bml.round, &bml.round_bold),
        (SQUARE, &bml.square, &bml.square_bold),
    ] {
        let Some(regular) = regular else {
            continue;
        };
        register(collection, regular, name, 400.0);
        if let Some(bold) = bold.as_ref().filter(|_| !regular.variable) {
            register(collection, bold, name, 700.0);
        }
    }
}

fn register(collection: &mut Collection, face: &Face, name: &str, weight: f32) {
    let blob = Blob::new(Arc::clone(&face.bytes) as Arc<dyn AsRef<[u8]> + Send + Sync>);
    collection.register_fonts(
        blob,
        Some(FontInfoOverride {
            family_name: Some(name),
            style: Some(FontStyle::Normal),
            weight: (!face.variable).then(|| FontWeight::new(weight)),
            ..Default::default()
        }),
    );
}

/// フォントファイル（TTC なら中の一つ）から、細い物（`bold` なら太い物）を選んで抜き出す。
/// 太い物を求めて太い物が無ければ `None`。
pub fn pick(bytes: &[u8], bold: bool, prefer: Option<&str>) -> Option<Face> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    let shared = Arc::new(bytes.to_vec());
    let blob = Blob::new(Arc::clone(&shared) as Arc<dyn AsRef<[u8]> + Send + Sync>);
    let found = collection.register_fonts(blob, None);
    let target = if bold { 700.0 } else { 400.0 };
    let mut faces: Vec<(String, u32, f32, bool)> = Vec::new();
    for (family, infos) in found {
        let name = collection.family_name(family).unwrap_or("").to_owned();
        for info in infos {
            if info.style() != FontStyle::Normal {
                continue;
            }
            faces.push((
                name.clone(),
                info.index(),
                info.weight().value(),
                info.has_weight_axis(),
            ));
        }
    }
    let best = faces.into_iter().min_by(|a, b| {
        let named = |face: &(String, u32, f32, bool)| {
            !prefer.is_some_and(|prefer| face.0.eq_ignore_ascii_case(prefer))
        };
        let off = |face: &(String, u32, f32, bool)| {
            if face.3 { 0.0 } else { (face.2 - target).abs() }
        };
        named(a)
            .cmp(&named(b))
            .then(off(a).total_cmp(&off(b)))
            .then(a.1.cmp(&b.1))
    })?;
    let (name, index, weight, variable) = best;
    if bold && !variable && weight < 600.0 {
        return None;
    }
    let bytes = if is_collection(bytes) {
        Arc::new(single_face(bytes, index)?)
    } else {
        shared
    };
    Some(Face {
        bytes,
        variable,
        name,
    })
}

fn is_collection(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ttcf")
}

fn mark(faces: &Faces) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |b: u8| hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    for face in [&faces.sans, &faces.mono] {
        match face {
            None => eat(0),
            Some(bytes) => {
                bytes.len().to_le_bytes().into_iter().for_each(&mut eat);
                bytes.iter().step_by(4096).copied().for_each(&mut eat);
                bytes.last().copied().into_iter().for_each(&mut eat);
            }
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_regular_and_bold_faces_are_picked_by_weight() {
        let read = |name: &str| std::fs::read(format!("C:\\Windows\\Fonts\\{name}")).ok();
        let (Some(regular), Some(bold)) = (read("BIZ-UDGothicR.ttc"), read("BIZ-UDGothicB.ttc"))
        else {
            return;
        };
        let face = pick(&regular, false, Some("BIZ UDGothic")).expect("細い物");
        assert_eq!(face.name, "BIZ UDGothic");
        assert!(!is_collection(&face.bytes), "一つに抜き出す");
        assert!(
            pick(&regular, true, None).is_none(),
            "細い物しか無いファイルに太字は無い"
        );
        let face = pick(&bold, true, Some("BIZ UDPGothic")).expect("太い物");
        assert_eq!(face.name, "BIZ UDPGothic", "名前で先に選ぶ");
    }

    #[test]
    fn a_family_without_a_regular_face_has_no_bold_either() {
        let face = Face {
            bytes: Arc::new(Vec::new()),
            variable: false,
            name: String::new(),
        };
        let bml = Bml {
            round: Some(face.clone()),
            round_bold: Some(face.clone()),
            square: None,
            square_bold: Some(face.clone()),
        };
        assert_eq!(bml.bold(), (true, false));
        let variable = Bml {
            square: Some(Face {
                variable: true,
                ..face
            }),
            ..Bml::default()
        };
        assert_eq!(
            variable.bold(),
            (false, true),
            "太さの軸があれば一つで太字もレンダリングできる"
        );
    }
}
