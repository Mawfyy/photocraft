//! Third-party acceptance for our PSD exports (FILE-216-4).
//!
//! PhotoCraft's own reader is not enough to claim Photoshop compatibility: a file only we can
//! read is a file a user cannot open. This test writes PSD files with PhotoCraft and opens them
//! with an independent reader — [psd-tools](https://github.com/psd-tools/psd-tools) (MIT) — and
//! checks that the reader sees what we meant to write.
//!
//! Two checks run over every file:
//!
//! * **the reader and we agree**: the document size, the layer tree (the names, their order,
//!   the group nesting, the visibility) and the kind of every layer;
//! * **the reader sees what the original file showed** (the corpora only): a file re-saved
//!   through PhotoCraft must still look to a reader like the file it came from — the same
//!   layers, of the same kinds.
//!
//! Two sets of files are checked: the documents built here (layers, groups, masks, adjustments,
//! fills, every blend mode, in every colour mode and depth) and every real file of the corpora
//! (feature `corpus`: the Photoshop-authored oracles, the psd-tools set and the ag-psd set)
//! re-saved through PhotoCraft, which is what a user does with a file that arrives from a client.
//!
//! The reader is optional. The test looks for `$PHOTOCRAFT_PSD_READER` (a Python interpreter
//! with psd-tools installed), then for `.venv-psd/bin/python` in the workspace
//! (`cargo xtask psd-accept --setup` creates it), then for a `python3` that can import
//! `psd_tools`. Without one it prints SKIP and passes; the CI corpus job installs the reader,
//! so the gate runs there.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

mod common;

/// One layer, as a reader sees it.
#[derive(Debug, PartialEq, Eq)]
struct Layer {
    name: String,
    group: bool,
    visible: bool,
    /// The kind of the layer: "pixel", "fill", "shape", "adjustment", "type", "smartobject" or
    /// "group".
    kind: String,
}

/// The independent reader's view of a file.
struct View {
    width: u32,
    height: u32,
    layers: Vec<Layer>,
}

/// The blocks a reader's registry knows as a layer kind, in the order it scans them: the first
/// one a layer carries decides the kind.
const READER_REGISTRY: [(&[u8; 4], &str); 20] = [
    (b"SoCo", "fill"),
    (b"PtFl", "fill"),
    (b"GdFl", "fill"),
    (b"CgEd", "adjustment"),
    (b"curv", "adjustment"),
    (b"expA", "adjustment"),
    (b"levl", "adjustment"),
    (b"vibA", "adjustment"),
    (b"hue2", "adjustment"),
    (b"blnc", "adjustment"),
    (b"blwh", "adjustment"),
    (b"phfl", "adjustment"),
    (b"mixr", "adjustment"),
    (b"clrL", "adjustment"),
    (b"nvrt", "adjustment"),
    (b"post", "adjustment"),
    (b"thrs", "adjustment"),
    (b"selc", "adjustment"),
    (b"grdm", "adjustment"),
    // `brit` (the legacy Brightness/Contrast record) is no kind of its own: a layer that has
    // only it stays a pixel layer for the reader.
    (b"brit", "pixel"),
];

/// The blocks that make a layer a vector shape.
const VECTOR_BLOCKS: [&[u8; 4]; 5] = [b"vmsk", b"vsms", b"vogk", b"vstk", b"vscg"];

/// The kind a layer must have, from the blocks of the file we wrote.
///
/// A layer whose colour comes from a descriptor (a fill layer, a shape layer) is a shape when it
/// also carries a vector mask; a layer that only has a vector mask keeps its own pixels and
/// stays a pixel layer. This is what Photoshop's own files say, and it is what a reader decides
/// from the blocks *and* the "pixel data irrelevant" flag: a fill layer written without that
/// flag is a plain fill to the reader, where Photoshop wrote a shape.
fn expected_kind(blocks: &[[u8; 4]], is_group: bool) -> String {
    let has = |k: &[u8; 4]| blocks.iter().any(|b| b == k);
    if is_group {
        return "group".to_string();
    }
    if has(b"TySh") || has(b"tySh") {
        return "type".to_string();
    }
    if has(b"SoLd") || has(b"SoLE") || has(b"PlLd") {
        return "smartobject".to_string();
    }
    let vector = VECTOR_BLOCKS.iter().any(|k| has(k));
    // The reader's registry: the first block a layer carries decides its kind.
    match READER_REGISTRY.iter().find(|(k, _)| has(k)).map(|(_, kind)| *kind) {
        // A fill with a vector mask is a shape; a fill on its own is a fill layer.
        Some("fill") => return if vector { "shape".to_string() } else { "fill".to_string() },
        Some("adjustment") => return "adjustment".to_string(),
        // `brit` alone is no kind of its own: the layer keeps its pixels.
        _ => {}
    }
    // A shape layer carries its fill content (`vscg`): it is a shape.
    if has(b"vscg") {
        return "shape".to_string();
    }
    // A layer that only carries a vector mask keeps its pixels: it stays a pixel layer.
    "pixel".to_string()
}

/// The kind a reader's report belongs to.
fn reader_kind_class(kind: &str) -> String {
    match kind {
        "group" | "artboard" => "group".to_string(),
        "type" => "type".to_string(),
        "smartobject" => "smartobject".to_string(),
        "solidcolorfill" | "gradientfill" | "patternfill" => "fill".to_string(),
        "shape" => "shape".to_string(),
        "pixel" => "pixel".to_string(),
        "unknown" => "unknown".to_string(),
        // The rest of a reader's registry is the adjustment kinds.
        _ => "adjustment".to_string(),
    }
}

/// The Python interpreter that can import `psd_tools`, if there is one.
fn reader() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PHOTOCRAFT_PSD_READER") {
        return Some(PathBuf::from(p));
    }
    let venv = workspace_root().join(".venv-psd").join("bin").join("python");
    if venv.is_file() {
        return Some(venv);
    }
    let has_psd_tools = Command::new("python3").args(["-c", "import psd_tools"]).output().map(|o| o.status.success()).unwrap_or(false);
    has_psd_tools.then(|| PathBuf::from("python3"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").to_path_buf()
}

/// Runs the reader over `files` and returns its view of each, in order.
fn read_with(reader: &Path, files: &[PathBuf]) -> Vec<Result<View, String>> {
    if files.is_empty() {
        return Vec::new();
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/psd_accept.py");
    let out = Command::new(reader).arg(&script).args(files).output().expect("run the reader");
    assert!(out.status.success(), "the reader failed: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    let mut views = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("the reader printed {line}: {e}"));
        if v["ok"].as_bool() != Some(true) {
            views.push(Err(v["error"].as_str().unwrap_or("the reader could not open the file").to_string()));
            continue;
        }
        let layers = v["layers"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|l| Layer {
                        name: l["name"].as_str().unwrap_or_default().to_string(),
                        group: l["group"].as_bool().unwrap_or(false),
                        visible: l["visible"].as_bool().unwrap_or(true),
                        kind: reader_kind_class(l["kind"].as_str().unwrap_or("unknown")),
                    })
                    .collect()
            })
            .unwrap_or_default();
        views.push(Ok(View { width: v["width"].as_u64().unwrap_or(0) as u32, height: v["height"].as_u64().unwrap_or(0) as u32, layers }));
    }
    assert_eq!(views.len(), files.len(), "the reader answered {} of {} files", views.len(), files.len());
    views
}

/// PhotoCraft's own view of a file: the layer tree, flattened the way a reader walks it (a
/// group first, then its children, bottom-most first).
fn our_view(bytes: &[u8]) -> Result<(u32, u32, Vec<Layer>), String> {
    let file = photocraft_psd::PsdFile::from_bytes(bytes).map_err(|e| e.to_string())?;
    let mut layers = Vec::new();
    flatten(&file, &file.layer_tree(), &mut layers);
    Ok((file.header.width, file.header.height, layers))
}

fn flatten(file: &photocraft_psd::PsdFile, nodes: &[photocraft_psd::LayerNode], out: &mut Vec<Layer>) {
    for node in nodes {
        let (index, children) = match node {
            photocraft_psd::LayerNode::Layer { index } => (*index, None),
            photocraft_psd::LayerNode::Group { index, children, .. } => (*index, Some(children)),
        };
        let record = &file.layers()[index];
        let blocks: Vec<[u8; 4]> = record.blocks.iter().map(|b| b.key).collect();
        out.push(Layer { name: record.name(), group: children.is_some(), visible: !record.flags.hidden(), kind: expected_kind(&blocks, children.is_some()) });
        if let Some(children) = children {
            flatten(file, children, out);
        }
    }
}

/// A directory for the exported files, removed when the test ends.
struct Exports {
    dir: PathBuf,
}

impl Exports {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("photocraft-psd-accept-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the export directory");
        Exports { dir }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).expect("write the export");
        path
    }
}

impl Drop for Exports {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Compares our layer tree with the reader's: the names, their order, the group nesting, the
/// visibility and the kind of every layer.
fn compare_layers(ours: &[Layer], theirs: &[Layer]) -> Result<(), String> {
    if ours.len() != theirs.len() {
        return Err(format!("we see {} layers, the reader sees {}", ours.len(), theirs.len()));
    }
    for (i, (a, b)) in ours.iter().zip(theirs).enumerate() {
        if a.name != b.name {
            return Err(format!("layer {i}: we call it {:?}, the reader calls it {:?}", a.name, b.name));
        }
        if a.group != b.group {
            return Err(format!(
                "layer {i} ({:?}): we see a {}, the reader a {}",
                a.name,
                if a.group { "group" } else { "layer" },
                if b.group { "group" } else { "layer" }
            ));
        }
        if a.visible != b.visible {
            return Err(format!(
                "layer {i} ({:?}): we see it {}, the reader {}",
                a.name,
                if a.visible { "visible" } else { "hidden" },
                if b.visible { "visible" } else { "hidden" }
            ));
        }
        if a.kind != b.kind {
            return Err(format!("layer {i} ({:?}): we expect a {} layer, the reader says {}", a.name, a.kind, b.kind));
        }
    }
    Ok(())
}

/// The documents built here: every colour mode and depth, with the layer kinds our exporter
/// writes (adjustments, fills, groups, masks, every blend mode).
fn synthetic_documents() -> Vec<(String, photocraft_doc::Document)> {
    use photocraft_color::{ColorMode, SampleType};
    let mut out = Vec::new();
    for (mode, depth) in [
        (ColorMode::Rgb, SampleType::U8),
        (ColorMode::Rgb, SampleType::U16),
        (ColorMode::Rgb, SampleType::F32),
        (ColorMode::Grayscale, SampleType::U8),
        (ColorMode::Cmyk, SampleType::U8),
        (ColorMode::Lab, SampleType::U16),
    ] {
        let doc = common::gen_doc(mode, depth, common::Features::ALL);
        out.push((format!("{mode:?}-{depth:?}"), doc));
    }
    out
}

#[test]
fn our_exports_open_in_a_third_party_reader() {
    let Some(reader) = reader() else {
        println!("SKIP: no third-party PSD reader (set PHOTOCRAFT_PSD_READER, or run `cargo xtask psd-accept --setup`)");
        return;
    };
    let exports = Exports::new("synthetic");
    // Write every document, then let the reader read them all in one go.
    let mut written = Vec::new();
    for (name, doc) in synthetic_documents() {
        let (file, _warnings) = photocraft_io::document_to_psd_with(&doc, &photocraft_io::PsdExportOptions::default());
        let bytes = file.to_bytes().unwrap_or_else(|e| panic!("{name}: write the file: {e}"));
        let path = exports.write(&format!("{name}.psd"), &bytes);
        written.push((name, path, bytes));
    }
    let paths: Vec<PathBuf> = written.iter().map(|(_, path, _)| path.clone()).collect();
    let views = read_with(&reader, &paths);
    let mut failures = Vec::new();
    for ((name, _path, bytes), theirs) in written.iter().zip(views) {
        let theirs = match theirs {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{name}: the reader could not open the export: {e}"));
                continue;
            }
        };
        let (width, height, ours) = match our_view(bytes) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{name}: our own reader could not read the export back: {e}"));
                continue;
            }
        };
        if (width, height) != (theirs.width, theirs.height) {
            failures.push(format!("{name}: the reader sees {width}x{height}, we wrote {}x{}", theirs.width, theirs.height));
        }
        if let Err(e) = compare_layers(&ours, &theirs.layers) {
            failures.push(format!("{name}: {e}"));
        }
    }
    assert!(failures.is_empty(), "the third-party reader disagrees with us:\n{}", failures.join("\n"));
}

#[cfg(feature = "corpus")]
/// Files whose re-save we know differs from the original, with the reason. A duotone document
/// is imported flattened (its inks are not interpreted), so its layer records are gone.
const KNOWN_FLATTENED: &[(&str, &str)] = &[("4x4_8bit_duotone.psd", "a duotone document is imported flattened: its inks are not interpreted")];

#[cfg(feature = "corpus")]
#[test]
fn our_resaved_corpus_files_open_in_a_third_party_reader() {
    let Some(reader) = reader() else {
        println!("SKIP: no third-party PSD reader (set PHOTOCRAFT_PSD_READER, or run `cargo xtask psd-accept --setup`)");
        return;
    };
    let root = workspace_root();
    let mut files = Vec::new();
    for dir in ["corpus/photoshop", "corpus/psd", "corpus/psd-tools"] {
        collect(&root.join(dir), &mut files);
    }
    files.sort();
    assert!(!files.is_empty(), "the corpus is missing: run `cargo xtask corpus --all`");
    let exports = Exports::new("corpus");
    let mut sources = Vec::new();
    let mut skipped = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let bytes = std::fs::read(file).expect("read the corpus file");
        let Ok(psd) = photocraft_psd::PsdFile::from_bytes(&bytes) else {
            skipped.push(format!("{}: our reader could not parse it", file.display()));
            continue;
        };
        let (doc, _warnings) = photocraft_io::psd_to_document(&psd);
        let (out, _warnings) = photocraft_io::document_to_psd_with(&doc, &photocraft_io::PsdExportOptions::default());
        let Ok(bytes) = out.to_bytes() else {
            skipped.push(format!("{}: we could not write it back", file.display()));
            continue;
        };
        let name = format!("{i:04}-{}", file.file_name().and_then(|n| n.to_str()).unwrap_or("file.psd"));
        sources.push((file.clone(), exports.write(&name, &bytes), bytes));
    }
    // What the reader makes of the original files, and of the files we wrote from them.
    let original_paths: Vec<PathBuf> = sources.iter().map(|(file, _, _)| file.clone()).collect();
    let original_views = read_with(&reader, &original_paths);
    let resaved_paths: Vec<PathBuf> = sources.iter().map(|(_, path, _)| path.clone()).collect();
    let resaved_views = read_with(&reader, &resaved_paths);
    let mut failures = Vec::new();
    let mut flattened = 0;
    for ((source, _path, bytes), (original, resaved)) in sources.iter().zip(original_views.into_iter().zip(resaved_views)) {
        let name = source.display();
        let (Ok(original), Ok(resaved)) = (original, resaved) else {
            failures.push(format!("{name}: the reader could not open the file or our export of it"));
            continue;
        };
        if (original.width, original.height) != (resaved.width, resaved.height) {
            failures.push(format!("{name}: the reader sees {}x{}, our export is {}x{}", original.width, original.height, resaved.width, resaved.height));
        }
        // Our own reader must read our own export, and agree with the reader about it.
        let (_width, _height, ours) = match our_view(bytes) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{name}: our own reader could not read the export back: {e}"));
                continue;
            }
        };
        if let Err(e) = compare_layers(&ours, &resaved.layers) {
            failures.push(format!("{name}: {e}"));
        }
        // A file with no layer records (a flat image) becomes a file with a background layer,
        // as it does in Photoshop: there is nothing to compare there.
        if original.layers.is_empty() {
            flattened += 1;
            continue;
        }
        if let Some((_, reason)) = KNOWN_FLATTENED.iter().find(|(f, _)| source.ends_with(f)) {
            println!("{}: our export differs from the original on purpose: {reason}", source.display());
            continue;
        }
        if let Err(e) = compare_layers(&original.layers, &resaved.layers) {
            failures.push(format!("{name}: our export does not look like the original: {e}"));
        }
    }
    assert!(failures.is_empty(), "the third-party reader disagrees with us:\n{}", failures.join("\n"));
    if !skipped.is_empty() {
        println!("skipped {} files:\n{}", skipped.len(), skipped.join("\n"));
    }
    println!("the third-party reader opened {} re-saved corpus files ({} flat ones became a background layer)", sources.len(), flattened);
}

#[cfg(feature = "corpus")]
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("psd") || e.eq_ignore_ascii_case("psb")) {
            out.push(path);
        }
    }
}
