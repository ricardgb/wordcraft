//! Pages handed to a host as vector drawing, for a host that draws egui's shapes itself (with
//! Skia, say) and wants a page as its draw items rather than as a raster texture: sharp at any
//! scale, and cheap to send again after an edit.
//!
//! Off unless a host installs a [`PageVectorHook`] in the egui context
//! ([`PageVectorHook::install`]); WordCraft's own app never does, and draws its pages as rasters.
//! With the hook, the canvas offers each visible page to [`PageVectorHook::accepts`]; a page the
//! host takes is painted as an egui [`egui::Shape::Callback`] whose callback is a [`PageCallback`]
//! (downcast it from [`egui::PaintCallback::callback`]). A page it declines is drawn as before.

use std::sync::Arc;

use egui::{Pos2, TextureId};
use wordcraft_doc::props::Rgb;
use wordcraft_layout::display::Draw;
use wordcraft_render::RenderOptions;

/// The host's side: which pages it takes, and whether it still wants the rasters.
#[derive(Clone)]
pub struct PageVectorHook {
    /// Draw each page as a raster too, under the callback, for a host that also shows egui's
    /// tessellated meshes (where a callback draws nothing). The callback then names the textures
    /// it stands for ([`PageCallback::raster`]), for the host to leave out where it plays it.
    pub raster_too: bool,
    /// Whether the host can draw this page; asked once per page content.
    pub accepts: Arc<dyn Fn(&PageVector) -> bool + Send + Sync>,
}

impl PageVectorHook {
    fn id() -> egui::Id {
        egui::Id::unique("wordcraft::host_pages::PageVectorHook")
    }

    /// Install the hook in `ctx` (`None` removes it).
    pub fn install(ctx: &egui::Context, hook: Option<PageVectorHook>) {
        ctx.data_mut(|d| match hook {
            Some(h) => {
                d.insert_temp(Self::id(), h);
            }
            None => d.remove::<PageVectorHook>(Self::id()),
        });
    }

    pub(crate) fn get(ctx: &egui::Context) -> Option<PageVectorHook> {
        ctx.data(|d| d.get_temp::<PageVectorHook>(Self::id()))
    }
}

/// A page's drawing, in page points (origin at the page's top left, y down), as the raster
/// renderer would draw it: the paper, then each layer at its opacity.
pub struct PageVector {
    /// The page's size in points.
    pub w: f32,
    pub h: f32,
    /// The paper colour, already inked ([`RenderOptions::paper_of`]).
    pub paper: Rgb,
    /// The document's page picture and watermark, which the renderer draws itself and these
    /// layers leave out: a host that cannot draw them should decline the page.
    pub page_picture: bool,
    pub watermark: bool,
    /// Headers and footers, then the body, each with the opacity to draw it at as one group
    /// ([`wordcraft_layout::display::page_layers`]).
    pub layers: Vec<(f32, Vec<Draw>)>,
    /// What colours become on screen ([`RenderOptions::ink`] and friends: a dark page).
    pub opts: RenderOptions,
    /// The pictures the items name, decoded (premultiplied RGBA8), by [`Draw::Image`]'s media
    /// key and effects.
    pub images: Vec<PagePicture>,
}

/// A decoded picture of a page.
pub struct PagePicture {
    pub media: String,
    pub fx: wordcraft_doc::picfx::PicFx,
    /// A metafile, which has no pixels here: `None`.
    pub pixels: Option<wordcraft_render::PicturePixels>,
}

impl PageVector {
    /// The decoded picture for an image item, if any.
    pub fn picture(&self, media: &str, fx: &wordcraft_doc::picfx::PicFx) -> Option<&PagePicture> {
        self.images.iter().find(|p| p.media == media && p.fx == *fx)
    }
}

/// What a page's [`egui::PaintCallback`] carries: the page and where it is on screen. The
/// callback's rect is the page's rect on screen.
pub struct PageCallback {
    pub page: Arc<PageVector>,
    /// The page's top left on screen, in points, and screen points per page point.
    pub origin: Pos2,
    pub scale: f32,
    /// With [`PageVectorHook::raster_too`]: the textures drawn for this page (one, or its tiles).
    pub raster: Vec<TextureId>,
}

/// Build a page's [`PageVector`].
pub(crate) fn page_vector(doc: &wordcraft_doc::Document, page: &wordcraft_layout::Page, opts: &RenderOptions) -> PageVector {
    let layers: Vec<(f32, Vec<Draw>)> = wordcraft_layout::display::page_layers(doc, page, &opts.display).into_iter().collect();
    let mut images: Vec<PagePicture> = Vec::new();
    fn walk(doc: &wordcraft_doc::Document, items: &[Draw], images: &mut Vec<PagePicture>) {
        for it in items {
            match it {
                Draw::Image { media, fx, .. } => {
                    if !images.iter().any(|p| p.media == *media && p.fx == *fx) {
                        let pixels = wordcraft_render::picture_pixels(doc, media, *fx);
                        images.push(PagePicture { media: media.clone(), fx: *fx, pixels });
                    }
                }
                Draw::Figure { draws: items, .. }
                | Draw::Turned { items, .. }
                | Draw::Rotated { items, .. }
                | Draw::Clip { items, .. } => walk(doc, items, images),
                _ => {}
            }
        }
    }
    for (_, items) in &layers {
        walk(doc, items, &mut images);
    }
    PageVector {
        w: page.w,
        h: page.h.min(1e6),
        paper: opts.paper_of(doc),
        page_picture: opts.read_paper.is_none() && doc.settings.page_picture.is_some(),
        watermark: doc.settings.watermark.is_some(),
        layers,
        opts: opts.clone(),
        images,
    }
}
