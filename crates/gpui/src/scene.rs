// todo("windows"): remove
#![cfg_attr(windows, allow(dead_code))]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AtlasTextureId, AtlasTile, Background, Bounds, ContentMask, Corners, Edges, GlobalElementId,
    Hsla, Pixels, Point, Radians, ScaledPixels, Size, bounds_tree::BoundsTree, point,
};
use std::{
    fmt::Debug,
    iter::Peekable,
    ops::{Add, Range, Sub},
    slice,
};

#[allow(non_camel_case_types, unused)]
#[expect(missing_docs)]
pub type PathVertex_ScaledPixels = PathVertex<ScaledPixels>;

#[expect(missing_docs)]
pub type DrawOrder = u32;

/// A boolean stored as an initialized `u32` for padding-free GPU buffer layouts.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PaddedBool32(u32);

impl From<bool> for PaddedBool32 {
    fn from(value: bool) -> Self {
        Self(value as u32)
    }
}

#[derive(Default)]
#[expect(missing_docs)]
pub struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    /// Retained compositor layers in this scene.
    pub retained_layers: Vec<RetainedLayer>,
    pub shadows: Vec<Shadow>,
    pub backdrop_blurs: Vec<BackdropBlur>,
    pub quads: Vec<Quad>,
    pub paths: Vec<Path<ScaledPixels>>,
    pub underlines: Vec<Underline>,
    pub monochrome_sprites: Vec<MonochromeSprite>,
    pub subpixel_sprites: Vec<SubpixelSprite>,
    pub polychrome_sprites: Vec<PolychromeSprite>,
    pub surfaces: Vec<PaintSurface>,
}

#[expect(missing_docs)]
impl Scene {
    pub fn clear(&mut self) {
        self.paint_operations.clear();
        self.primitive_bounds.clear();
        self.layer_stack.clear();
        self.retained_layers.clear();
        self.paths.clear();
        self.shadows.clear();
        self.backdrop_blurs.clear();
        self.quads.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.subpixel_sprites.clear();
        self.polychrome_sprites.clear();
        self.surfaces.clear();
    }

    pub fn len(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn push_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        let order = self.primitive_bounds.insert(bounds);
        self.layer_stack.push(order);
        self.paint_operations
            .push(PaintOperation::StartLayer(bounds));
    }

    pub fn pop_layer(&mut self) {
        self.layer_stack.pop();
        self.paint_operations.push(PaintOperation::EndLayer);
    }

    pub fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        let clipped_bounds = primitive
            .bounds()
            .intersect(&primitive.content_mask().bounds);

        if clipped_bounds.is_empty() {
            return;
        }

        let order = self
            .layer_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.primitive_bounds.insert(clipped_bounds));
        match &mut primitive {
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(shadow.clone());
            }
            Primitive::BackdropBlur(blur) => {
                blur.order = order;
                self.backdrop_blurs.push(blur.clone());
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(quad.clone());
            }
            Primitive::Path(path) => {
                path.order = order;
                path.id = PathId(self.paths.len());
                self.paths.push(path.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(underline.clone());
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.order = order;
                self.monochrome_sprites.push(sprite.clone());
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.order = order;
                self.subpixel_sprites.push(sprite.clone());
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.order = order;
                self.polychrome_sprites.push(sprite.clone());
            }
            Primitive::Surface(surface) => {
                surface.order = order;
                self.surfaces.push(surface.clone());
            }
        }
        self.paint_operations
            .push(PaintOperation::Primitive(primitive));
    }

    pub fn replay(&mut self, range: Range<usize>, prev_scene: &Scene) {
        let start = self.paint_operations.len();
        for operation in &prev_scene.paint_operations[range.clone()] {
            match operation {
                PaintOperation::Primitive(primitive) => self.insert_primitive(primitive.clone()),
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
            }
        }

        self.retained_layers.extend(
            prev_scene
                .retained_layers
                .iter()
                .filter(|layer| {
                    range.start <= layer.paint_range.start && layer.paint_range.end <= range.end
                })
                .map(|layer| {
                    let mut layer = layer.clone();
                    layer.content_dirty = false;
                    layer.paint_range = start + (layer.paint_range.start - range.start)
                        ..start + (layer.paint_range.end - range.start);
                    layer
                }),
        );
    }

    pub(crate) fn insert_retained_layer(&mut self, layer: RetainedLayer) {
        debug_assert!(layer.paint_range.end <= self.paint_operations.len());
        self.retained_layers.push(layer);
    }

    #[doc(hidden)]
    pub fn paint_operation_count(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn clone_paint_range(&self, range: Range<usize>) -> Self {
        self.clone_paint_operations(|index| range.contains(&index))
    }

    pub fn clone_excluding_paint_ranges(&self, excluded_ranges: &[Range<usize>]) -> Self {
        self.clone_paint_operations(|index| {
            !excluded_ranges
                .iter()
                .any(|range| range.start <= index && index < range.end)
        })
    }

    fn clone_paint_operations(&self, include: impl Fn(usize) -> bool) -> Self {
        let mut scene = Self::default();
        for (index, operation) in self.paint_operations.iter().enumerate() {
            if include(index) {
                scene.push_cloned_operation(operation);
            }
        }
        scene.finish();
        scene
    }

    fn push_cloned_operation(&mut self, operation: &PaintOperation) {
        self.paint_operations.push(operation.clone());
        if let PaintOperation::Primitive(primitive) = operation {
            match primitive {
                Primitive::Shadow(shadow) => self.shadows.push(shadow.clone()),
                Primitive::BackdropBlur(blur) => self.backdrop_blurs.push(blur.clone()),
                Primitive::Quad(quad) => self.quads.push(quad.clone()),
                Primitive::Path(path) => self.paths.push(path.clone()),
                Primitive::Underline(underline) => self.underlines.push(underline.clone()),
                Primitive::MonochromeSprite(sprite) => {
                    self.monochrome_sprites.push(sprite.clone());
                }
                Primitive::SubpixelSprite(sprite) => {
                    self.subpixel_sprites.push(sprite.clone());
                }
                Primitive::PolychromeSprite(sprite) => {
                    self.polychrome_sprites.push(sprite.clone());
                }
                Primitive::Surface(surface) => self.surfaces.push(surface.clone()),
            }
        }
    }

    pub fn finish(&mut self) {
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.backdrop_blurs.sort_by_key(|blur| blur.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.paths.sort_by_key(|path| path.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.monochrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.subpixel_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.polychrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.surfaces.sort_by_key(|surface| surface.order);
    }

    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    pub fn batches(&self) -> impl Iterator<Item = PrimitiveBatch> + '_ {
        BatchIterator {
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            backdrop_blurs_start: 0,
            backdrop_blurs_iter: self.backdrop_blurs.iter().peekable(),
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            paths_start: 0,
            paths_iter: self.paths.iter().peekable(),
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            monochrome_sprites_start: 0,
            monochrome_sprites_iter: self.monochrome_sprites.iter().peekable(),
            subpixel_sprites_start: 0,
            subpixel_sprites_iter: self.subpixel_sprites.iter().peekable(),
            polychrome_sprites_start: 0,
            polychrome_sprites_iter: self.polychrome_sprites.iter().peekable(),
            surfaces_start: 0,
            surfaces_iter: self.surfaces.iter().peekable(),
        }
    }
}

/// Monotonic caller-owned revision for cached retained layer content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct RetainedLayerContentRevision(pub u64);

impl From<u64> for RetainedLayerContentRevision {
    fn from(revision: u64) -> Self {
        Self(revision)
    }
}

/// Stable scene contract for renderer-owned retained compositor layer caches.
#[derive(Clone, Debug, PartialEq)]
pub struct RetainedLayer {
    /// Stable element id for cache lookup across frames.
    pub id: GlobalElementId,
    /// Caller-owned revision for child paint output.
    pub content_revision: RetainedLayerContentRevision,
    /// Whether renderer must repaint cached child content for this frame.
    pub content_dirty: bool,
    /// Layer bounds in scene coordinates.
    pub bounds: Bounds<ScaledPixels>,
    /// Active mask in scene coordinates.
    pub content_mask: ContentMask<ScaledPixels>,
    /// Compositor transform applied to cached child content.
    pub transform: TransformationMatrix,
    /// Compositor opacity applied to cached child content.
    pub opacity: f32,
    /// Paint operation range that produced child content.
    pub paint_range: Range<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub(crate) enum PrimitiveKind {
    Shadow,
    BackdropBlur,
    #[default]
    Quad,
    Path,
    Underline,
    MonochromeSprite,
    SubpixelSprite,
    PolychromeSprite,
    Surface,
}

#[derive(Clone)]
pub(crate) enum PaintOperation {
    Primitive(Primitive),
    StartLayer(Bounds<ScaledPixels>),
    EndLayer,
}

#[derive(Clone)]
#[expect(missing_docs)]
pub enum Primitive {
    Shadow(Shadow),
    BackdropBlur(BackdropBlur),
    Quad(Quad),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    SubpixelSprite(SubpixelSprite),
    PolychromeSprite(PolychromeSprite),
    Surface(PaintSurface),
}

#[expect(missing_docs)]
impl Primitive {
    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) if shadow.inset != 0 => &shadow.element_bounds,
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::BackdropBlur(blur) => &blur.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Path(path) => &path.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::MonochromeSprite(sprite) => &sprite.bounds,
            Primitive::SubpixelSprite(sprite) => &sprite.bounds,
            Primitive::PolychromeSprite(sprite) => &sprite.bounds,
            Primitive::Surface(surface) => &surface.bounds,
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::BackdropBlur(blur) => &blur.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Path(path) => &path.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &sprite.content_mask,
            Primitive::SubpixelSprite(sprite) => &sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &sprite.content_mask,
            Primitive::Surface(surface) => &surface.content_mask,
        }
    }
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
struct BatchIterator<'a> {
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    backdrop_blurs_start: usize,
    backdrop_blurs_iter: Peekable<slice::Iter<'a, BackdropBlur>>,
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    paths_start: usize,
    paths_iter: Peekable<slice::Iter<'a, Path<ScaledPixels>>>,
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    monochrome_sprites_start: usize,
    monochrome_sprites_iter: Peekable<slice::Iter<'a, MonochromeSprite>>,
    subpixel_sprites_start: usize,
    subpixel_sprites_iter: Peekable<slice::Iter<'a, SubpixelSprite>>,
    polychrome_sprites_start: usize,
    polychrome_sprites_iter: Peekable<slice::Iter<'a, PolychromeSprite>>,
    surfaces_start: usize,
    surfaces_iter: Peekable<slice::Iter<'a, PaintSurface>>,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (
                self.shadows_iter.peek().map(|s| s.order),
                PrimitiveKind::Shadow,
            ),
            (
                self.backdrop_blurs_iter.peek().map(|b| b.order),
                PrimitiveKind::BackdropBlur,
            ),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (self.paths_iter.peek().map(|q| q.order), PrimitiveKind::Path),
            (
                self.underlines_iter.peek().map(|u| u.order),
                PrimitiveKind::Underline,
            ),
            (
                self.monochrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::MonochromeSprite,
            ),
            (
                self.subpixel_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::SubpixelSprite,
            ),
            (
                self.polychrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::PolychromeSprite,
            ),
            (
                self.surfaces_iter.peek().map(|s| s.order),
                PrimitiveKind::Surface,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max_order_and_kind) = if first.0.is_some() {
            (first.1, (second.0.unwrap_or(u32::MAX), second.1))
        } else {
            return None;
        };

        match batch_kind {
            PrimitiveKind::Shadow => {
                let shadows_start = self.shadows_start;
                let mut shadows_end = shadows_start + 1;
                self.shadows_iter.next();
                while self
                    .shadows_iter
                    .next_if(|shadow| (shadow.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    shadows_end += 1;
                }
                self.shadows_start = shadows_end;
                Some(PrimitiveBatch::Shadows(shadows_start..shadows_end))
            }
            PrimitiveKind::BackdropBlur => {
                let blurs_start = self.backdrop_blurs_start;
                let mut blurs_end = blurs_start + 1;
                self.backdrop_blurs_iter.next();
                while self
                    .backdrop_blurs_iter
                    .next_if(|blur| (blur.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    blurs_end += 1;
                }
                self.backdrop_blurs_start = blurs_end;
                Some(PrimitiveBatch::BackdropBlurs(blurs_start..blurs_end))
            }
            PrimitiveKind::Quad => {
                let quads_start = self.quads_start;
                let mut quads_end = quads_start + 1;
                self.quads_iter.next();
                while self
                    .quads_iter
                    .next_if(|quad| (quad.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    quads_end += 1;
                }
                self.quads_start = quads_end;
                Some(PrimitiveBatch::Quads(quads_start..quads_end))
            }
            PrimitiveKind::Path => {
                let paths_start = self.paths_start;
                let mut paths_end = paths_start + 1;
                self.paths_iter.next();
                while self
                    .paths_iter
                    .next_if(|path| (path.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    paths_end += 1;
                }
                self.paths_start = paths_end;
                Some(PrimitiveBatch::Paths(paths_start..paths_end))
            }
            PrimitiveKind::Underline => {
                let underlines_start = self.underlines_start;
                let mut underlines_end = underlines_start + 1;
                self.underlines_iter.next();
                while self
                    .underlines_iter
                    .next_if(|underline| (underline.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    underlines_end += 1;
                }
                self.underlines_start = underlines_end;
                Some(PrimitiveBatch::Underlines(underlines_start..underlines_end))
            }
            PrimitiveKind::MonochromeSprite => {
                let texture_id = self.monochrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.monochrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.monochrome_sprites_iter.next();
                while self
                    .monochrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.monochrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::SubpixelSprite => {
                let texture_id = self.subpixel_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.subpixel_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.subpixel_sprites_iter.next();
                while self
                    .subpixel_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.subpixel_sprites_start = sprites_end;
                Some(PrimitiveBatch::SubpixelSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::PolychromeSprite => {
                let texture_id = self.polychrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.polychrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.polychrome_sprites_iter.next();
                while self
                    .polychrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.polychrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::Surface => {
                let surfaces_start = self.surfaces_start;
                let mut surfaces_end = surfaces_start + 1;
                self.surfaces_iter.next();
                while self
                    .surfaces_iter
                    .next_if(|surface| (surface.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    surfaces_end += 1;
                }
                self.surfaces_start = surfaces_end;
                Some(PrimitiveBatch::Surfaces(surfaces_start..surfaces_end))
            }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
#[expect(missing_docs)]
pub enum PrimitiveBatch {
    Shadows(Range<usize>),
    BackdropBlurs(Range<usize>),
    Quads(Range<usize>),
    Paths(Range<usize>),
    Underlines(Range<usize>),
    MonochromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    SubpixelSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    PolychromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    Surfaces(Range<usize>),
}

#[derive(Default, Debug, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
}

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Underline {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: PaddedBool32,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub element_bounds: Bounds<ScaledPixels>,
    pub element_corner_radii: Corners<ScaledPixels>,
    /// 0 = drop shadow (rendered outside the element), 1 = inset shadow (rendered inside).
    pub inset: u32,
    pub pad: u32, // align to 8 bytes
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

/// Internal blur-pyramid configuration shared by renderer backends.
#[doc(hidden)]
#[derive(Copy, Clone, Debug)]
pub struct BackdropBlurPlan {
    /// Number of downsample and upsample passes.
    pub passes: usize,
    /// Upsample tent-filter distance in input texels.
    pub sample_distance: f32,
}

impl BackdropBlurPlan {
    /// Largest pyramid supported by renderer backends.
    pub const MAX_PASSES: usize = 6;

    const DOWNSAMPLE_VARIANCE: f32 = 0.75;
    const MAX_SAMPLE_DISTANCE: f32 = 1.5;
    const SUPPORT_SIGMAS: f32 = 3.0;
    const SOLVER_STEPS: usize = 20;

    /// A plan that samples the unblurred backdrop.
    pub const IDENTITY: Self = Self {
        passes: 0,
        sample_distance: 0.0,
    };

    /// Builds a bounded pyramid whose three-sigma support approximates `radius`.
    ///
    /// Positive radii below the pyramid's minimum support use that minimum.
    pub fn for_radius(radius: f32, max_passes: usize) -> Self {
        if !radius.is_finite() || radius <= 0.0 || max_passes == 0 {
            return Self::IDENTITY;
        }

        let max_passes = max_passes.min(Self::MAX_PASSES);
        let sigma_squared = (radius / Self::SUPPORT_SIGMAS).powi(2);
        for passes in 1..=max_passes {
            if sigma_squared <= Self::variance(passes, 0.0) {
                return Self {
                    passes,
                    sample_distance: 0.0,
                };
            }

            if sigma_squared <= Self::variance(passes, Self::MAX_SAMPLE_DISTANCE) {
                let mut lower = 0.0;
                let mut upper = Self::MAX_SAMPLE_DISTANCE;
                for _ in 0..Self::SOLVER_STEPS {
                    let sample_distance = (lower + upper) * 0.5;
                    if Self::variance(passes, sample_distance) < sigma_squared {
                        lower = sample_distance;
                    } else {
                        upper = sample_distance;
                    }
                }
                return Self {
                    passes,
                    sample_distance: (lower + upper) * 0.5,
                };
            }
        }

        Self {
            passes: max_passes,
            sample_distance: Self::MAX_SAMPLE_DISTANCE,
        }
    }

    /// Returns source padding required by the supported three-sigma kernel.
    pub fn padding(radius: f32) -> f32 {
        if radius.is_finite() && radius > 0.0 {
            let minimum_support = Self::variance(1, 0.0).sqrt() * Self::SUPPORT_SIGMAS;
            let maximum_support = Self::variance(Self::MAX_PASSES, Self::MAX_SAMPLE_DISTANCE)
                .sqrt()
                * Self::SUPPORT_SIGMAS;
            let support = radius.clamp(minimum_support, maximum_support);
            (support + 2.0).ceil()
        } else {
            0.0
        }
    }

    fn variance(passes: usize, sample_distance: f32) -> f32 {
        if passes == 0 {
            return 0.0;
        }

        let four_to_passes = 4.0_f32.powi(passes as i32);
        let level_scale_sum = (four_to_passes - 1.0) / 3.0;
        (Self::DOWNSAMPLE_VARIANCE + Self::upsample_variance(sample_distance)) * level_scale_sum
    }

    fn upsample_variance(sample_distance: f32) -> f32 {
        Self::linear_upsample_second_moment(0.0) / 6.0
            + (Self::linear_upsample_second_moment(sample_distance)
                + Self::linear_upsample_second_moment(-sample_distance))
                / 3.0
            + (Self::linear_upsample_second_moment(2.0 * sample_distance)
                + Self::linear_upsample_second_moment(-2.0 * sample_distance))
                / 12.0
    }

    fn linear_upsample_second_moment(offset: f32) -> f32 {
        // Bilinear reconstruction of one input texel is a radius-two tent on
        // the output lattice. Measure around 0.5, the shared centroid of the
        // symmetric 8-tap upsample kernel.
        let center = 0.5 - 2.0 * offset;
        let first_output = center.floor() as i32 - 2;
        let mut weight_sum = 0.0;
        let mut second_moment = 0.0;
        for output in first_output..=first_output + 5 {
            let weight = (1.0 - ((output as f32 - center).abs() / 2.0)).max(0.0);
            weight_sum += weight;
            second_moment += weight * (output as f32 - 0.5).powi(2);
        }
        second_moment / weight_sum
    }

    #[cfg(test)]
    fn sigma(self) -> f32 {
        Self::variance(self.passes, self.sample_distance).sqrt()
    }
}

impl PartialEq for BackdropBlurPlan {
    fn eq(&self, other: &Self) -> bool {
        self.passes == other.passes
            && self.sample_distance.to_bits() == other.sample_distance.to_bits()
    }
}

impl Eq for BackdropBlurPlan {}

#[derive(Debug, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct BackdropBlur {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub blur_radius: ScaledPixels,
    pub source_origin_x: f32,
    pub source_origin_y: f32,
    pub source_width: f32,
    pub source_height: f32,
    pub pad2: u32, // align storage buffer stride to WGSL layout
}

impl From<BackdropBlur> for Primitive {
    fn from(blur: BackdropBlur) -> Self {
        Primitive::BackdropBlur(blur)
    }
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum BorderStyle {
    /// A solid border.
    #[default]
    Solid = 0,
    /// A dashed border.
    Dashed = 1,
}

/// A data type representing a 2 dimensional transformation that can be applied to an element.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    /// 2x2 matrix containing rotation and scale,
    /// stored row-major
    pub rotation_scale: [[f32; 2]; 2],
    /// translation vector
    pub translation: [f32; 2],
}

impl Eq for TransformationMatrix {}

impl TransformationMatrix {
    /// The unit matrix, has no effect.
    pub fn unit() -> Self {
        Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [0.0, 0.0],
        }
    }

    /// Returns true when this transform is identity.
    pub fn is_unit(&self) -> bool {
        *self == Self::unit()
    }

    /// Move the origin by a given point
    pub fn translate(mut self, point: Point<ScaledPixels>) -> Self {
        self.compose(Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [point.x.0, point.y.0],
        })
    }

    /// Clockwise rotation in radians around the origin
    pub fn rotate(self, angle: Radians) -> Self {
        self.compose(Self {
            rotation_scale: [
                [angle.0.cos(), -angle.0.sin()],
                [angle.0.sin(), angle.0.cos()],
            ],
            translation: [0.0, 0.0],
        })
    }

    /// Scale around the origin
    pub fn scale(self, size: Size<f32>) -> Self {
        self.compose(Self {
            rotation_scale: [[size.width, 0.0], [0.0, size.height]],
            translation: [0.0, 0.0],
        })
    }

    /// Perform matrix multiplication with another transformation
    /// to produce a new transformation that is the result of
    /// applying both transformations: first, `other`, then `self`.
    #[inline]
    pub fn compose(self, other: TransformationMatrix) -> TransformationMatrix {
        if other == Self::unit() {
            return self;
        }
        // Perform matrix multiplication
        TransformationMatrix {
            rotation_scale: [
                [
                    self.rotation_scale[0][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][0],
                    self.rotation_scale[0][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][1],
                ],
                [
                    self.rotation_scale[1][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][0],
                    self.rotation_scale[1][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][1],
                ],
            ],
            translation: [
                self.translation[0]
                    + self.rotation_scale[0][0] * other.translation[0]
                    + self.rotation_scale[0][1] * other.translation[1],
                self.translation[1]
                    + self.rotation_scale[1][0] * other.translation[0]
                    + self.rotation_scale[1][1] * other.translation[1],
            ],
        }
    }

    /// Apply transformation to a point, mainly useful for debugging
    pub fn apply(&self, point: Point<Pixels>) -> Point<Pixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(output[0].into(), output[1].into())
    }

    /// Apply transformation to a scaled point.
    pub fn apply_scaled(&self, point: Point<ScaledPixels>) -> Point<ScaledPixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(ScaledPixels(output[0]), ScaledPixels(output[1]))
    }

    /// Attempt to compute inverse matrix.
    pub fn try_inverse(&self) -> Option<Self> {
        let [[a, b], [c, d]] = self.rotation_scale;
        let det = a * d - b * c;
        if det.abs() < f32::EPSILON {
            return None;
        }
        let inv_det = 1.0 / det;
        let inv_rotation_scale = [[d * inv_det, -b * inv_det], [-c * inv_det, a * inv_det]];

        let tx = self.translation[0];
        let ty = self.translation[1];
        let inv_translation = [
            -(inv_rotation_scale[0][0] * tx + inv_rotation_scale[0][1] * ty),
            -(inv_rotation_scale[1][0] * tx + inv_rotation_scale[1][1] * ty),
        ];

        Some(Self {
            rotation_scale: inv_rotation_scale,
            translation: inv_translation,
        })
    }
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct MonochromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<MonochromeSprite> for Primitive {
    fn from(sprite: MonochromeSprite) -> Self {
        Primitive::MonochromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct SubpixelSprite {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<SubpixelSprite> for Primitive {
    fn from(sprite: SubpixelSprite) -> Self {
        Primitive::SubpixelSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PolychromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub grayscale: PaddedBool32,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub tile: AtlasTile,
}

impl From<PolychromeSprite> for Primitive {
    fn from(sprite: PolychromeSprite) -> Self {
        Primitive::PolychromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[expect(missing_docs)]
pub struct PaintSurface {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    #[cfg(target_os = "macos")]
    pub image_buffer: core_video::pixel_buffer::CVPixelBuffer,
}

impl From<PaintSurface> for Primitive {
    fn from(surface: PaintSurface) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs)]
pub struct PathId(pub usize);

/// A line made up of a series of vertices and control points.
#[derive(Clone, Debug)]
#[expect(missing_docs)]
pub struct Path<P: Clone + Debug + Default + PartialEq> {
    pub id: PathId,
    pub order: DrawOrder,
    pub bounds: Bounds<P>,
    pub content_mask: ContentMask<P>,
    pub vertices: Vec<PathVertex<P>>,
    pub color: Background,
    start: Point<P>,
    current: Point<P>,
    contour_count: usize,
}

impl Path<Pixels> {
    /// Create a new path with the given starting point.
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            id: PathId(0),
            order: DrawOrder::default(),
            vertices: Vec::new(),
            start,
            current: start,
            bounds: Bounds {
                origin: start,
                size: Default::default(),
            },
            content_mask: Default::default(),
            color: Default::default(),
            contour_count: 0,
        }
    }

    /// Scale this path by the given factor.
    pub fn scale(&self, factor: f32) -> Path<ScaledPixels> {
        Path {
            id: self.id,
            order: self.order,
            bounds: self.bounds.scale(factor),
            content_mask: self.content_mask.scale(factor),
            vertices: self
                .vertices
                .iter()
                .map(|vertex| vertex.scale(factor))
                .collect(),
            start: self.start.map(|start| start.scale(factor)),
            current: self.current.scale(factor),
            contour_count: self.contour_count,
            color: self.color,
        }
    }

    /// Move the start, current point to the given point.
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        self.start = to;
        self.current = to;
    }

    /// Draw a straight line from the current point to the given point.
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }
        self.current = to;
    }

    /// Draw a curve from the current point to the given point, using the given control point.
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }

        self.push_triangle(
            (self.current, ctrl, to),
            (point(0., 0.), point(0.5, 0.), point(1., 1.)),
        );
        self.current = to;
    }

    /// Push a triangle to the Path.
    pub fn push_triangle(
        &mut self,
        xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>),
        st: (Point<f32>, Point<f32>, Point<f32>),
    ) {
        self.bounds = self
            .bounds
            .union(&Bounds {
                origin: xy.0,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.1,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.2,
                size: Default::default(),
            });

        self.vertices.push(PathVertex {
            xy_position: xy.0,
            st_position: st.0,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.1,
            st_position: st.1,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.2,
            st_position: st.2,
            content_mask: Default::default(),
        });
    }
}

impl<T> Path<T>
where
    T: Clone + Debug + Default + PartialEq + PartialOrd + Add<T, Output = T> + Sub<Output = T>,
{
    #[allow(unused)]
    #[expect(missing_docs)]
    pub fn clipped_bounds(&self) -> Bounds<T> {
        self.bounds.intersect(&self.content_mask.bounds)
    }
}

impl From<Path<ScaledPixels>> for Primitive {
    fn from(path: Path<ScaledPixels>) -> Self {
        Primitive::Path(path)
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PathVertex<P: Clone + Debug + Default + PartialEq> {
    pub xy_position: Point<P>,
    pub st_position: Point<f32>,
    pub content_mask: ContentMask<P>,
}

#[expect(missing_docs)]
impl PathVertex<Pixels> {
    pub fn scale(&self, factor: f32) -> PathVertex<ScaledPixels> {
        PathVertex {
            xy_position: self.xy_position.scale(factor),
            st_position: self.st_position,
            content_mask: self.content_mask.scale(factor),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, offset_of, size_of};
    use std::sync::Arc;

    use super::*;
    use crate::{ElementId, size};

    fn test_bounds() -> Bounds<ScaledPixels> {
        Bounds::new(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(ScaledPixels(10.0), ScaledPixels(10.0)),
        )
    }

    fn test_global_id() -> GlobalElementId {
        GlobalElementId(Arc::from([ElementId::from("layer")]))
    }

    fn test_backdrop_blur(order: DrawOrder) -> BackdropBlur {
        let bounds = test_bounds();
        BackdropBlur {
            order,
            pad: 0,
            bounds,
            content_mask: ContentMask::new(bounds),
            corner_radii: Corners::all(ScaledPixels(2.0)),
            blur_radius: ScaledPixels(12.0),
            source_origin_x: 0.0,
            source_origin_y: 0.0,
            source_width: 1.0,
            source_height: 1.0,
            pad2: 0,
        }
    }

    #[test]
    fn content_mask_gpu_layout_matches_shader_storage_contract() {
        assert_eq!(align_of::<ContentMask<ScaledPixels>>(), 4);
        assert_eq!(size_of::<ContentMask<ScaledPixels>>(), 48);
        assert_eq!(offset_of!(ContentMask<ScaledPixels>, bounds), 0);
        assert_eq!(offset_of!(ContentMask<ScaledPixels>, rounded_bounds), 16);
        assert_eq!(offset_of!(ContentMask<ScaledPixels>, corner_radii), 32);
    }

    #[test]
    fn backdrop_blur_gpu_layout_matches_shader_storage_contract() {
        assert_eq!(align_of::<BackdropBlur>(), 4);
        assert_eq!(size_of::<BackdropBlur>(), 112);
        assert_eq!(offset_of!(BackdropBlur, order), 0);
        assert_eq!(offset_of!(BackdropBlur, bounds), 8);
        assert_eq!(offset_of!(BackdropBlur, content_mask), 24);
        assert_eq!(offset_of!(BackdropBlur, corner_radii), 72);
        assert_eq!(offset_of!(BackdropBlur, blur_radius), 88);
        assert_eq!(offset_of!(BackdropBlur, source_origin_x), 92);
        assert_eq!(offset_of!(BackdropBlur, source_origin_y), 96);
        assert_eq!(offset_of!(BackdropBlur, source_width), 100);
        assert_eq!(offset_of!(BackdropBlur, source_height), 104);
        assert_eq!(offset_of!(BackdropBlur, pad2), 108);
    }

    #[test]
    fn backdrop_blur_plan_preserves_radius_across_scale_and_large_values() {
        let cases = [
            (0.0, 0),
            (1.0, 1),
            (11.0, 1),
            (12.0, 2),
            (24.0, 2),
            (25.0, 3),
            (50.0, 3),
            (51.0, 4),
            (101.0, 4),
            (102.0, 5),
            (203.0, 5),
            (204.0, 6),
            (240.0, 6),
        ];

        for (radius, expected_passes) in cases {
            let plan = BackdropBlurPlan::for_radius(radius, BackdropBlurPlan::MAX_PASSES);
            assert_eq!(plan.passes, expected_passes, "radius {radius}");
            if radius >= 4.0 {
                assert!(
                    (plan.sigma() - radius / 3.0).abs() < 0.001,
                    "radius {radius}"
                );
            }
        }
    }

    #[test]
    fn backdrop_blur_plan_clamps_positive_radii_to_minimum_support() {
        let plan = BackdropBlurPlan::for_radius(1.0, BackdropBlurPlan::MAX_PASSES);
        assert_eq!(plan.passes, 1);
        assert!((plan.sigma() * 3.0 - 3.674_234_6).abs() < 0.001);
    }

    #[test]
    fn backdrop_blur_plan_uses_available_levels_without_invalid_shader_values() {
        let plan = BackdropBlurPlan::for_radius(240.0, 4);
        assert_eq!(plan.passes, 4);
        assert!(plan.sample_distance.is_finite());
        assert_eq!(plan.sample_distance, 1.5);

        for radius in [f32::NEG_INFINITY, -1.0, 0.0, f32::INFINITY, f32::NAN] {
            assert_eq!(
                BackdropBlurPlan::for_radius(radius, BackdropBlurPlan::MAX_PASSES),
                BackdropBlurPlan::IDENTITY
            );
        }
        assert_eq!(
            BackdropBlurPlan::for_radius(24.0, 0),
            BackdropBlurPlan::IDENTITY
        );
    }

    #[test]
    fn backdrop_blur_plan_accounts_for_bilinear_upsample_variance() {
        assert!((BackdropBlurPlan::upsample_variance(0.0) - 0.75).abs() < 0.0001);
        assert!((BackdropBlurPlan::upsample_variance(1.0) - 6.083_333).abs() < 0.0001);
        assert!((BackdropBlurPlan::upsample_variance(1.5) - 12.75).abs() < 0.0001);
    }

    #[test]
    fn backdrop_blur_padding_tracks_kernel_support() {
        assert_eq!(BackdropBlurPlan::padding(0.0), 0.0);
        assert_eq!(BackdropBlurPlan::padding(1.0), 6.0);
        assert_eq!(BackdropBlurPlan::padding(24.0), 26.0);
        assert_eq!(BackdropBlurPlan::padding(120.0), 122.0);
        assert!(BackdropBlurPlan::padding(1_000.0) < 500.0);
        assert_eq!(BackdropBlurPlan::padding(f32::NAN), 0.0);
    }

    #[test]
    fn padded_bool32_has_initialized_u32_representation() {
        assert_eq!(size_of::<PaddedBool32>(), size_of::<u32>());
        assert_eq!(align_of::<PaddedBool32>(), align_of::<u32>());
        assert_eq!(PaddedBool32::from(false).0, 0);
        assert_eq!(PaddedBool32::from(true).0, 1);
    }

    #[test]
    fn underline_gpu_layout_matches_shader_storage_contract() {
        assert_eq!(align_of::<Underline>(), 4);
        assert_eq!(size_of::<Underline>(), 96);
        assert_eq!(offset_of!(Underline, order), 0);
        assert_eq!(offset_of!(Underline, bounds), 8);
        assert_eq!(offset_of!(Underline, content_mask), 24);
        assert_eq!(offset_of!(Underline, color), 72);
        assert_eq!(offset_of!(Underline, thickness), 88);
        assert_eq!(offset_of!(Underline, wavy), 92);
    }

    #[test]
    fn polychrome_sprite_gpu_layout_matches_shader_storage_contract() {
        assert_eq!(align_of::<PolychromeSprite>(), 4);
        assert_eq!(size_of::<PolychromeSprite>(), 128);
        assert_eq!(offset_of!(PolychromeSprite, order), 0);
        assert_eq!(offset_of!(PolychromeSprite, grayscale), 8);
        assert_eq!(offset_of!(PolychromeSprite, opacity), 12);
        assert_eq!(offset_of!(PolychromeSprite, bounds), 16);
        assert_eq!(offset_of!(PolychromeSprite, content_mask), 32);
        assert_eq!(offset_of!(PolychromeSprite, corner_radii), 80);
        assert_eq!(offset_of!(PolychromeSprite, tile), 96);
    }

    #[test]
    fn backdrop_blurs_sort_and_batch_between_surrounding_primitives() {
        let bounds = test_bounds();
        let mut scene = Scene::default();
        scene.backdrop_blurs = vec![test_backdrop_blur(3), test_backdrop_blur(2)];
        scene.quads = vec![
            Quad {
                order: 4,
                bounds,
                content_mask: ContentMask::new(bounds),
                ..Default::default()
            },
            Quad {
                order: 1,
                bounds,
                content_mask: ContentMask::new(bounds),
                ..Default::default()
            },
        ];

        scene.finish();

        assert_eq!(
            scene
                .backdrop_blurs
                .iter()
                .map(|blur| blur.order)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            scene
                .quads
                .iter()
                .map(|quad| quad.order)
                .collect::<Vec<_>>(),
            vec![1, 4]
        );

        let mut batches = scene.batches();
        assert!(matches!(
            batches.next(),
            Some(PrimitiveBatch::Quads(range)) if range == (0..1)
        ));
        assert!(matches!(
            batches.next(),
            Some(PrimitiveBatch::BackdropBlurs(range)) if range == (0..2)
        ));
        assert!(matches!(
            batches.next(),
            Some(PrimitiveBatch::Quads(range)) if range == (1..2)
        ));
        assert!(batches.next().is_none());
    }

    #[test]
    fn replay_retained_layer_ranges_and_marks_content_clean() {
        let bounds = test_bounds();
        let content_mask = ContentMask::new(bounds);
        let mut prev_scene = Scene::default();

        prev_scene.insert_primitive(Quad {
            bounds,
            content_mask: content_mask.clone(),
            ..Default::default()
        });
        prev_scene.insert_retained_layer(RetainedLayer {
            id: test_global_id(),
            content_revision: 1.into(),
            content_dirty: true,
            bounds,
            content_mask,
            transform: TransformationMatrix::unit(),
            opacity: 0.5,
            paint_range: 0..1,
        });

        let mut next_scene = Scene::default();
        next_scene.replay(0..1, &prev_scene);

        assert_eq!(next_scene.paint_operations.len(), 1);
        assert_eq!(next_scene.retained_layers.len(), 1);
        assert_eq!(next_scene.retained_layers[0].paint_range, 0..1);
        assert!(!next_scene.retained_layers[0].content_dirty);
        assert_eq!(next_scene.retained_layers[0].opacity, 0.5);
    }

    #[test]
    fn replay_offsets_retained_layer_ranges_after_existing_ops() {
        let bounds = test_bounds();
        let content_mask = ContentMask::new(bounds);
        let mut prev_scene = Scene::default();

        prev_scene.insert_primitive(Quad {
            bounds,
            content_mask: content_mask.clone(),
            ..Default::default()
        });
        prev_scene.insert_retained_layer(RetainedLayer {
            id: test_global_id(),
            content_revision: 1.into(),
            content_dirty: true,
            bounds,
            content_mask: content_mask.clone(),
            transform: TransformationMatrix::unit(),
            opacity: 1.0,
            paint_range: 0..1,
        });

        let mut next_scene = Scene::default();
        next_scene.insert_primitive(Quad {
            bounds,
            content_mask,
            ..Default::default()
        });
        next_scene.replay(0..1, &prev_scene);

        assert_eq!(next_scene.paint_operations.len(), 2);
        assert_eq!(next_scene.retained_layers[0].paint_range, 1..2);
    }

    #[test]
    fn shadow_bounds_treats_any_nonzero_inset_as_inset() {
        let bounds = test_bounds();
        let element_bounds = Bounds::new(
            point(ScaledPixels(1.0), ScaledPixels(1.0)),
            size(ScaledPixels(2.0), ScaledPixels(2.0)),
        );
        let shadow = Primitive::Shadow(Shadow {
            order: 0,
            blur_radius: ScaledPixels(0.0),
            bounds,
            corner_radii: Corners::all(ScaledPixels(0.0)),
            content_mask: ContentMask::new(bounds),
            color: Hsla::default(),
            element_bounds,
            element_corner_radii: Corners::all(ScaledPixels(0.0)),
            inset: 2,
            pad: 0,
        });

        assert_eq!(shadow.bounds(), &element_bounds);
    }
}
