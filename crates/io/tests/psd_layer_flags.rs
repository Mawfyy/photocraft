//! The "pixel data irrelevant" flag on the layers we write (Photoshop compatibility).
//!
//! Photoshop marks the layers whose appearance comes from a descriptor instead of from pixel
//! data: every adjustment layer, every fill layer, every shape layer and every group folder.
//! Measured over the 3135 layers of the real-file corpora (the Photoshop-authored oracles, the
//! psd-tools set and the ag-psd set), Photoshop sets the flag on all 346 adjustment layers, all
//! 251 fill layers, all 622 shape layers and 373 of the 376 group folders, and on no text layer
//! (123), no smart object (206) and no plain raster layer (1174). A layer that only carries a
//! vector mask keeps its own pixels, and the flag stays off (17 of 104 such layers).
//!
//! Without the flag a reader takes the layer for a raster layer: psd-tools then calls a fill
//! layer a "solidcolorfill" where Photoshop's own file says "shape".

use std::sync::Arc;

use photocraft_color::Color;
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent, Path, ShapeLayer, SmartObject, SmartSource, Subpath, TextLayer};
use photocraft_geom::{Rect, Size};
use photocraft_psd::LayerFlags;

mod common;

/// The flags of every layer of the file we write, by name.
fn exported_flags(doc: &Document) -> Vec<(String, u8)> {
    let file = photocraft_io::document_to_psd(doc);
    let bytes = file.to_bytes().expect("write the file");
    let psd = photocraft_psd::PsdFile::from_bytes(&bytes).expect("read the file back");
    psd.layers().iter().map(|r| (r.name(), r.flags.0)).collect()
}

fn flags_of(doc: &Document, name: &str) -> u8 {
    exported_flags(doc).into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no layer named {name}")).1
}

/// A document with one layer of every kind the exporter writes.
fn document() -> Document {
    let mut doc = Document::new("flags", Size::new(32, 32), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = doc.pixel_format();
    doc.layers.push(common::raster("Raster", fmt, Rect::new(0, 0, 32, 32), 1, false));
    doc.layers.push(Layer::new("Adjustment", LayerContent::Adjustment(Adjustment::Invert)));
    doc.layers.push(Layer::new("Fill", LayerContent::Fill(Fill::Solid(Color::BLACK))));
    let path = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])]);
    doc.layers.push(Layer::new(
        "Shape",
        LayerContent::Shape(ShapeLayer { path, fill: Some(Fill::Solid(Color::rgb(1.0, 0.0, 0.0))), stroke: None, live: None, cache: None, psd_raw: None }),
    ));
    doc.layers.push(Layer::new("Text", LayerContent::Text(TextLayer::default())));
    doc.layers.push(Layer::new(
        "Smart",
        LayerContent::Smart(SmartObject::new(
            SmartSource::Embedded { file_name: "source.png".into(), bytes: Arc::new(vec![1, 2, 3]) },
            photocraft_geom::Affine::IDENTITY,
            None,
        )),
    ));
    doc.layers.push(Layer::group("Group", vec![common::raster("Inner", fmt, Rect::new(0, 0, 4, 4), 2, true)]));
    doc
}

#[test]
fn the_layers_that_carry_their_appearance_in_a_descriptor_are_marked() {
    let doc = document();
    // The layers whose appearance a descriptor decides: the flag is set.
    for name in ["Adjustment", "Fill", "Shape", "Group"] {
        let flags = flags_of(&doc, name);
        assert!(flags & LayerFlags::PIXEL_DATA_IRRELEVANT != 0, "the {name} layer carries its appearance in a descriptor, so it must be marked: {flags:#04x}");
        assert!(flags & LayerFlags::BIT4_USEFUL != 0, "the {name} layer must say that bit 4 means something: {flags:#04x}");
    }
    // The layers with pixel data of their own: the flag stays off.
    for name in ["Raster", "Text", "Smart"] {
        let flags = flags_of(&doc, name);
        assert!(flags & LayerFlags::PIXEL_DATA_IRRELEVANT == 0, "the {name} layer has pixel data, so it must not be marked: {flags:#04x}");
    }
}
